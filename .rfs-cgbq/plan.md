# Plan: Safe multi-root Workspace References

## Partition arithmetic

- Slice 1 diff estimate: 1,700 changed lines.
- Slice 2 diff estimate: 2,600 changed lines.
- Slice 3 diff estimate: 2,100 changed lines.
- Estimated sum: 6,400 changed lines.
- Churn margin: 1,600 lines (25%). The parser corpus, clean-cutover caller migration, platform conditionals, race fences, and raw JSON-RPC harness are the most likely sources of upward drift.
- Projected total: 8,000 changed lines.
- Review-size result: greater than 4,000; partition into three independently mergeable PR increments in dependency order.

### PR increment: `core-reference-grammar`

Contains Slice 1. Mergeable when the workspace remains at three members, core has no ambient-I/O or protocol dependency, the parser corpus and timing fence pass, and the standalone fuzzer completes without the source or MCP changes.

### PR increment: `local-multiroot-authority`

Contains Slice 2 and depends on `core-reference-grammar`. Mergeable when launch-root parsing, capability-contained direct reads, exact primary selection, overlap rejection, backing-path policy, and all source adapter contracts pass without client Roots lifecycle support.

### PR increment: `live-client-root-authority`

Contains Slice 3 and depends on `local-multiroot-authority`. Mergeable when client Roots replacement/fallback, generation invalidation, request-associated bounded acquisition, both MCP revisions, unchanged schema behavior, and architecture checks all pass.

## Slice 1: Establish the pure Path Reference grammar and source-neutral root values

**Claim IDs:** C14, C15

**Expected behavior:** Core parses and canonically renders accepted POSIX, Windows, `file://`, `rfs://workspace`, percent-encoded, and selector-candidate spellings without filesystem access; rejects malformed/traversing/over-limit forms deterministically; preserves the existing 4 KiB debug timing bound; and survives 10,000 invariant-checking fuzzer executions.

**Oracle:** `tests/fixtures/workspace_references.json` declares the expected syntax result independently of parser structs. Timing uses the pre-existing `Instant` fence. The fuzz target independently checks no panic, deterministic result, canonical reparse equality, no decoded traversal, and no encoded-separator bypass.

**Stress fixture:** The shared corpus includes empty input, exactly 64 KiB and 64 KiB plus one byte, malformed percent escapes, `%2F`/`%5C`, `%2E%2E`, `%253A`, Unicode and spaces, `notes:2` versus `notes%3A2`, Windows drive/UNC/verbatim/drive-relative/root-relative forms, POSIX `/foo`, local and non-local `file://`, and canonical references. Expected outcomes and stable categories are recorded per row.

**Regression fence:** `crates/resourcefs-core/tests/path_reference_contract.rs::parses_production_shaped_reference_within_budget`; `fuzz/fuzz_targets/path_reference.rs` exercised by `cargo +nightly fuzz run path_reference -- -runs=10000`.

**Named mutation:** C14 — add one full-input scan inside every input-byte iteration before classification. C15 — decode percent escapes twice, accept a decoded separator, or render canonical components without encoding literal `:`.

**Complexity/production scale:** `PathReference::parse` and canonical rendering are O(n) in input bytes with one 64 KiB allocation ceiling; selector range validation is O(n). At the 64 KiB product cap the accepted maximum is 50 ms in an unoptimized test build, while the retained 4 KiB production-shaped fence remains 5 ms. The bounds prevent quadratic rescanning without making CI depend on sub-millisecond noise.

**Wall budget/phase:** Parsing is always-on for each tool reference: at most 5 ms for the existing 4 KiB debug fixture and 50 ms for a 64 KiB boundary fixture. Corpus generation and fuzzing are one-off verification phases; no runtime wall budget applies.

