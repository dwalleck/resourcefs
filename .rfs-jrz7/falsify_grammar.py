#!/usr/bin/env python3
"""C1 design-level grammar witness, not a production parser or acceptance test."""
from urllib.parse import quote

SHA = "0123456789abcdef0123456789abcdef01234567"
PREFIX = f"github://owner/repo/source/{SHA}/"
UNRESERVED = frozenset(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
HEX = frozenset("0123456789abcdefABCDEF")


def segment_decode(encoded):
    data = bytearray()
    pos = 0
    while pos < len(encoded):
        ch = encoded[pos]
        if ch == "%":
            if pos + 2 >= len(encoded) or not set(encoded[pos + 1:pos + 3]) <= HEX:
                raise ValueError("malformed escape")
            data.append(int(encoded[pos + 1:pos + 3], 16))
            pos += 3
        elif ord(ch) in UNRESERVED:
            data.append(ord(ch))
            pos += 1
        else:
            raise ValueError("noncanonical literal byte")
    result = data.decode("utf-8", errors="strict")
    if not result or result in (".", "..") or "/" in result or "\0" in result:
        raise ValueError("invalid path segment")
    return result


def read_path(reference):
    if not reference.startswith(PREFIX) or not reference.endswith("/facts"):
        raise ValueError("wrong resource shape")
    return "/".join(segment_decode(x) for x in reference[len(PREFIX):-len("/facts")].split("/"))


# Independent encoder: standard-library UTF-8 URI encoding. Decoder above uses
# an explicit byte scanner; expected decoded identities are fixed literals.
paths = ["facts", "a/facts", "caf\u00e9/\u65e5\u672c\u8a9e.txt", "a b/%:?#", "%2F",
         "back\\slash", "line\nname", "plain", "a/facts/facts"]
for path in paths:
    encoded = "/".join(quote(x, safe="-._~") for x in path.split("/"))
    ref = PREFIX + encoded + "/facts"
    assert read_path(ref) == path, ("C1", path, ref)
    print(f"C1 roundtrip PASS {path!r} -> {ref}")
invalid = ["", ".", "..", "%2E", "%2e%2E", "%2F", "a%2fb", "%00", "%FF", "%C0%AF", "%", "%2", "%GG", "a//b", "/a", "a/", "a?b", "a#b", "a:b"]
for encoded in invalid:
    try:
        read_path(PREFIX + encoded + "/facts")
    except (ValueError, UnicodeDecodeError):
        print(f"C1 refusal PASS {encoded!r}")
    else:
        raise AssertionError(("C1 refusal", encoded))
# Positive selector controls: encoded :raw stays a filename, structural suffix
# remains distinguishable. The production adapter rejects source selectors;
# output recovery stays artifact-based, not a second source fetch.
assert read_path(PREFIX + "x%3Araw/facts") == "x:raw"
try:
    read_path(PREFIX + "x/facts:raw")
except ValueError:
    print("C1 selector distinction PASS")
else:
    raise AssertionError("C1 selector collision")
print(f"C1 PASS: {len(paths)} roundtrips, {len(invalid)} refusals, selector distinction; no ResourceFS implementation claim")
