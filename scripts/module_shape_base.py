#!/usr/bin/env python3
"""C1/C12 placement census. Not a Rust parser or a behavioral proof.

Compare a source tree (--root may be a disposable copy) with the pinned approved
starting tree in --repository. Run retained policy. Parent body token checks
ignore comments/formatting, not implementation; query facade edits admit only
existing calls plus the named dispatch seam. Review still owns semantic meaning,
macro expansion, module depth, allocation cost, and behavioral bounds.
"""

from collections import Counter
import difflib
from pathlib import Path
import re
import subprocess

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
    # rfs-r31i adds four pr:// facts grammar arms to the shared parser;
    # rfs-jrz7 adds the immutable github:// grammar arm and its bounded path
    # type, which raises this tripwire from 2,010 as that increment's plan
    # records. Any later increment raises it here, in the pinned policy, not
    # through a ledger relaxation.
    CORE + "reference.rs": 2080,
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
    "fetch_bounded_attempts", "fetch_with_optional_budget", "charge_attempt",
)
DECL = re.compile(r"\b(?:fn|struct|enum|trait|const|type)\s+([A-Za-z_]\w*)")
FN = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
TOKEN = re.compile(r"[A-Za-z_]\w*|\d+|::|=>|->|[^\s]")
CALL = re.compile(r"\b([A-Za-z_]\w*)\s*(?:!\s*)?\(")
CONTROL = re.compile(r"\b(?:for|while|loop|unsafe)\b|\b(?:sort\w*|spawn\w*|sleep|timeout\w*)\s*\(")
# Historical C1 placement obligations are permanent policy, not executable
# evidence. Keep their original owners, stages, and responsibility checks here
# so the active successor gate can run without importing an archived checkout.
HISTORICAL_PARENTS = {
    CORE + "reference.rs": (2089, 2080),
    CORE + "discovery.rs": (1342, 1367),
    SOURCES + "atlassian/jira.rs": (467, 317),
    SOURCES + "atlassian/wire.rs": (571, 601),
    SOURCES + "atlassian/render.rs": (480, 488),
    HTTP + "mod.rs": (1759, 1679),
}
HISTORICAL_CHILD_LIMITS = {
    CORE + "reference/jira.rs": 650,
    CORE + "reference/source_page.rs": 160,
    SOURCES + "http/read.rs": 350,
    SOURCES + "atlassian/jira/transport.rs": 400,
    SOURCES + "atlassian/jira/browse.rs": 600,
    SOURCES + "atlassian/jira/cursor.rs": 250,
    SOURCES + "atlassian/wire/collections.rs": 600,
    SOURCES + "atlassian/render/collections.rs": 350,
}
HISTORICAL_REQUIRED = {
    "extraction": (
        CORE + "reference/jira.rs",
        SOURCES + "http/read.rs",
        SOURCES + "atlassian/jira/transport.rs",
    ),
    "projects": (
        CORE + "reference/source_page.rs",
        SOURCES + "atlassian/jira/browse.rs",
        SOURCES + "atlassian/wire/collections.rs",
        SOURCES + "atlassian/render/collections.rs",
    ),
    "issues": (SOURCES + "atlassian/jira/cursor.rs",),
}
HISTORICAL_OWNERS = {
    "parse_jira_address": CORE + "reference/jira.rs",
    "encode_jira_segment": CORE + "reference/jira.rs",
    "fetch_bounded_attempts": SOURCES + "http/read.rs",
    "decode_project_page": SOURCES + "atlassian/wire/collections.rs",
    "decode_project": SOURCES + "atlassian/wire/collections.rs",
    "decode_issue_page": SOURCES + "atlassian/wire/collections.rs",
    "render_project": SOURCES + "atlassian/render/collections.rs",
    "render_projects": SOURCES + "atlassian/render/collections.rs",
    "render_issues": SOURCES + "atlassian/render/collections.rs",
    "encode_cursor": SOURCES + "atlassian/jira/cursor.rs",
    "decode_cursor": SOURCES + "atlassian/jira/cursor.rs",
}
HISTORICAL_JIRA_TRANSPORT = {
    "fetch_cached", "fetch_uncached", "cache_namespace", "cache_key",
    "classify_status", "sanitize_fetch_error",
}
HISTORICAL_DECLARATION = re.compile(
    r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_]\w*)\b"
)
HISTORICAL_INFRASTRUCTURE = re.compile(r"\b(?:serde_json|reqwest|rmcp)\b")


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


