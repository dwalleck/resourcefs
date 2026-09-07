#!/usr/bin/env python3
"""C1/C12 placement census. Not a Rust parser or a behavioral proof.

Compare a source tree (--root may be a disposable copy) with the pinned approved
starting tree in --repository. Run inherited policy unchanged. Parent body token
checks ignore comments/formatting, not implementation; query facade edits admit
only existing calls plus the named dispatch seam. Review still owns semantic
meaning, macro expansion, module depth, allocation cost, and behavioral bounds.
"""

import argparse
from collections import Counter
import difflib
import importlib.util
from pathlib import Path
import re
import subprocess
import sys

BASELINE = "fad4cf2c11ec27f4272355c9e741badc9be0920e"
CORE = "crates/resourcefs-core/src/"
SOURCES = "crates/resourcefs-sources/src/"
HTTP = SOURCES + "http/"
JIRA = SOURCES + "atlassian/jira/"
WIRE = SOURCES + "atlassian/wire/"
REQUEST = HTTP + "request.rs"
READ = HTTP + "read.rs"
QUERY = JIRA + "query.rs"
WIRE_QUERY = WIRE + "query.rs"
PARENTS = {
    CORE + "reference.rs": 1989,
    CORE + "discovery.rs": 1367,
    SOURCES + "atlassian/jira.rs": 317,
    SOURCES + "atlassian/wire.rs": 601,
    SOURCES + "atlassian/render.rs": 488,
    HTTP + "mod.rs": 1679,
}
LIMITS = {
    CORE + "reference/jira.rs": 650,
    CORE + "reference/source_page.rs": 160,
    JIRA + "cursor.rs": 250,
    JIRA + "transport.rs": 400,
    JIRA + "browse.rs": 600,
    WIRE + "collections.rs": 600,
    SOURCES + "atlassian/render/collections.rs": 350,
    REQUEST: 280,
    READ: 350,
    QUERY: 400,
    WIRE_QUERY: 200,
}
REQUEST_SYMBOLS = (
    "HttpRequest", "HttpMethod", "RedirectBehavior", "SourceRequestHeader",
    "invalid_source_header", "is_authority_or_framing_header",
    "retry_copy", "MAX_SOURCE_REQUEST_HEADERS",
    "MAX_SOURCE_REQUEST_HEADER_BYTES", "MAX_HTTP_MUTATION_REQUEST_BYTES",
)
READ_SYMBOLS = (
    "HttpReadBudget", "BoundedRead", "MAX_HTTP_READ_ATTEMPTS",
    "fetch_idempotent", "fetch_with_optional_budget", "charge_attempt",
)
DECL = re.compile(r"\b(?:fn|struct|enum|trait|const|type)\s+([A-Za-z_]\w*)")
FN = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
TOKEN = re.compile(r"[A-Za-z_]\w*|\d+|::|=>|->|[^\s]")
CALL = re.compile(r"\b([A-Za-z_]\w*)\s*(?:!\s*)?\(")
CONTROL = re.compile(r"\b(?:for|while|loop|unsafe)\b|\b(?:sort\w*|spawn\w*|sleep|timeout\w*)\s*\(")


def git(repository, *args, optional=False):
    result = subprocess.run(
        ["git", "-C", str(repository), *args], capture_output=True, text=True,
        encoding="utf-8", check=False,
    )
    if result.returncode and not optional:
        raise ValueError(f"git {' '.join(args)} failed (exit {result.returncode})")
    return result.stdout.strip() if result.returncode == 0 else None


def upstream(repository):
    """Discover tracking/default refs, without assuming a remote or branch name."""
    tracking = git(repository, "rev-parse", "--abbrev-ref", "@{upstream}", optional=True)
    if tracking:
        return tracking
    refs = git(repository, "for-each-ref", "--format=%(refname) %(symref)", "refs/remotes")
    defaults = sorted(line.split(" ", 1)[1] for line in refs.splitlines()
                      if line.split(" ", 1)[0].endswith("/HEAD") and " " in line)
    return ",".join(defaults) or "unavailable (pinned baseline remains authoritative)"


