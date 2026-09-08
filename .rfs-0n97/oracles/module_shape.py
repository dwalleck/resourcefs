#!/usr/bin/env python3
"""C02 staged successor to the nae2/h212 placement census (not a Rust parser).

No feature-presence inference: the checked-in ledger chooses the default stage.
--stage overrides that choice for an explicit preparatory checkpoint. --root is
an independent disposable source tree; --repository supplies pinned Git objects.
"""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
CORE = "crates/resourcefs-core/src/"
SOURCES = "crates/resourcefs-sources/src/"
MCP = "crates/resourcefs-mcp/src/"
SERVER = MCP + "server.rs"
GITHUB = SOURCES + "github/mod.rs"


def load_inherited(repository):
    spec = importlib.util.spec_from_file_location(
        "nae2_shape", repository / ".rfs-nae2/oracles/module_shape.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def node_spans(source, inherited):
    """Named declaration spans including attributes; preserve overload ordinals."""
    code = inherited.mask(source)
    counts = Counter()
    result = {}
    pattern = re.compile(r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe)\s+)?(fn|struct|enum|const|type)\s+(\w+)\b")
    for match in pattern.finditer(code):
        kind, name = match.groups()
        opening = code.find("{", match.end())
        semi = code.find(";", match.end())
        if opening < 0 or 0 <= semi < opening:
            end = semi + 1
        else:
            end = inherited.closing(code, opening) + 1
            if code[end:end + 1] == ";":
                end += 1
        if end <= match.start():
            raise ValueError(f"unterminated {kind} {name}")
        start = match.start()
        # Attributes may span lines. Match the immediately preceding balanced
        # bracket, never a comment or a preceding unrelated declaration.
        cursor = start
        while True:
            cursor = len(code[:cursor].rstrip())
            if not cursor or code[cursor - 1] != "]":
                break
            depth, left = 1, cursor - 2
            while left >= 0 and depth:
                depth += (code[left] == "]") - (code[left] == "[")
                left -= 1
            if left < 0 or code[left] != "#":
                break
            start = left
            cursor = left
        counts[name] += 1
        result[(name, counts[name])] = (start, end)
    return result


def erase(source, spans):
    for start, end in sorted(spans, reverse=True):
        source = source[:start] + " " * (end - start) + source[end:]
    return source