**Files:** `.gitignore`, `Cargo.toml`, `Cargo.lock`, `crates/resourcefs-core/Cargo.toml`, `crates/resourcefs-core/src/reference.rs`, `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/tests/path_reference_contract.rs`, `crates/resourcefs-core/tests/version_tag_contract.rs`, `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`, `crates/resourcefs-mcp/src/cli.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `tests/fixtures/workspace_references.json`, `fuzz/Cargo.toml`, `fuzz/fuzz_targets/path_reference.rs`.

**Estimate:** 5–7 engineering hours.

**Diff estimate:** 1,700 changed lines: about 600 core implementation, 250 clean-cutover caller migration, 500 contract/corpus, 250 fuzz target/package, and 100 manifest/exports.

**PR increment:** `core-reference-grammar`

**Commands and expected results:**

- `cargo test -p resourcefs-core --test path_reference_contract` → every applicable corpus row equals its independently recorded parse/category/canonical result; 64 KiB passes, one byte over returns `limit_exceeded`; the 4 KiB parse remains within 5 ms and the 64 KiB boundary remains within 50 ms.
- `cargo test -p resourcefs-core` → core unit and contract tests remain green with no filesystem or MCP dependency.
- `cargo +nightly fuzz run path_reference -- -runs=10000` from `fuzz/` → exactly 10,000 executions complete with no panic, nondeterminism, failed canonical round-trip, traversal, or encoded-separator bypass.
- Re-run the first command after each C14 mutation and the fuzz command after each C15 mutation → the named fence fails; restore production code and re-run → it passes.

### Slice 1 checkpoint result — PASS (2026-08-18)

- Affected tests: `cargo test -p resourcefs-core` passed 12 tests across four suites; the pre-existing source adapter suite passed 7 tests and compiled-stdio MCP suite passed 4 tests after the clean cutover.
- Independent oracle/stress: all 36 shared corpus rows, the exact 64 KiB boundary, one-byte-over rejection, canonical reparse checks, and exactly 10,000 libFuzzer executions passed.
- Budgets: the 4 KiB parser fence remained below 5 ms and the 64 KiB boundary below 50 ms in the unoptimized contract suite.
- Regression fences: C14 timing and C15 deterministic/containment/round-trip assertions were green in restored production code.
- Mutations: the C14 quadratic scan failed at 33.97 ms; C15 double decoding, encoded-separator acceptance, and unescaped-colon rendering each failed the fuzz fence before any generated input was accepted.
- Restoration: production code was restored after each mutation; the final core, source, MCP, and 10,000-run fuzz commands all passed.
- Diff-size gate: 1,535 changed product/tracker lines including new corpus/fuzz files, below the 1,700-line Slice 1 cap. Generated workflow artifacts and ignored fuzz build/corpus/lock outputs are excluded.

## Slice 2: Resolve launch roots through capability-contained multi-root authority

**Claim IDs:** C3, C4, C5, C6, C8, C10, C11

**Expected behavior:** One or many CLI launch roots are validated atomically and read through retained capability directories. Relative references require exactly one effective primary; canonical references address every root; absolute and local-file spellings map only through current roots; overlapping matches are ambiguous; literal files precede selector candidates; escape attempts never disclose or return outside content; root and reference limits are exact; and backing `file://` metadata appears only under explicit `Visible` construction.

**Oracle:** The shared corpus supplies symbolic launch roots, selector names, primary selectors, topology, files, and exact expected identities/categories. Tests independently compute candidate counts, expected root-relative paths, canonical IDs, and backing URIs with `url::Url::from_file_path`. Outside sentinel content exists only in the test fixture. Native OS-created paths, symlinks, and reparse points supply the platform oracle.

**Stress fixture:** Create 256 distinct roots plus a rejected 257th; parent and nested roots; two equal selector names; unnamed, empty, NUL-containing, Unicode, and case-varied display names; filenames `notes`, `notes:2`, `notes%3A2`, and percent-encoded `?`/`#`; aliases retargeted between inside and outside directories during repeated reads; absolute/file aliases resolving into nested roots; and a 48 KiB UTF-8 file. Expected results are exact canonical identity, `ambiguous_reference`, `invalid_reference`, `permission_denied`, `not_found`, `unsupported_projection`, or bounded content as declared by the corpus.

**Regression fence:** `path_reference_contract::golden_workspace_references`; `filesystem_adapter_contract::relative_reads_require_unique_primary`; `literal_paths_precede_selectors`; `retargeted_links_never_escape`; native `windows_reparse_points_never_escape`; `overlapping_absolute_inputs_are_ambiguous`; `backing_uri_requires_visibility_policy`; `root_count_limit_is_exact`; native `windows_path_forms_follow_contract`; `stdio_mcp_contract::launch_primary_selection_is_exact`; `canonical_identity_stays_private`.