def mask(source):
    """Blank Rust comments/literals preserving offsets, newlines and lifetimes."""
    chars = list(source)
    index = 0
    while index < len(source):
        end = index
        if source.startswith("//", index):
            end = source.find("\n", index)
            if end < 0:
                end = len(source)
        elif source.startswith("/*", index):
            end, depth = index + 2, 1
            while end < len(source) and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            if depth:
                raise ValueError("unterminated Rust block comment")
        else:
            raw = re.match(r'(?:br|cr|r)(#*)"', source[index:])
            if raw:
                delimiter = '"' + raw.group(1)
                stop = source.find(delimiter, index + raw.end())
                if stop < 0:
                    raise ValueError("unterminated Rust raw string")
                end = stop + len(delimiter)
            elif source[index] == '"':
                end = index + 1
                while end < len(source):
                    if source[end] == "\\":
                        end += 2
                    elif source[end] == '"':
                        end += 1
                        break
                    else:
                        end += 1
            elif source[index] == "'":
                character = re.match(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'", source[index:])
                if character:
                    end = index + character.end()
        if end > index:
            for position in range(index, end):
                if chars[position] != "\n":
                    chars[position] = " "
            index = end
        else:
            index += 1
    return "".join(chars)


def closing(code, opening):
    depth = 1
    for index in range(opening + 1, len(code)):
        depth += (code[index] == "{") - (code[index] == "}")
        if depth == 0:
            return index
    raise ValueError("unbalanced Rust body")


def production(source):
    code = mask(source)
    # Unit test modules are excluded from dependency checks, but not line limits
    # or the protected-parent function census (new parent tests are forbidden).
    for match in reversed(list(re.finditer(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+\w+\s*\{", code))):
        end = closing(code, match.end() - 1) + 1
        code = code[:match.start()] + " " * (end - match.start()) + code[end:]
    return code


def functions(source):
    code = mask(source)
    counts, result = Counter(), {}
    for match in FN.finditer(code):
        start = code.find("{", match.end())
        semicolon = code.find(";", match.end())
        if start < 0 or 0 <= semicolon < start:
            continue
        end = closing(code, start)
        name = match.group(1)
        counts[name] += 1
        result[(name, counts[name])] = (code[start + 1:end], source[start + 1:end])
    return result


def check(root, repository, stage):
    failures, observations = [], []

    def fail(path, predicate):
        failures.append(f"C12 FAIL {path}: {predicate}")

    inherited_path = root / ".rfs-h212/oracles/module_shape.py"
    spec = importlib.util.spec_from_file_location("h212_module_shape", inherited_path)
    inherited = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(inherited)
    inherited_failures, inherited_observations = inherited.check(root, "issues")
    failures.extend(item.replace("C16 FAIL", "C1 FAIL", 1) for item in inherited_failures)
    observations.extend("C1 " + item for item in inherited_observations)
    git(repository, "cat-file", "-e", BASELINE + "^{commit}")
    observations.append(f"C12 baseline={BASELINE} upstream={upstream(repository)}")

    limits = {path: limit for path, limit in LIMITS.items()
              if stage == "query" or path not in (QUERY, WIRE_QUERY)}
    ledger = set(PARENTS) | set(limits)
    sources = {}
    for crate in sorted((root / "crates").glob("*/src")):
        for path in sorted(crate.rglob("*.rs")):
            sources[path.relative_to(root).as_posix()] = path.read_text(encoding="utf-8")
    codes = {path: production(source) for path, source in sources.items()}
    read_tests = HTTP + "read_tests.rs"
    if read_tests in sources:
        test_mount = re.compile(r'#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*#\s*\[\s*path\s*=\s*"read_tests.rs"\s*\]\s*mod\s+tests\s*;')
        if not test_mount.search(sources.get(READ, "")):
            fail(read_tests, "separate read tests must be cfg(test)-mounted by http/read.rs")
        elif any(re.search(r"\bmod\s+read_tests\b", code) for code in codes.values()):
            fail(read_tests, "read test file must not acquire a production module declaration")
        else:
            ledger.add(read_tests)
            codes[read_tests] = ""
    mcp_parent = "crates/resourcefs-mcp/src/server.rs"
    mcp_tests = "crates/resourcefs-mcp/src/server/jira_query_tests.rs"
    if mcp_tests in sources:
        mount = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+jira_query_tests\s*;")
        parent_code = codes.get(mcp_parent, "")
        matches = list(mount.finditer(parent_code))
        without_mount = mount.sub("", parent_code)
        if len(matches) != 1:
            fail(mcp_tests, "MCP query tests require exactly one cfg(test) child declaration in server.rs")
        elif any(re.search(r"\bmod\s+jira_query_tests\b", without_mount if path == mcp_parent else code)
                 for path, code in codes.items()):
            fail(mcp_tests, "MCP query tests must not acquire a production module declaration")
        else:
            ledger.add(mcp_tests)
            codes[mcp_tests] = ""
        before_server = git(repository, "show", f"{BASELINE}:{mcp_parent}")
        if TOKEN.findall(without_mount) != TOKEN.findall(production(before_server)):
            fail(mcp_parent, "MCP query proof permits only a cfg(test) child declaration, not new production constructors/visibility/profile wiring")
    for path, maximum in sorted({**PARENTS, **limits}.items()):
        if path not in sources:
            fail(path, "required ledger owner is missing")
            continue
        lines = len(sources[path].splitlines())
        observations.append(f"C12 {path}: lines={lines} maximum={maximum}")
        if lines > maximum:
            fail(path, f"growth tripwire: lines={lines} maximum={maximum}; placement review required")

    baseline_paths = set(git(repository, "ls-tree", "-r", "--name-only", BASELINE).splitlines())
    watched = (CORE + "reference/", SOURCES + "atlassian/", HTTP)
    for path in sorted(sources):
        if path not in baseline_paths and path not in ledger:
            fail(path, "new production owner is outside approved ledger")
        elif path.startswith(watched) and path not in ledger:
            before = git(repository, "show", f"{BASELINE}:{path}", optional=True)
            if before is not None and before != sources[path].rstrip():
                fail(path, "changed production owner is outside approved ledger")

    owners = {symbol: REQUEST for symbol in REQUEST_SYMBOLS}
    owners.update({symbol: READ for symbol in READ_SYMBOLS})
    if stage == "query":
        owners["JiraQuery"] = CORE + "reference/jira.rs"
        owners["read_query"] = QUERY
        owners["encode_query_request"] = WIRE_QUERY
        owners["decode_query_rejection"] = WIRE_QUERY
        owners.update({symbol: REQUEST for symbol in (
            "RequestIntent", "post_json_read_only", "MAX_HTTP_READ_ONLY_REQUEST_BYTES",
        )})
    declarations = {path: DECL.findall(code) for path, code in codes.items()}
    for symbol, owner in sorted(owners.items()):
        if owner.startswith(HTTP):
            namespaces = (HTTP,)
        elif owner.startswith(CORE):
            namespaces = (CORE + "reference.rs", CORE + "reference/", SOURCES + "atlassian/")
        else:
            namespaces = (SOURCES + "atlassian/",)
        found = [path for path, names in declarations.items()
                 if path.startswith(namespaces) and symbol in names]
        if owner not in found:
            fail(owner, f"required owned symbol {symbol} is missing")
        for path in found:
            if path != owner:
                fail(path, f"symbol {symbol} belongs in {owner}, not this owner")

    for path, code in sorted(codes.items()):
        if path != HTTP + "mod.rs" and re.search(r"\breqwest\b", code):
            fail(path, "reqwest dependency/client belongs solely in http/mod.rs; use parent header alias")
        if path.startswith(HTTP) and re.search(r"\b(?:Jira\w*|Atlassian\w*|serde_json|jql)\b", code):
            fail(path, "source-specific query/JSON dependency reached source-neutral HTTP")
        if path in (REQUEST, WIRE_QUERY) and re.search(r"\b(?:HttpSubstrate|BoundedRead|HttpReadBudget|tokio|Secret|OriginCredential)\b", code):
            fail(path, "client/retry/credential dependency violates request or wire direction")
        if path == QUERY and re.search(r"\b(?:serde_json|fetch_cached|sort\w*|Client|ClientBuilder|HttpSubstrate)\b", code):
            fail(path, "query assembler acquired JSON/cache/client/fixed-sort responsibility")
        if path == WIRE_QUERY and re.search(r"\b(?:http|HttpRequest|JiraRead)\b", code):
            fail(path, "wire query must not depend on HTTP or Jira transport")
        if path in (QUERY, JIRA + "transport.rs") and re.search(r"\b(?:sleep|timeout|retry_available|remaining_attempts\s*:)\b", code):
            fail(path, "shared retry/deadline state belongs in http/read.rs")
        if path == SOURCES + "atlassian/render/collections.rs" and re.search(r"\b(?:sort\w*|fetch\w*)\s*\(", code):
            fail(path, "renderer acquired sorting/fetch responsibility")
        if path in (REQUEST, QUERY, WIRE_QUERY) and re.search(r"\bpub\s+(?:async\s+)?(?:fn|enum|struct|trait|mod)\b", code):
            # HttpRequest and its existing public methods are the preserved API.
            if path != REQUEST:
                fail(path, "new query owner exposes an unrestricted public interface")

    request = codes.get(REQUEST, "")
    if (stage == "query" or "post_json_read_only" in declarations.get(REQUEST, ())) and not re.search(r"pub\s*\(\s*crate\s*\)\s+fn\s+post_json_read_only\b", request):
        fail(REQUEST, "post_json_read_only must be explicitly crate-private")
    if not re.search(r"\bbody\s*:\s*Option\s*<\s*(?:bytes\s*::\s*)?Bytes\s*>", request):
        fail(REQUEST, "HttpRequest body must use shared immutable Option<Bytes>")
    parent = codes.get(HTTP + "mod.rs", "")
    if not re.search(r"(?m)^mod\s+request\s*;", parent):
        fail(HTTP + "mod.rs", "request module must remain private")
    if not re.search(r"\bpub\s+use\s+request\s*::\s*(?:HttpRequest\b|\{[^}]*\bHttpRequest\b)", parent):
        fail(HTTP + "mod.rs", "existing public HttpRequest re-export is missing")

    if stage == "query":
        for child, facade in ((QUERY, SOURCES + "atlassian/jira.rs"), (WIRE_QUERY, SOURCES + "atlassian/wire.rs")):
            code = codes.get(child, "")
            if not functions(code):
                fail(child, "required query owner has no implementation function body")
            if re.search(r"\btrait\b", code):
                fail(child, "unapproved generic query seam; retain concrete private owner")
            if not re.search(r"(?m)^(?:pub\s*\(\s*(?:super|crate)\s*\)\s+)?mod\s+query\s*;", codes.get(facade, "")):
                fail(facade, f"missing private query child declaration for {child}")

    for path in sorted(PARENTS):
        if path not in sources:
            continue
        before = git(repository, "show", f"{BASELINE}:{path}")
        after = sources[path]
        delta = len(after.splitlines()) - len(before.splitlines())
        observations.append(f"C12 {path}: baseline-delta={delta:+d}")
        if path == HTTP + "mod.rs" and delta >= 0:
            fail(path, f"request extraction must net shrink parent; delta={delta:+d}")
        added_declarations = Counter(DECL.findall(mask(after))) - Counter(DECL.findall(mask(before)))
        for symbol in sorted(added_declarations):
            fail(path, f"new responsibility declaration {symbol}; parent delta={delta:+d}")
        old_functions, new_functions = functions(before), functions(after)
        for key, (body, raw_body) in new_functions.items():
            symbol = key[0]
            if key not in old_functions:
                fail(path, f"new responsibility/test function {symbol}; parent delta={delta:+d}")
                continue
            old_body, old_raw = old_functions[key]
            if TOKEN.findall(body) == TOKEN.findall(old_body) and re.findall(r'"(?:\\.|[^"\\])*"', raw_body) == re.findall(r'"(?:\\.|[^"\\])*"', old_raw):
                continue
            if path in (HTTP + "mod.rs", SOURCES + "atlassian/wire.rs", SOURCES + "atlassian/render.rs"):
                fail(path, f"changed responsibility body {symbol}; only extraction/declarations/visibility permitted; delta={delta:+d}")
                continue
            old_tokens, new_tokens = TOKEN.findall(old_body), TOKEN.findall(body)
            added = []
            for tag, _, _, start, end in difflib.SequenceMatcher(a=old_tokens, b=new_tokens, autojunk=False).get_opcodes():
                if tag in ("insert", "replace"):
                    added.extend(new_tokens[start:end])
            additions = " ".join(added)
            new_calls = set(CALL.findall(body)) - set(CALL.findall(old_body))
            allowed_calls = {"read_query", "Query"} if path == SOURCES + "atlassian/jira.rs" else {"Query"}
            forbidden_calls = new_calls - allowed_calls
            if CONTROL.search(additions) or forbidden_calls:
                fail(path, f"non-wiring body change in {symbol}; new_calls={sorted(forbidden_calls)} added={additions[:160]!r}; delta={delta:+d}")
            else:
                observations.append(f"C12 {path}: wiring body {symbol} changed; semantic placement review still required")
    return failures, observations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", choices=("http", "query"), required=True)
    parser.add_argument("--root", type=Path, help="source tree, including inherited oracle; may be a disposable copy")
    parser.add_argument("--repository", type=Path, default=Path(__file__).resolve().parents[2],
                        help="Git checkout providing pinned baseline (default: oracle checkout)")
    args = parser.parse_args()
    try:
        repository = Path(git(args.repository.resolve(strict=True), "rev-parse", "--show-toplevel"))
        root = args.root.resolve(strict=True) if args.root else repository
        failures, observations = check(root, repository, args.stage)
    except (OSError, UnicodeError, ValueError, ImportError) as error:
        print(f"C12 FAIL oracle input: {error}", file=sys.stderr)
        return 1
    print("\n".join(observations))
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"C1/C12 PASS stage={args.stage} baseline={BASELINE}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