def check_historical(root, stage, transition=None):
    """Run the retained historical C1 checks from the permanent policy."""
    failures = []
    observations = []

    def fail(path, predicate):
        failures.append(f"C1 FAIL {path}: {predicate}")

    stages = tuple(HISTORICAL_REQUIRED)
    if stage not in stages:
        raise ValueError(f"invalid historical policy stage {stage!r}")
    required = {
        path
        for name in stages[: stages.index(stage) + 1]
        for path in HISTORICAL_REQUIRED[name]
    }
    for relative in sorted(required):
        if not (root / relative).is_file():
            fail(relative, "required published owner is missing")

    for relative, (before, maximum) in HISTORICAL_PARENTS.items():
        path = root / relative
        if not path.is_file():
            fail(relative, "protected parent is missing")
            continue
        lines = len(path.read_text(encoding="utf-8").splitlines())
        observations.append(f"{relative}: {before} -> {lines}, maximum {maximum}")
        if lines > maximum:
            fail(relative, f"{lines} lines exceeds approved {maximum}; placement review required")

    watched = (
        root / CORE / "reference",
        root / SOURCES / "atlassian",
        root / SOURCES / "http",
    )
    historical_owners = dict(HISTORICAL_OWNERS)
    if transition is not None:
        for old, relocation in transition.relocated_symbols.items():
            if old not in historical_owners:
                raise ValueError(f"unknown historical owner symbol {old!r}")
            del historical_owners[old]
            historical_owners[relocation['name']] = relocation['owner']
    relocated_counts = {
        row['name']: 0 for row in (
            transition.relocated_symbols.values() if transition is not None else ()
        )
    }
    paths = {path for directory in watched for path in directory.rglob("*.rs")}
    paths.add(root / CORE / "reference.rs")
    for path in sorted(paths):
        relative = path.relative_to(root).as_posix()
        source = path.read_text(encoding="utf-8")
        if relative not in HISTORICAL_PARENTS:
            maximum = HISTORICAL_CHILD_LIMITS.get(relative, 650)
            lines = len(source.splitlines())
            if lines > maximum:
                fail(relative, f"{lines} lines exceeds child tripwire {maximum}")
        for name in HISTORICAL_DECLARATION.findall(source):
            owner = historical_owners.get(name)
            if name in relocated_counts:
                relocated_counts[name] += 1
            if transition is not None and name in transition.relocated_symbols:
                fail(relative, f"obsolete shared helper {name} must migrate to its approved owner")
            if relative.startswith(SOURCES + "atlassian/jira") and name in HISTORICAL_JIRA_TRANSPORT:
                owner = SOURCES + "atlassian/jira/transport.rs"
            if owner is not None and relative != owner:
                fail(relative, f"{name} belongs in {owner}")
        if relative.startswith(CORE + "reference/"):
            if HISTORICAL_INFRASTRUCTURE.search(source):
                fail(relative, "provider/protocol infrastructure reached core reference grammar")
        if relative != SOURCES + "http/mod.rs" and re.search(r"\breqwest\b", source):
            fail(relative, "HTTP client must remain in the existing substrate module")

    for name, count in relocated_counts.items():
        if count != 1:
            fail(historical_owners[name], f"shared helper {name} requires one owner, found {count}")

    for path in (root / "crates/resourcefs-mcp/src").rglob("*.rs"):
        source = path.read_text(encoding="utf-8")
        if re.search(r"\b(?:with_fixture_browse_limits|FixtureBrowseLimits|with_jira_browse_limits_for_test|BrowseLimits)\b", source):
            fail(path.relative_to(root).as_posix(), "test-only browse limits reached production protocol code")
    return failures, observations