def fingerprint(source, inherited):
    """Production code tokens AND literal spellings, excluding comments/tests."""
    code = inherited.production(source)
    # The inherited mask blanks comments and literals. Recover literals only
    # at their lexical positions, using a lexer rather than comment regexes.
    literals = []
    excluded = []
    masked = inherited.mask(source)
    for match in re.finditer(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+\w+\s*\{", masked):
        excluded.append((match.start(), inherited.closing(masked, match.end() - 1) + 1))
    lex = re.compile(r'//[^\n]*|/\*|(?:br|cr|r)(?P<hashes>\#*)"|"|\'(?:\\.|[^\'\\\n])\'')
    pos = 0
    while match := lex.search(source, pos):
        token = match.group()
        start, end = match.start(), match.end()
        if token.startswith('//'):
            pos = end
            continue
        if token == '/*':
            depth = 1
            while end < len(source) and depth:
                if source.startswith('/*', end):
                    depth += 1
                    end += 2
                elif source.startswith('*/', end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            pos = end
            continue
        if match.group('hashes') is not None:
            delimiter = '"' + match.group('hashes')
            stop = source.find(delimiter, end)
            if stop < 0:
                raise ValueError('unterminated raw literal')
            end = stop + len(delimiter)
        elif token == '"':
            while end < len(source):
                if source[end] == '\\':
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
        # Inline test modules were blanked by production(); do not recover them.
        if not any(left <= start < right for left, right in excluded):
            literals.append((start, source[start:end]))
        pos = end
    tokens = [(m.start(), m.group()) for m in inherited.TOKEN.finditer(code)]
    return tuple(value for _, value in sorted(tokens + literals))


class Transition:
    """Explicit policy consumed by the inherited checker; no failure filtering."""
    def __init__(self, ledger, stage, inherited):
        self.ledger, self.stage, self.inherited = ledger, stage, inherited
        self.active = ledger['stages'][:ledger['stages'].index(stage) + 1]
        self.admitted_paths = {path for path, row in ledger['owners'].items()
                               if row['stage'] in self.active}
        self.changed_bodies = {}
        self.added_functions = {}
        self.test_children = {path: row for path, row in ledger.get('test_children', {}).items()
                              if row['stage'] in self.active}
        for name in self.active:
            for path, symbols in ledger['body_changes'].get(name, {}).items():
                self.changed_bodies.setdefault(path, set()).update(symbols)
            for path, symbols in ledger.get('added_functions', {}).get(name, {}).items():
                self.added_functions.setdefault(path, set()).update(symbols)

    def server_projection(self, source):
        inherited = self.inherited
        if 'mcp' in self.active:
            names = set(self.ledger['server_moves']) | {'read'}
            spans = [span for (name, _), span in node_spans(source, inherited).items()
                     if name in names]
            source = erase(source, spans)
            source = re.sub(r'\bimpl\s+ReadLimitsInput\s*\{\s*\}', '', source)
            source = re.sub(r'(?m)^\s*mod\s+read\s*;', '', source)
            source = re.sub(r'(?m)^\s*use\s+(?:self::)?read::(?:ReadInput|\{\s*ReadInput\s*,?\s*\})\s*;', '', source)
            # Only these existing core imports can disappear with read extraction.
            def imports(match):
                tokens = inherited.TOKEN.findall(match.group())
                return ' '.join(t for t in tokens if t not in self.ledger['server_imports'] and t != ',')
            source = re.sub(r'\buse\s+resourcefs_core\s*::\s*\{[^;]*;', imports, source)
        elif 'core' in self.active:
            spans = node_spans(source, inherited)
            span = spans.get(('execute_read', 1))
            if span:
                start, end = span
                body = source[start:end]
                body = re.sub(r'(ReadRequest\s*\{[^{}]*?)\bacquisition\s*:\s*None\s*,', r'\1', body)
                source = source[:start] + body + source[end:]
        source = re.sub(r'#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+jira_query_tests\s*;', '', source)
        return source

    def server_tokens(self, source):
        return self.inherited.TOKEN.findall(self.server_projection(source))


def check(root, repository, ledger, stage):
    inherited = load_inherited(repository)
    policy = Transition(ledger, stage, inherited)
    failures, observations = inherited.check(root, repository, 'query', policy)
    failures = ['C02 inherited ' + failure for failure in failures]

    def fail(path, message):
        failures.append(f'C02 FAIL {path}: {message}')

    baseline = ledger['baseline']
    inherited.git(repository, 'cat-file', '-e', baseline + '^{commit}')
    observations.append(f'C02 stage={stage} baseline={baseline} upstream={inherited.upstream(repository)}')
    sources = {p.relative_to(root).as_posix(): p.read_text(encoding='utf-8')
               for directory in (root / 'crates').glob('*/src') for p in directory.rglob('*.rs')}
    codes = {path: inherited.production(source) for path, source in sources.items()}
    # Inherited checks validated these exact mounts and their cfg(test)
    # ancestry before exclusion from the production ownership census.
    for path in policy.test_children:
        if path in codes:
            codes[path] = ''
    declarations = {path: inherited.DECL.findall(code) for path, code in codes.items()}
    if 'transport' in policy.active:
        path = SOURCES + 'http/mod.rs'
        before = inherited.git(repository, 'show', f'{baseline}:{path}')
        before_bodies = inherited.functions(before)
        after_bodies = inherited.functions(sources.get(path, ''))
        # These are error-decoration changes, not general body exemptions.
        reasons = {
            ('new', 1): 'LimitExceeded',
            ('header_within_ceiling', 1): 'LimitExceeded',
            ('cancelled_mid_request', 1): 'Cancelled',
            ('policy_error_or', 1): 'DeadlineExceeded|TransportFailure',
        }
        for key, allowed_reasons in reasons.items():
            if key not in after_bodies:
                fail(path, f'required error boundary {key[0]}#{key[1]} is missing')
                continue
            _, raw = after_bodies[key]
            projection = re.sub(
                r'\.\s*with_details\s*\(\s*ResourceErrorDetails\s*::\s*new\s*\(\s*'
                r'ErrorReason\s*::\s*(?:' + allowed_reasons + r')\s*\)\s*\)',
                '', raw)
            if fingerprint(projection, inherited) != fingerprint(before_bodies[key][1], inherited):
                fail(path, f'{key[0]}#{key[1]} permits only its typed error-detail decoration')
        for name in ('last_modified', 'date', 'selected_api_version', 'rate_limit_reset'):
            expected = 'self.' + name + ('' if name == 'rate_limit_reset' else '.as_deref()')
            found = after_bodies.get((name, 1))
            if found is None or fingerprint(found[1], inherited) != fingerprint(expected, inherited):
                fail(path, f'{name} must remain a pure bounded-response field getter')
        retained = after_bodies.get(('retained_observation', 1))
        if retained is None:
            fail(path, 'required retained_observation metadata helper is missing')
        else:
            allowed_calls = {'header_within_ceiling', 'Some', 'Ok', 'Err', 'to_str',
                             'is_empty', 'bytes', 'all', 'contains', 'parse_http_date',
                             'is_err', 'to_owned'}
            foreign_calls = set(inherited.CALL.findall(retained[0])) - allowed_calls
            if foreign_calls or re.search(r'\b(?:for|while|loop|unsafe|async|await)\b', retained[0]):
                fail(path, f'retained_observation acquired non-metadata work: {sorted(foreign_calls)}')
            if len(inherited.TOKEN.findall(retained[0])) > 250:
                fail(path, 'retained_observation exceeds its bounded metadata-helper shape')

    for path, row in ledger['owners'].items():
        if row['stage'] not in policy.active:
            if path in sources:
                fail(path, f"owner is premature; requires explicit {row['stage']} stage")
            continue
        if path not in sources:
            fail(path, 'required staged owner is missing (no placeholder permitted)')
            continue
        code = codes[path]
        if len(sources[path].splitlines()) > row['maximum']:
            fail(path, f"growth tripwire exceeds {row['maximum']} physical lines")
        bodies = inherited.functions(code)
        if not bodies or not any(body.strip() for body, _ in bodies.values()):
            fail(path, 'required owner has no implementation body')
        if re.search(r'\b(?:todo|unimplemented)\s*!', code):
            fail(path, 'placeholder implementation is forbidden')
        if re.search(r'\btrait\b', code):
            fail(path, 'unapproved generic seam; retain concrete owner')
        if path.startswith(SOURCES) and re.search(r'\bpub\s+(?:(?:async|const)\s+)?(?:fn|struct|enum|mod)\b', code):
            fail(path, 'new GitHub owner must remain private')
        mount = rf'(?m)^(?:pub\s*\(\s*(?:crate|super)\s*\)\s+)?mod\s+{row["module"]}\s*;'
        if not re.search(mount, codes.get(row['parent'], '')):
            fail(row['parent'], f"missing private declaration for {path}")
        for symbol in row['symbols']:
            if symbol not in declarations[path]:
                fail(path, f'required owned symbol {symbol} is missing')
            # Owner uniqueness is scoped to the concrete subsystem, never a
            # wildcard exception for all GitHub or MCP production files.
            namespace = str(Path(path).parent) + '/'
            for other, symbols in declarations.items():
                if other != path and other.startswith(namespace) and symbol in symbols:
                    fail(other, f'symbol {symbol} belongs in {path}')

    if 'facts' in policy.active:
        identity_path = SOURCES + 'github/facts/identity.rs'
        facts_path = SOURCES + 'github/facts.rs'
        for symbol in ledger['owners'][identity_path]['symbols']:
            if symbol in declarations.get(facts_path, []):
                fail(facts_path, f'identity responsibility {symbol} belongs in {identity_path}')
        entry_points = re.findall(
            r'\bpub\s*\(\s*super\s*\)\s+(?:async\s+)?fn\s+(\w+)',
            codes.get(identity_path, ''))
        if entry_points != ['validate']:
            fail(identity_path, 'identity validation must expose exactly one function: validate')

    # ErrorCategory already has a stable Serialize contract. Preserve it without
    # admitting serialization responsibilities into the new operational details.
    error_path = CORE + 'error.rs'
    before_error = inherited.git(repository, 'show', f'{baseline}:{error_path}')
    serialization = r'\b(?:serde|Serialize)\b'
    added_serialization = (Counter(re.findall(serialization, codes[error_path]))
                           - Counter(re.findall(serialization, inherited.production(before_error))))
    if added_serialization:
        fail(error_path, 'new serialization ownership beyond existing ErrorCategory')

    for path, forbidden in ledger['forbidden_dependencies'].items():
        match = re.search(r'\b(?:' + forbidden + r')\b', codes.get(path, ''))
        if match:
            fail(path, f'forbidden dependency/responsibility {match.group()}')
    for path, code in codes.items():
        if path.startswith(SOURCES + 'http/'):
            match = re.search(r'\b(?:Github\w*|PullRequest\w*|schemaVersion|unavailableFacts)\b', code)
            if match:
                fail(path, f'provider responsibility {match.group()} reached HTTP')

    for path, maximum in ledger['protected_limits'].items():
        if path not in sources:
            fail(path, 'protected parent is missing')
            continue
        count = len(sources[path].splitlines())
        # Only preparatory signature/request wiring may grow before extraction.
        preparation = 0
        if 'core' in policy.active:
            if path == GITHUB and 'fetch' not in policy.active:
                preparation = 30
            elif path == SERVER and 'mcp' not in policy.active:
                preparation = 10
        ceiling = maximum + preparation
        if count > ceiling:
            fail(path, f'parent physical growth: {count} > {ceiling}')
        shrink_stage = 'fetch' if path == GITHUB else 'mcp'
        if shrink_stage in policy.active and count >= maximum:
            fail(path, f'parent must net shrink by {shrink_stage}: {count} >= {maximum}')

    before_server = inherited.git(repository, 'show', f'{baseline}:{SERVER}')
    if SERVER in sources:
        before = policy.server_projection(before_server)
        after = policy.server_projection(sources[SERVER])
        if fingerprint(before, inherited) != fingerprint(after, inherited):
            old_nodes = node_spans(before, inherited)
            new_nodes = node_spans(after, inherited)
            changed = []
            for key in sorted(set(old_nodes) | set(new_nodes)):
                old = before[slice(*old_nodes[key])] if key in old_nodes else ''
                new = after[slice(*new_nodes[key])] if key in new_nodes else ''
                if fingerprint(old, inherited) != fingerprint(new, inherited):
                    changed.append(f'{key[0]}#{key[1]}')
            fail(SERVER, 'unrelated frozen production token/literal delta: ' + (', '.join(changed) or 'imports/declarations/attributes'))
        if 'mcp' in policy.active:
            for symbol in ledger['server_moves']:
                if symbol in declarations[SERVER]:
                    fail(SERVER, f'extracted symbol {symbol} belongs in server/read.rs')
            functions = inherited.functions(sources[SERVER])
            read = functions.get(('read', 1))
            if read is None or ('read', 2) in functions:
                fail(SERVER, 'exactly one read registration delegate is required')
            elif not re.search(r'\.\s*await\b', read[0]):
                fail(SERVER, 'read registration must await its concrete read delegate')
            if read and (inherited.CONTROL.search(read[0]) or len(inherited.TOKEN.findall(read[0])) > 100):
                fail(SERVER, 'read registration must delegate, not own processing/control flow')

    before_github = inherited.git(repository, 'show', f'{baseline}:{GITHUB}')
    if GITHUB in sources:
        before_nodes = node_spans(before_github, inherited)
        after_nodes = node_spans(sources[GITHUB], inherited)
        changes = {symbol for name in policy.active for symbol in ledger['github_changes'].get(name, [])}
        moves = set(ledger['github_moves']) if 'fetch' in policy.active else set()
        move_slots = ledger.get('github_move_slots', {}) if 'fetch' in policy.active else {}
        for key in sorted(set(before_nodes) | set(after_nodes)):
            symbol = key[0]
            if key not in before_nodes:
                fail(GITHUB, f'new parent responsibility declaration {symbol}#{key[1]}')
            elif key not in after_nodes:
                if symbol not in moves and f'{symbol}#{key[1]}' not in move_slots:
                    fail(GITHUB, f'unapproved removed declaration {symbol}#{key[1]}')
            elif symbol not in changes and f'{symbol}#{key[1]}' not in changes:
                old = before_github[slice(*before_nodes[key])]
                new = sources[GITHUB][slice(*after_nodes[key])]
                if fingerprint(old, inherited) != fingerprint(new, inherited):
                    fail(GITHUB, f'untouched body/declaration changed: {symbol}#{key[1]}')
        if 'fetch' in policy.active:
            for symbol in ledger['owners'][SOURCES + 'github/fetch.rs']['symbols']:
                if symbol in declarations[GITHUB]:
                    fail(GITHUB, f'extracted fetch symbol {symbol} retained in parent')
            fetch_path = SOURCES + 'github/fetch.rs'
            fetch_slots = {f'{name}#{ordinal}' for name, ordinal in
                           node_spans(sources.get(fetch_path, ''), inherited)}
            parent_slots = {f'{name}#{ordinal}' for name, ordinal in after_nodes}
            for source_slot, target_slot in move_slots.items():
                if source_slot in parent_slots:
                    fail(GITHUB, f'extracted slot {source_slot} belongs in {fetch_path}')
                if target_slot not in fetch_slots:
                    fail(fetch_path, f'required extracted slot {target_slot} from {source_slot} is missing')
    return failures, observations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', choices=('baseline', 'core', 'configuration', 'transport', 'fetch', 'mcp', 'facts'))
    parser.add_argument('--root', type=Path, help='disposable source tree')
    parser.add_argument('--repository', type=Path, default=HERE.parents[1], help='Git baseline provider')
    args = parser.parse_args()
    try:
        repository = args.repository.resolve(strict=True)
        inherited = load_inherited(repository)
        repository = Path(inherited.git(repository, 'rev-parse', '--show-toplevel'))
        root = args.root.resolve(strict=True) if args.root else repository
        # Policy is trusted checker input, not mutable fixture/source content.
        ledger = json.loads((HERE / 'module-ledger.json').read_text(encoding='utf-8'))
        stage = args.stage or ledger['stage']
        if stage not in ledger['stages']:
            raise ValueError(f'invalid explicit policy stage {stage!r}')
        failures, observations = check(root, repository, ledger, stage)
    except (OSError, UnicodeError, ValueError, ImportError, KeyError) as error:
        print(f'C02 FAIL oracle input: {error}', file=sys.stderr)
        return 1
    print('\n'.join(observations))
    if failures:
        print('\n'.join(failures), file=sys.stderr)
        return 1
    print(f'C02 PASS stage={stage} baseline={ledger["baseline"]}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
