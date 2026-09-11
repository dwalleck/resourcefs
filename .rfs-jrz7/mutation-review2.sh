#!/usr/bin/env bash
# Review round 2 named mutations: routing, typed ceiling, budget honesty,
# placement-gate claim labelling, pinned baseline, ledger policy shape and
# workspace-wide ownership. Each mutant is compiled where applicable, observed
# red, restored byte-exact, and observed green.
#
# Reproduce from the S1 worktree root:  bash .rfs-jrz7/mutation-review2.sh
# Every edited file is backed up under the probe directory and restored by an
# exit trap, so an interrupted run cannot leave a mutant behind.
set -uo pipefail

WT="$(cd "$(dirname "$0")/.." && pwd)"
PROBE="/tmp/jrz7-mutation-review2"
TREE="$PROBE/tree"

backup() {
    mkdir -p "$PROBE/backup/$(dirname "$1")"
    cp "$WT/$1" "$PROBE/backup/$1"
}

restore() {
    [ -d "$PROBE/backup" ] || return 0
    while IFS= read -r -d '' f; do
        cp "$f" "$WT/${f#"$PROBE"/backup/}"
    done < <(find "$PROBE/backup" -type f -print0)
}

cleanup() { restore; }

reset_probe() {
    rm -rf "$PROBE"
    mkdir -p "$PROBE" "$TREE" "$PROBE/backup"
    cp -r "$WT/scripts" "$PROBE/scripts"
    rm -rf "$PROBE/scripts/__pycache__"
    git -C "$WT" archive HEAD crates | tar -x -C "$TREE"
}

gate() { # gate <label> [pristine]
    local label="$1" root="$TREE" scripts="$PROBE/scripts"
    if [ "${2:-}" = "pristine" ]; then root="$WT"; scripts="$WT/scripts"; fi
    local out status
    out="$(python3 -B "$scripts/module_shape.py" --root "$root" --repository "$WT" 2>&1)"
    status=$?
    printf '  [%s] exit=%s\n' "$label" "$status"
    printf '%s\n' "$out" | grep -E 'FAIL' | head -2 | sed 's/^/    /'
    return $status
}

run_core_test() {
    cargo test --locked -q -p resourcefs-core --test github_immutable_reference_contract "$2" -- --exact 2>&1 \
        | grep -E 'test result|panicked' | head -2 | sed "s/^/  [$1] /"
}

run_sources_test() {
    cargo test --locked -q -p resourcefs-sources --test compiled_sources_contract "$2" -- --exact 2>&1 \
        | grep -E 'test result|panicked' | head -2 | sed "s/^/  [$1] /"
}

reset_probe
trap cleanup EXIT

echo "=== routing: the unserved family must not be gated on the GitHub mount (F8) ==="
backup crates/resourcefs-sources/src/compiled.rs
python3 - "$WT/crates/resourcefs-sources/src/compiled.rs" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
old = """            ResourceAddress::Github(_) => Err(github_family_unreadable()),
            ResourceAddress::Issue(_) | ResourceAddress::PullRequest(_) => {"""
assert old in s
p.write_text(s.replace(old, """            ResourceAddress::Github(_)
            | ResourceAddress::Issue(_)
            | ResourceAddress::PullRequest(_) => {""", 1))
PY
echo "Mutation: fold the family back into the GitHub-record read arm"
run_sources_test "mutant" unserved_github_family_is_refused_before_configuration
restore
run_sources_test "restored" unserved_github_family_is_refused_before_configuration

echo
echo "=== typed path ceiling: a path that cannot fit a reference must not exist (F15, F22) ==="
backup crates/resourcefs-core/src/reference/github.rs
python3 - "$WT/crates/resourcefs-core/src/reference/github.rs" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
start = s.index("        // One ceiling for every family")
end = s.index("        Ok(Self {", start)
p.write_text(s[:start] + s[end:])
PY
echo "Mutation: remove the typed reference-ceiling check"
run_core_test "mutant" typed_source_paths_stay_within_the_reference_ceiling
restore
run_core_test "restored" typed_source_paths_stay_within_the_reference_ceiling

echo
echo "=== budget honesty: a debug run must refuse, not pass silently (F21) ==="
cargo test --locked -q -p resourcefs-sources --test github_facts_contract immutable_reference_parse_budget -- --ignored --exact 2>&1 \
    | grep -E 'test result|run this budget' | head -3 | sed 's/^/  [debug] /'