**Named mutation:** C3 — pick the first root, search all roots for relatives, or return `invalid_reference` for no unique primary. C4 — decode twice, split encoded colon, or split before literal existence. C5 — canonicalize then reopen ambiently, use only lexical prefix checks, or map permission denial to not-found. C6 — choose longest prefix, choose primary, or compare only lexical input. C8 — always render backing metadata or use it as canonical identity. C10 — delegate Windows classification to `PathIsRelative`, accept non-local POSIX hosts, or accept URI query/fragment. C11 — make either cap off by one or count Unicode scalars instead of UTF-8 bytes.

**Complexity/production scale:** Root construction is O(r log r + r·p) for at most 256 roots and normalized path length p; one-off launch construction may spend up to 5 seconds on local capability acquisition. Each read performs O(r·p) candidate containment checks only for absolute/file input, then one capability-relative open and an O(1) authority check. At 256 roots, a 64 KiB reference, and 48 KiB content, the accepted maximum is 250 ms on local test fixtures; ordinary one-root 48 KiB reads retain the existing 100 ms bound.

**Wall budget/phase:** Launch-root validation is one-off per process; no recurring wall budget applies and the accepted maximum is 5 seconds for 256 local roots. Resolution/read is always-on: 250 ms at the 256-root/64 KiB stress boundary and 100 ms for the existing one-root 48 KiB fixture.

**Files:** `Cargo.toml`, `Cargo.lock`, `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`, `crates/resourcefs-mcp/src/cli.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `tests/fixtures/workspace_references.json`.

**Estimate:** 8–12 engineering hours.

**Diff estimate:** 2,600 changed lines: about 1,050 source/core implementation, 250 CLI/MCP caller migration, 1,100 direct/stdio tests, and 200 corpus/manifest changes.

**PR increment:** `local-multiroot-authority`

**Commands and expected results:**

- `cargo test -p resourcefs-core --test path_reference_contract golden_workspace_references` → syntax/source-neutral rows match the corpus and platform filtering only follows each row's declared platform.
- `cargo test -p resourcefs-sources --test filesystem_adapter_contract` → direct reads match every resolution row; exact primary, literal precedence, nested ambiguity, visibility, 256-root boundary, stale/unknown canonical root, and containment fences pass; no outside sentinel or backing path appears in denied results.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract launch_primary_selection_is_exact` → one root is implicit primary; zero/one/two selector matches produce the exact relative/canonical behavior and stable categories.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract canonical_identity_stays_private` → canonical `rfs://` identity and complete text remain unchanged while backing `file://` metadata is absent by default.
- On a native Windows runner, `cargo test -p resourcefs-sources --test filesystem_adapter_contract windows_path_forms_follow_contract windows_reparse_points_never_escape` → Windows drive/UNC/verbatim inputs and reparse containment match the corpus without outside content.
- Re-run the owning command after each C3/C4/C5/C6/C8/C10/C11 mutation → at least the named fence for that claim fails; restore production code and re-run → it passes.

### Slice 2 checkpoint result — PASS (2026-08-18)

- Affected contracts: the final all-feature run passed 7 Path Reference tests and 22 direct filesystem adapter tests, including exact primary selection, literal precedence, non-native absolute rejection, nested-root ambiguity, strict client names, symlinked declared-root mapping, capability-contained symlinks, retarget races, backing-path policy, and exact limits.
- Platform evidence: Linux executed the containment fixtures; Windows MSVC and macOS cross-target builds passed. The independent Wine/Win32 probe confirmed drive/UNC classification and final-handle reparse escape visibility. Native Windows runtime acceptance remains intentionally owned by rfs-58r1.
- Restoration: every reviewed normalization/root-mapping defect has a reproducing regression fence recorded in `review-decisions.md`; restored production code passes the full suite and strict Clippy.

## Slice 3: Install live client-root generations through request-associated MCP acquisition

**Claim IDs:** C1, C2, C7, C9, C12, C13

**Expected behavior:** Client Roots capability causes initialization and change notifications to suspend workspace authority and park the newest epoch without sending a request. The next `tools/list` or `tools/call` performs request-associated `roots/list`; a valid non-empty set replaces launch roots, a valid empty set restores them, invalid/error/timeout disables authority, and a valid later refresh recovers. Client IDs are full URI-derived SHA-256 values, remain stable across order/name changes, and cannot alias historical authority. Reads deliver only when their root ID/canonical URI survives the current generation. MCP 2026-07-28 and 2025-11-25, output schemas, complete text equality, invalid-params behavior, unknown methods/tools, limits, and clean EOF remain unchanged.

