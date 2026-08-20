# Evidence: rfs-vl0u

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | Current Rust and PCRE2 crates can implement automatic fallback with bounded Unicode-aware advanced matching. | Do `regex` 1.13.1 and forced-bundled `pcre2-sys` 0.2.10 respectively reject and accept a PCRE2-only lookbehind, agree on Unicode caseless matching, and can PCRE2 match/depth limits produce their typed exhaustion codes? | PASS — probe, `pcre2test`, and exact-version source oracle agree |
| P2 | `ignore::WalkBuilder` can be used directly without weakening Workspace Root containment. | With link following enabled, does `ignore` 0.4.33 prevent a directory symlink under the walk root from enumerating an outside sentinel? | PASS — learned that the premise is false; probe and oracle agree the walker escapes |
| P3 | The low-level ignore matcher can reproduce the specified contained nested `.gitignore` behavior independently of ambient Git configuration. | Do `ignore::gitignore` 0.4.33 decisions for a generated root plus nested `.gitignore` corpus agree with `git check-ignore --no-index` for every corpus path? | PASS — all observable ignore/whitelist/no-match decisions agree |
| P4 | `globset` implements the exact path-language behavior adopted by the spec. | Does `globset` 0.4.20 match the hand-counted corpus for component `*`, `?`, classes, recursive `**`, `{a,b}`, slash separation, backslash escape, and ASCII case-insensitive mode? | PASS — all 20 hand-counted cases agree |
| P5 | rmcp 3.1.3 exposes request cancellation through `RequestContext.ct` and the server may await its gated operation before returning. | Current `.rfs-34pz/evidence.md` P1 already proves this against the still-pinned rmcp 3.1.3; mid-traversal response time is feature behavior owned by design/checkpoint evidence. | N/A — current applicable evidence already covers the external premise |
| P6 | Exact result limits, ordering, partial diagnostics, and Artifact pagination meet the requested behavior. | These are requirements decided in approved `spec.md`, not claims about an existing system or external dependency. | N/A — specification and later design/checkpoint territory |

## Data

- Source: production-shaped generated data
- Shape: UTF-8 line corpus with Rust/PCRE2-only, Unicode-case, and backtracking patterns; a temporary Workspace-shaped tree with root and nested `.gitignore` files plus an escaping directory link; and 39 slash-normalized glob paths across 20 cases covering every adopted operator.
- Safety: every probe uses a fresh operating-system temporary directory, isolates `CARGO_HOME` and `CARGO_TARGET_DIR` there, forces the vendored PCRE2 build where claimed, and removes the directory after the run. Oracles use only their own temporary trees/files, installed read-only executables, and immutable exact-version crate source; no production code or operator-owned Resource state is read or mutated.

## Probe

- Files: `probe_pcre2.py`, `probe_ignore.py`, `probe_globset.py`
- Mechanism: each standalone script generates a minimal temporary Rust crate pinned to the candidate dependency version, runs only the candidate-library behavior named by its premise, validates dependency pins, and emits normalized JSON.
- Runs: `python .rfs-vl0u/probe_pcre2.py`; `python .rfs-vl0u/probe_ignore.py`; `python .rfs-vl0u/probe_globset.py`

## Oracle