def check(root, repository, stage, transition=None):
    """Run historical assertions, optionally using an explicit successor policy.

    transition supplies admitted_paths, changed_bodies and server_tokens(source).
    The latter projects only approved read nodes; all other server tokens remain
    frozen. The caller must independently enforce those nodes and new owners.
    Omitting the policy retains the historical invocation exactly.
    """
    failures, observations = [], []

    def fail(path, predicate):
        failures.append(f"C12 FAIL {path}: {predicate}")

    # The final successor policy always includes all three historical stages.
    # Run that retained check on every explicit successor checkpoint, as the
    # former active gate did, while transition controls newer owners.
    inherited_failures, inherited_observations = check_historical(root, "issues", transition)
    failures.extend(inherited_failures)
    observations.extend("C1 " + item for item in inherited_observations)
    git(repository, "cat-file", "-e", BASELINE + "^{commit}")
    observations.append(f"C12 baseline={BASELINE} upstream={upstream(repository)}")

    limits = {path: limit for path, limit in LIMITS.items()
              if stage == "query" or path not in (QUERY, WIRE_QUERY)}
    ledger = set(PARENTS) | set(limits)
    if transition is not None:
        ledger.update(transition.admitted_paths)
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
    if transition is not None:
        for path, row in transition.test_children.items():
            if path not in sources:
                fail(path, "required successor test-only child is missing")
                continue
            parent = row["parent"]
            child = row["module"]
            filename = re.escape(Path(path).name)
            mount_text = r'#\s*\[\s*path\s*=\s*"' + filename + r'"\s*\]\s*mod\s+' + re.escape(child) + r'\s*;'
            direct_test = row.get("cfg_test", False)
            if direct_test:
                mount_text = r'#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*' + mount_text
            mount = re.compile(mount_text)
            parent_source = sources.get(parent, "")
            matches = list(mount.finditer(parent_source))
            # Either the exact read::tests ancestor is already validated,
            # or the ledger requires a direct cfg(test) attribute at the mount.
            test_ancestor = parent == read_tests and codes.get(parent) == ""
            if (not direct_test and not test_ancestor) or len(matches) != 1:
                fail(path, f"requires exactly one test-only {child} mount in {parent}")
            for owner, source in sources.items():
                remaining = mount.sub("", source) if owner == parent else source
                if not direct_test and owner.startswith(HTTP) and re.search(
                        r'\bmod\s+' + re.escape(child) + r'\s*;', mask(remaining)):
                    fail(owner, f"test-only child {path} has another module mount")
                if re.search(r'\bmod\s+' + re.escape(Path(path).stem) + r'\s*;', mask(remaining)):
                    fail(owner, f"test-only child {path} has a bare module mount")
                if re.search(r'#\s*\[\s*path\s*=\s*"[^"]*' + filename + r'"', remaining):
                    fail(owner, f"test-only child {path} has another path mount")
            if len(sources[path].splitlines()) > row["maximum"]:
                fail(path, f'test-only child exceeds {row["maximum"]} physical lines')
            ledger.add(path)
            codes[path] = ""
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
        server_tokens = TOKEN.findall if transition is None else transition.server_tokens
        if server_tokens(without_mount) != server_tokens(production(before_server)):
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
        allowed_new = () if transition is None else transition.added_functions.get(path, ())
        for symbol in sorted(added_declarations):
            if symbol not in allowed_new or added_declarations[symbol] != 1:
                fail(path, f"new responsibility declaration {symbol}; parent delta={delta:+d}")
        old_functions, new_functions = functions(before), functions(after)
        for key, (body, raw_body) in new_functions.items():
            symbol = key[0]
            if key not in old_functions:
                if symbol not in allowed_new or key[1] != 1:
                    fail(path, f"new responsibility/test function {symbol}; parent delta={delta:+d}")
                continue
            old_body, old_raw = old_functions[key]
            if TOKEN.findall(body) == TOKEN.findall(old_body) and re.findall(r'"(?:\\.|[^"\\])*"', raw_body) == re.findall(r'"(?:\\.|[^"\\])*"', old_raw):
                continue
            allowed_bodies = () if transition is None else transition.changed_bodies.get(path, ())
            if symbol in allowed_bodies or f"{symbol}#{key[1]}" in allowed_bodies:
                observations.append(f"C12 {path}: successor-owned body delta {symbol}")
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
            if transition is not None:
                allowed_calls.update(transition.wiring_calls.get(path, set()))
            forbidden_calls = new_calls - allowed_calls
            if CONTROL.search(additions) or forbidden_calls:
                fail(path, f"non-wiring body change in {symbol}; new_calls={sorted(forbidden_calls)} added={additions[:160]!r}; delta={delta:+d}")
            else:
                observations.append(f"C12 {path}: wiring body {symbol} changed; semantic placement review still required")
    return failures, observations


def main():
    raise SystemExit(
        "module_shape_base.py is the shared placement policy, not an entry point: "
        "run scripts/module_shape.py, which loads the pinned ledger and passes this "
        "module its transition. The standalone entry was pinned to a superseded "
        "baseline and could not express any later stage's policy."
    )


if __name__ == "__main__":
    main()