**Oracle:** The raw stdio driver owns advertised capability, root responses, nested request timing, normalized URI fixtures, expected SHA-256 IDs, expected generation transitions, and reference visibility independently of server code. Direct source tests own gate release order, expected membership, and watchdog time. Existing architecture and protocol tests predate this implementation.

**Stress fixture:** Send two back-to-back equivalent changes, an older slow completion after a newer valid completion, a dropped acquisition, validation held past five seconds, a removed-root read held between capability I/O and delivery, a retained-root read over the same transition, one synthetic ID collision, renamed/reordered roots, invalid mixed sets, empty fallback, and delayed `roots/list` under both protocol revisions. Expected outcomes are stable generation for equivalent refreshes, newest-epoch authority, `source_unavailable` during refresh/disabled state, `invalid_reference` for removed-root completion, successful retained-root completion, and recovery only after a wholly valid later set.

**Regression fence:** `stdio_mcp_contract::client_roots_replace_and_empty_restores_launch_authority`; `filesystem_adapter_contract::client_ids_ignore_order_and_names`; `reference::tests::duplicate_root_id_is_rejected`; `filesystem::tests::seen_id_cannot_alias_another_uri`; `root_refresh_invalidates_removed_inflight_reads`; `root_change_outcome_names_authority`; `latest_refresh_wins`; `overlapping_equivalent_refreshes_preserve_generation`; `dropped_acquisition_deadline_disables_authority`; `late_root_validation_cannot_commit`; `stdio_mcp_contract::removed_generation_result_is_rejected`; `roots_timeout_fails_closed_and_later_refresh_recovers`; existing `filesystem_adapter_contract`; existing `stdio_mcp_contract`; `architecture_contract::workspace_enforces_dependency_direction`.

**Named mutation:** C1 — union client and launch roots or treat empty as failure. C2 — hash display names, truncate the digest, or omit cross-generation collision history. C7 — validate only before I/O, discard the original baseline on refresh re-entry, let an old epoch install last, increment generation for equivalent reordered roots, or omit removed IDs. C9 — request Roots from notification/spawned scope, use generated unbounded `list_roots`, omit/cancel the watchdog, restart the timeout before validation, permit late validation to commit, or retain launch authority on failure. C12 — return partial over-limit content, remove manual argument pre-validation, or use SDK `LATEST`. C13 — add source/protocol dependencies to core or make `fuzz` a workspace member.

**Complexity/production scale:** Refresh validation is O(r log r + r·p) for at most 256 roots; equality/history checks are O(r). One acquisition has a hard 5-second deadline covering protocol wait and validation, enforced by an epoch watchdog. The ordinary no-refresh request path performs O(1) parked-token and authority checks. Per-read I/O remains bounded to `MAX_TEXT_BYTES + 1` and one final O(1) generation lookup.

**Wall budget/phase:** Client-root acquisition is a one-off phase per initialization/change event with a hard 5-second total deadline. The no-pending `tools/list`/`tools/call` control path is always-on and must add no more than 5 ms in the local stdio fixture. `rfs_read` retains the existing 100 ms one-root 48 KiB local-file bound; a request that performs root acquisition may take at most 5 seconds plus the already-expired source error rendering path.

**Files:** `Cargo.toml`, `Cargo.lock`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`, `crates/resourcefs-mcp/Cargo.toml`, `crates/resourcefs-mcp/src/cli.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 8–12 engineering hours.

**Diff estimate:** 2,100 changed lines: about 850 source/MCP implementation, 1,100 direct/compiled-process tests and test gate, and 150 manifest/CLI adjustments.

**PR increment:** `live-client-root-authority`

**Commands and expected results:**

- `python3 .rfs-cgbq/probe_rmcp.py` → the independent probe reports MCP 2026-07-28, notification-scope association rejection, successful request-scoped initial and changed root fetches, and exactly two `roots/list` calls.
- `cargo test -p resourcefs-sources --features test-support --test filesystem_adapter_contract` → refresh suspension, dropped/expired acquisition, late validation, epoch ordering, equivalent-generation reuse, root-change payloads, client-ID stability/collision history, removed-root rejection, and retained-root completion match independently controlled state/time fixtures.
- `cargo test -p resourcefs-mcp --features test-support --test stdio_mcp_contract` → the installed binary exercises initial/change/empty/invalid/timeout Roots, nested request order, removed-generation delivery, both protocol revisions, unchanged complete text/structured equality, malformed input as MCP invalid params, unknown method/tool categories, limits, and clean EOF.
- `cargo test -p resourcefs-mcp --test architecture_contract` → dependency direction remains MCP → sources → core and exactly three product workspace packages exist.
- `cargo metadata --no-deps --format-version 1` → workspace members are exactly `resourcefs-core`, `resourcefs-sources`, and `resourcefs-mcp`; `fuzz` is excluded; core has no `cap-std`, Tokio, rmcp, rustix, windows-sys, or parking_lot dependency.
- Re-run the owning command after each C1/C2/C7/C9/C12/C13 mutation → at least the named fence for that claim fails; restore production code and re-run → it passes.