- P1 mechanism: system PCRE2 10.47 through `pcre2test` computes lookbehind, Unicode caseless, match-limit, and depth-limit outcomes independently of the Rust bindings. Exact-version source inspection confirms regex-syntax's typed lookaround rejection and `pcre2-sys` 0.2.10's forced-static PCRE2 10.46 build, limit setters, typed codes, and interpreter enforcement.
- P2 mechanism: Python `realpath` classifies the generated link target against the authorized root without using the `ignore` walker; exact-version source confirms `WalkBuilder::follow_links` passes directly to ambient `walkdir` with loop detection but no root containment.
- P3 mechanism: hermetic `git check-ignore --no-index -v` evaluates the same generated tree using Git rather than the Rust ignore parser; comparison uses ignored/whitelist/no-match semantics, not non-contractual responsible-pattern attribution.
- P4 mechanism: `oracle_globset.py` returns a manually enumerated expected table without any glob, regex, or `fnmatch` machinery; exact-version source independently identifies the required literal-separator, backslash-escape, and per-glob case flags.
- Runs: `python .rfs-vl0u/oracle_pcre2.py`; `python .rfs-vl0u/oracle_ignore.py`; `python .rfs-vl0u/oracle_globset.py`

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | `regex` rejected lookbehind and matched Unicode caseless text; forced-bundled `pcre2-sys` built/linked/ran, matched lookbehind at bytes 3–6, matched `CAFÉ` against `café`, accepted both limit setters, and returned `-47`/`-53`. | `pcre2test` 10.47 matched the same lookbehind and Unicode case, rejected unbounded lookbehind, returned `PCRE2_ERROR_MATCHLIMIT (-47)` and `PCRE2_ERROR_DEPTHLIMIT (-53)`, and succeeded without the lowered limits; source exposes the same bundled APIs/codes. | PASS |
| P2 | `WalkBuilder::follow_links(true)` visited `linked/secret.txt`; canonical comparison showed it was the outside sentinel. | Python `realpath` classified `linked` outside the authorized root; source says followed links are ambient and only loops are detected. | PASS |
| P3 | For 11 paths, low-level root/child matcher stacking classified six ignored, two whitelisted, and three no-match cases. | Hermetic Git classified the same six/two/three paths identically; only responsible-pattern attribution for a parent-excluded descendant differed and is not observable behavior. | PASS |
| P4 | `globset` with explicit `literal_separator(true)`, `backslash_escape(true)`, and per-pattern case mode produced the expected sorted paths for all 20 cases. | The independent hand table produced byte-identical normalized JSON across all 39 candidate paths. | PASS |

## Validated / learned

- P1: learned — automatic fallback is viable, including Unicode caseless matching and typed bounded-work errors, but the high-level `pcre2` crate does not expose match/depth limits. Production must use a small audited `pcre2-sys` wrapper, force static vendoring for version determinism, keep JIT disabled because depth limits are interpreter-only, and preserve native Windows/macOS builds as checkpoint falsifiers rather than claiming them from this Linux runtime probe.
- P2: learned — `ignore::WalkBuilder` cannot be the authority-preserving walker: following links enumerates outside the Workspace Root. Production traversal must stay capability/final-handle based and use `ignore` only for lexical ignore decisions.
- P3: validated prior understanding — one `GitignoreBuilder` per containing directory, stacked parent-before-child, agrees with Git for ignore, negation, nested override, directory-parent exclusion, and no-match cases without ambient config. Candidates must be proven beneath each matcher root before calling APIs that assert containment.
- P4: learned — the adopted glob language is available only with explicit flags: literal separators and backslash escaping must be enabled on every platform, case-insensitivity is ASCII-only, and the default globset configuration would incorrectly let `*` and `?` cross `/`.

## Related issues

- Consulted: rfs-jk0d (accepted Rust-regex-plus-PCRE2, bounded discovery, and platform contract); rfs-34pz (current rmcp cancellation and Artifact recovery evidence); rfs-cgbq (capability/final-handle containment); rfs-87cv (MCP cancellation seam); rfs-5os7 (future Kiro search gate); rfs-ww6w (separate Resource listing pagination); rfs-r9m6 (future profile limit lowering); rfs-60g1, rfs-trdn, rfs-xiwg, rfs-9o5v, rfs-btji, rfs-nb4s, rfs-uo2w, rfs-cdlp, rfs-vl7z, rfs-g2z9, rfs-45ww, and rfs-c85i (future Source Adapters consuming the common discovery contract).
- Filed: none — the mismatches are documented candidate-library behavior handled by this feature's design, not defects or deferred work.
