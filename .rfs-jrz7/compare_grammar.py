#!/usr/bin/env python3
"""Compare the public Rust parser with the independent C1 URI oracle.

The temporary Cargo example is removed even on failure; only this repeatable
comparison driver is retained as evidence. No implementation helper is used to
produce expected identities.
"""
import contextlib
import io
from pathlib import Path
import runpy
import subprocess
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
with contextlib.redirect_stdout(io.StringIO()):
    oracle = runpy.run_path(str(ROOT / ".rfs-jrz7/falsify_grammar.py"))
prefix, sha = oracle["PREFIX"], oracle["SHA"]
cases = []
for path in oracle["paths"]:
    encoded = "/".join(quote(segment, safe="-._~") for segment in path.split("/"))
    reference = prefix + encoded + "/facts"
    assert oracle["read_path"](reference) == path
    cases.append((reference, f"OK\t{reference}\tfalse"))
for encoded in oracle["invalid"]:
    cases.append((prefix + encoded + "/facts", "ERR\tinvalid_reference"))
for encoded in ["%61", "caf%c3%a9", "%255c"]:
    decoded = oracle["read_path"](prefix + encoded + "/facts")
    canonical = prefix + quote(decoded, safe="-._~") + "/facts"
    cases.append((prefix + encoded + "/facts", f"OK\t{canonical}\tfalse"))
commit = f"github://owner/repo/commits/{sha}/facts"
cases.append((commit, f"OK\t{commit}\tfalse"))
cases.append((commit.replace("owner/repo", "Owner/Repo"), f"OK\t{commit}\tfalse"))
for invalid in [commit.replace(sha, sha.upper()), commit.replace(sha, "main"),
                commit.replace(sha, sha[:-1]), prefix + "facts"]:
    cases.append((invalid, "ERR\tinvalid_reference"))
for path, selected in [("x%3Araw/facts", False), ("x/facts:raw", True)]:
    reference = prefix + path
    cases.append((reference, f"OK\t{reference}\t{str(selected).lower()}"))

example_dir = ROOT / "crates/resourcefs-core/examples"
example = example_dir / "rfs_jrz7_reference_oracle.rs"
created_dir = not example_dir.exists()
example_dir.mkdir(exist_ok=True)
source = '''use resourcefs_core::PathReference;
fn main() {
    for input in std::env::args().skip(1) {
        match PathReference::parse(&input) {
            Ok(reference) => println!("OK\\t{}\\t{}", reference.requested(), reference.projection().is_some()),
            Err(error) => println!("ERR\\t{}", error.category().as_str()),
        }
    }
}
'''
created_file = False
try:
    with example.open("x") as output:
        created_file = True
        output.write(source)
    result = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "resourcefs-core", "--example",
         "rfs_jrz7_reference_oracle", "--", *(reference for reference, _ in cases)],
        cwd=ROOT, text=True, capture_output=True, check=True,
    )
    actual = result.stdout.splitlines()
    assert len(actual) == len(cases), (len(actual), len(cases), result.stdout)
    for (reference, expected), observed in zip(cases, actual):
        assert observed == expected, (reference, expected, observed)
        print(f"C1 production/oracle PASS {reference!r}: {observed}")
    print(f"C1 production/oracle PASS: {len(cases)} identities/refusals/selector cases")
except subprocess.CalledProcessError as error:
    print(error.stderr)
    raise
finally:
    if created_file:
        example.unlink()
    if created_dir:
        example_dir.rmdir()