### Slice 3 checkpoint result — PASS (2026-08-18)

- Affected contracts: the final all-feature run passed 11 compiled-process MCP tests and the 22-test direct adapter suite. Initial/change/empty/invalid/timeout Roots flows, legacy and current protocol revisions, one serialized concurrent acquisition, removed-generation delivery, cancellation, and fail-closed recovery all passed.
- Independent oracle: `probe_rmcp.py` negotiated `2026-07-28`, reproduced notification-scope rejection, returned both request-scoped root sets, and observed exactly two `roots/list` calls; `oracle.py` independently matched that lifecycle.
- Architecture/release: dependency-direction tests, three-package workspace metadata, strict Clippy, Rust 1.88, Linux/Windows/macOS target checks, release installation, and installed CLI smoke all passed.
- Review: every verified F1–F10 finding is fixed and fenced; the refuted F3 dependency-patch proposal is documented with pinned rmcp cancellation/removal evidence.

## Tracker taxonomy

The plan introduces no deferred behavior. Selector execution, profiles, mutation snapshots, MCP Resource mirroring, release-platform execution, and real-Kiro evidence remain outside this change under verified issues rfs-34pz, rfs-r9m6, rfs-73dz, rfs-ww6w, rfs-58r1, and rfs-5os7 respectively. They are not implementation slices here.

## Self-review

- [x] Every design claim C1–C15 is assigned exactly once; each pending falsifier names its owning slice and checkpoint.
- [x] Every slice has all thirteen mandatory fields.
- [x] Every claim's regression fence and named mutation are in its owning slice; no fence-less risk is accepted.
- [x] New loops state asymptotic behavior, production-scale bounds, and explicit accepted costs; always-on phases state wall budgets.
- [x] The 6,400-line estimate plus 1,600-line churn margin is partitioned into three independently mergeable increments.
- [x] Every non-goal is mapped to a verified tracker issue; no uncited intended work appears.
- [x] No slice is declared complete; checkpointed-build owns completion.

## Review-fix slice: Serialize bounded root acquisition and enforce strict normalization

**Finding IDs:** F1–F10 from `review-decisions.md`.

**Expected behavior:** Parking a Roots refresh suspends authority without starting its clock. The first request-associated acquisition stamps one immutable five-second deadline, one source watchdog owns fail-closed expiry, and one MCP mutex serializes protocol fetch plus source validation while notifications retain only the newest parked epoch. rmcp's configured request timeout emits cancellation and unregisters its pending responder. Client display names either satisfy Workspace Root ID grammar or reject the entire root set fail-closed. File URIs validate backslashes using the same separator grammar that `url` normalizes; absolute forms reject raw ambiguous delimiters except the structural Windows verbatim prefix; fully qualified drive/UNC roots remain accepted only on Windows; malformed extended/device/UNC forms and non-native absolute spellings remain rejected. Existing native absolute and `file://` spellings are canonicalized only for containment mapping, while final handle checks remain authoritative. Matching a nested root directory cannot return before all overlapping roots are counted.

**Oracle:** Paused Tokio time independently separates a parked epoch from a started acquisition. The raw stdio harness holds the first server `roots/list` response while sending a second tool call and requires both calls to succeed after one serialized fetch. The checked-in JSON corpus owns exact normalization/error outcomes. Direct source tests own strict client-name rejection, non-native Windows spelling rejection on POSIX, symlinked declared-root spelling, and nested-root directory ambiguity. Pinned rmcp source independently shows timeout cancellation and local responder removal.