echo "  [debug] expected: FAILED with \"run this budget in release mode\""
cargo test --locked -q --release --config profile.release.debug-assertions=false -p resourcefs-sources \
    --test github_facts_contract immutable_reference_parse_budget -- --ignored --exact --nocapture 2>&1 \
    | grep -E 'test result|immutable_reference_budget' | head -4 | sed 's/^/  [release] /'
echo "  [release] bounds: 5ms parse average and 1us control average; heap bound unchanged"

echo
echo "=== placement gate: claims, baselines, ledger shape, ownership ==="
reset_probe
gate "pristine" && echo "  [pristine] expected GREEN"

python3 - "$TREE/crates/resourcefs-sources/src/http/mod.rs" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
old = 'pub fn last_modified(&self) -> Option<&str> {\n        self.last_modified.as_deref()\n    }'
assert old in s
p.write_text(s.replace(old, 'pub fn last_modified(&self) -> Option<&str> {\n        None\n    }', 1))
PY
echo "Mutation: transport-stage responsibility change (the claim must read C02, not C11)"
gate "transport mutant"
python3 - "$TREE/crates/resourcefs-sources/src/http/mod.rs" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
old = 'pub fn last_modified(&self) -> Option<&str> {\n        None\n    }'
assert old in s
p.write_text(s.replace(old, 'pub fn last_modified(&self) -> Option<&str> {\n        self.last_modified.as_deref()\n    }', 1))
PY
gate "restored"

python3 - "$TREE/crates/resourcefs-sources/src/github/facts.rs" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
p.write_text(s + '\nfn decode_git_tree(bytes: &[u8]) -> usize {\n    bytes.len()\n}\n')
PY
echo "Mutation: new declaration in a protected parent (C11)"
gate "protected parent mutant"
python3 - "$TREE/crates/resourcefs-sources/src/github/facts.rs" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
tail = '\nfn decode_git_tree(bytes: &[u8]) -> usize {\n    bytes.len()\n}\n'
assert s.endswith(tail)
p.write_text(s[: -len(tail)])
PY
gate "restored"

python3 - "$TREE" <<'PY'
import pathlib, re, sys
tree = pathlib.Path(sys.argv[1])
owner = tree / 'crates/resourcefs-core/src/reference/github.rs'
body = re.search(r'(?ms)^pub\(crate\) fn parse_github_reference\(.*?^\}', owner.read_text()).group(0)
other = tree / 'crates/resourcefs-mcp/src/acquisition.rs'
other.write_text(other.read_text() + '\n' + body + '\n')
PY
echo "Mutation: verbatim copy of an owner declaration in another crate (C11 uniqueness)"
gate "duplicate ownership mutant"
python3 - "$TREE" <<'PY'
import pathlib, sys
tree = pathlib.Path(sys.argv[1])
other = tree / 'crates/resourcefs-mcp/src/acquisition.rs'
s = other.read_text()
marker = '\npub(crate) fn parse_github_reference'
assert marker in s
other.write_text(s[: s.index(marker)] + '\n')
PY
gate "restored"

mutate_ledger() {
    python3 - "$PROBE/scripts/module-ledger.json" "$1" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1]); d = json.loads(p.read_text())
row = d['protected_parents']['crates/resourcefs-sources/src/github/facts.rs']
if sys.argv[2] == 'branch':
    row['baseline'] = 'main'
elif sys.argv[2] == 'dangling':
    row['baseline'] = '0' * 40
elif sys.argv[2] == 'stage':
    row['stage'] = 'immutablegrammar'
elif sys.argv[2] == 'wiring':
    d['wiring_calls']['immutable-grammar']['crates/resourcefs-core/src/reference.rs'] = ['Github.*']
p.write_text(json.dumps(d, indent=2) + '\n')
PY
}

reset_probe; mutate_ledger branch
echo "Mutation: protected-parent baseline replaced by a moving branch name"
gate "branch baseline mutant"
reset_probe; mutate_ledger dangling
echo "Mutation: protected-parent baseline names an absent object"
gate "dangling baseline mutant"
reset_probe; mutate_ledger stage
echo "Mutation: protected-parent stage name mistyped (would silently skip the row)"
gate "stage typo mutant"
reset_probe
echo "Mutation: wiring_calls entry is not an identifier"
mutate_ledger wiring
gate "wiring shape mutant"
reset_probe
gate "restored"
