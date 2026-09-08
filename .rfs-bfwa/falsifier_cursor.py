#!/usr/bin/env python3
"""C3 cheapest falsifier driver.

Builds the proposed continuation envelope for the observed provider next link,
checks it with an independent Python canonicality/length oracle, then asks the
real resourcefs-core grammar (via the throwaway /tmp probe) what it accepts.
Positive controls prove `pr://` accepts a selector today, so the missing
`:cursor:` split is an isolated, identified gap rather than a parser quirk.
"""

import base64
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

PROBE = "/tmp/rfs-bfwa-falsifier-target/release/rfs-bfwa-falsifier"
PROBE_MANIFEST = str(Path(__file__).resolve().parent / "falsifier_probe" / "Cargo.toml")
PROBE_TARGET = "/tmp/rfs-bfwa-falsifier-target"
CEILING = 64 * 1024
OWNER = "pr://rust-lang/rust/112049/comments/facts"
REFERENCE_PREFIX = "pr://rust-lang/rust/112049/facts"
ORIGIN = "https://api.github.com"
SESSION = "session-token-placeholder-for-binding"
OBSERVED_NEXT = (
    "https://api.github.com/repositories/724712/issues/112049/comments"
    "?per_page=100&page=2"
)
SYNTHETIC_NEXT = "https://api.github.com/repositories/724712/issues/112049/comments?" + "&".join(
    f"q{i}=0123456789abcdef" for i in range(100)
)

CANONICAL = re.compile(r"^[A-Za-z0-9_-]+$")


def envelope(next_link):
    return {
        "v": 1,
        "r": OWNER,
        "o": hashlib.sha256(ORIGIN.encode()).hexdigest(),
        "s": hashlib.sha256(SESSION.encode()).hexdigest(),
        "n": next_link,
    }


def encode(case_next, padded=False):
    raw = json.dumps(envelope(case_next), separators=(",", ":"), sort_keys=True).encode()
    if padded:
        return base64.urlsafe_b64encode(raw).decode()
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def oracle(encoded):
    """Independent canonicality/length rules read from source_page.rs."""
    no_padding = "=" not in encoded
    alphabet = bool(CANONICAL.match(encoded)) if encoded else False
    decodable = False
    canonical_tail = False
    try:
        raw = base64.urlsafe_b64decode(encoded + "=" * (-len(encoded) % 4))
        decodable = True
        canonical_tail = base64.urlsafe_b64encode(raw).rstrip(b"=").decode() == encoded
    except Exception:
        pass
    total = len(REFERENCE_PREFIX) + len(":cursor:") + len(encoded)
    return {
        "no_padding": no_padding,
        "alphabet": alphabet,
        "decodable": decodable,
        "canonical_tail": canonical_tail,
        "selector_len": total,
        "within_ceiling": total <= CEILING,
        "canonical": no_padding and alphabet and decodable and canonical_tail,
    }


def main():
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--manifest-path",
            PROBE_MANIFEST,
            "--target-dir",
            PROBE_TARGET,
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    # The throwaway probe's resolution lock is build noise, not evidence.
    (Path(PROBE_MANIFEST).parent / "Cargo.lock").unlink(missing_ok=True)
    observed = encode(OBSERVED_NEXT)
    synthetic = encode(SYNTHETIC_NEXT)
    padded = encode(OBSERVED_NEXT, padded=True)
    cases = [
        # name, reference, expect canonical encoding, expect current parse
        ("observed_next", f"{REFERENCE_PREFIX}:cursor:{observed}", True, False),
        ("synthetic_2kib_next", f"{REFERENCE_PREFIX}:cursor:{synthetic}", True, False),
        ("padded_mutation", f"{REFERENCE_PREFIX}:cursor:{padded}", False, False),
        ("positive_control_page", f"{REFERENCE_PREFIX}:page:2", None, True),
        ("positive_control_lines", f"{REFERENCE_PREFIX}:2-4", None, True),
    ]
    payload = "\n".join(reference for _, reference, _, _ in cases) + "\n"
    completed = subprocess.run(
        [PROBE], input=payload, text=True, capture_output=True, check=True
    )
    verdicts = json.loads(completed.stdout)

    rows = []
    for (name, reference, expect_canonical, expect_parse), verdict in zip(cases, verdicts):
        encoded = reference.rsplit(":cursor:", 1)[1] if ":cursor:" in reference else ""
        independent = oracle(encoded) if encoded else None
        if expect_canonical is None:
            # Positive control: a selector on this route must parse today.
            agrees = verdict["reference_parse_ok"] is True and verdict["selector_kind"] != "none"
        elif expect_canonical:
            # Canonical encoding accepted by the selector grammar, fits the
            # ceiling, and currently unparseable as a route (the design gap).
            agrees = (
                independent["canonical"]
                and independent["within_ceiling"]
                and verdict["reference_parse_ok"] is False
            )
        else:
            agrees = independent["canonical"] is False and verdict["reference_parse_ok"] is False
        rows.append(
            {
                "case": name,
                "encoded_len": len(encoded),
                "oracle": independent,
                "grammar": verdict,
                "agrees": agrees,
            }
        )

    result = {"all_pass": all(row["agrees"] for row in rows), "rows": rows}
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["all_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