**Stress fixture:** Delay a parked refresh by 60 virtual seconds; drop a started acquisition at five seconds; issue two concurrent tool calls around one held root reply; replace the pending epoch during acquisition; submit an invalid client display name; place drive- and UNC-shaped literal filenames beneath the POSIX process working directory; map an existing file through a symlinked declared root; target a nested root directory shared by two configured roots; supply raw and encoded dot segments after file-URI backslashes; submit raw `?`/`#` in POSIX and Windows absolute paths; and parse drive, UNC, verbatim-drive, verbatim-UNC, malformed multiple-prefix, device-namespace, missing-share, drive-relative, and root-relative forms.

**Regression fence:** `filesystem_adapter_contract::parked_root_refresh_does_not_expire_before_acquisition_starts`; `expired_root_refresh_disables_authority_and_cannot_clobber_recovery`; `invalid_client_display_name_fails_closed`; `rejects_non_native_windows_absolute_spelling_before_ambient_mapping`; `configured_root_symlink_accepts_declared_absolute_spelling`; `overlapping_absolute_inputs_are_ambiguous`; `stdio_mcp_contract::concurrent_requests_share_one_serialized_root_acquisition`; `client_root_acquisition_times_out_fail_closed`; and `path_reference_contract::golden_workspace_references`.

**Named mutation:** Start the deadline in `begin_client_root_refresh`; release the acquisition mutex before awaiting the peer; return early rather than looping when a newer token is parked; call generated `list_roots()` under an outer timeout; silently discard an invalid client display name; return immediately on a root-directory match; skip canonical mapping of an existing native absolute spelling; omit the native `Path::is_absolute` guard before ambient canonicalization; validate file URIs with slash-only splitting; accept arbitrary Windows device/multiple-prefix forms; or move delimiter rejection after absolute classification. Each mutation must fail its owning fence.

**Complexity/production scale:** The pending-token path is O(1); one connection runs at most one O(r log r + r·p) root acquisition for at most 256 roots, with one queued latest token and one five-second watchdog per active acquisition. Root matching remains O(r·p) and checks every root before classifying ambiguity. Path validation remains O(n) within 64 KiB and adds no allocation beyond the decoded path; existing absolute mapping performs one platform canonicalization before capability-relative open and final-handle revalidation.

**Wall budget/phase:** Parking is constant-time and has no expiry while waiting for a legal request scope. Once started, protocol wait plus validation share one hard five-second deadline. Concurrent relevant requests wait behind the same acquisition mutex and then run without another root fetch unless a newer token exists.

**Files:** `crates/resourcefs-core/src/reference.rs`, `tests/fixtures/workspace_references.json`, `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `.rfs-cgbq/review-decisions.md`.

**Estimate:** 2–4 engineering hours.

**Diff estimate:** 500 changed lines: about 210 source/MCP state-machine changes, 190 regression tests/corpus rows, and 100 workflow evidence and review-decision lines.

**PR increment:** `review-fixes-root-acquisition-and-path-normalization`, applied before final integration because the owning increments remain uncommitted.

**Commands and expected results:**

- `cargo test -p resourcefs-core --test path_reference_contract golden_workspace_references -- --exact` → all exact root-form, traversal-normalization, and delimiter rows pass.
- `cargo test -p resourcefs-sources --features test-support --test filesystem_adapter_contract parked_root_refresh_does_not_expire_before_acquisition_starts -- --exact` and `expired_root_refresh_disables_authority_and_cannot_clobber_recovery -- --exact` → parking does not expire; a started acquisition does expire and a newer epoch recovers.
- `cargo test -p resourcefs-mcp --features test-support --test stdio_mcp_contract concurrent_requests_share_one_serialized_root_acquisition -- --exact` → two concurrent tool calls wait for one root acquisition and both return the same successful content.
- `cargo test -p resourcefs-mcp --features test-support --test stdio_mcp_contract client_root_acquisition_times_out_fail_closed -- --exact` → timeout emits cancellation, leaves authority unavailable, and later valid notification recovers.
- `cargo test --workspace --all-targets --all-features`, strict Clippy, formatting, native target checks, declared-MS RV check, and exactly 10,000 fuzzer runs → final integration remains green.

### Review-fix checkpoint result — PASS (2026-08-18)

- `cargo test --workspace --all-targets --all-features` passed 51 tests across 9 suites after the final security-review fix; the root timeout fence completed in 5.02 seconds and emitted one cancellation.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --all`, Rust 1.88 checking, Windows MSVC checking, and macOS checking passed.
- The release installation succeeded, the installed `resourcefs --help` command ran, and exactly 10,000 final parser fuzz executions completed without a finding.
