# Plan: Operate ResourceFS from one strict Server Profile

## Approved inputs

- Route: Empirical (`route.md`): unverified OS/runtime subprocess behavior, a new public CLI/configuration seam, and production-scale process/output risks.
- Behavior: `spec.md` (approval recorded there).
- Design: `design.md` (approval recorded there, 2026-08-21, no accepted risks).
- Design verification (budgeted-plan step 1): the Falsification table is complete with no empty cells; the cheapest falsifier (C1) is PASS; no row is FAIL; every other row is `PENDING — checkpointed-build` with its per-slice gate assigned below; every oracle, mutation, and fence is mechanically specific. Risk acceptances: none; every claim has a deterministic regression fence.
- Empirical basis: `evidence.md` P1–P6 PASS; independent oracles `oracle_schema.py`, `oracle_environment.py`, `oracle_bounded_pipes.py`, `oracle_posix_tree.py`, and `oracle_windows.ps1` (native Windows via `windows_evidence_runner.py`).

## Integration budget

| Slice | Claims | Diff estimate |
|---|---|---:|
| 1. Strict bounded profile model, null-rejecting decode, schema emission, architecture direction | C1, C4, C29, C32 | 2,300 |
| 2. Independent false-by-default grants and closed globally-unique source catalog | C6, C7 | 750 |
| 3. Network source configuration: HTTPS, GitHub, SSH, downstream MCP | C8, C9, C10, C17 | 1,300 |
| 4. Local source configuration: documents, skills, rules, memory, vault, agent export | C11–C16 | 1,400 |
| 5. Bounded command executor: direct argv, minimal environment, admission, tree cleanup | C19–C22 | 1,750 |
| 6. Opaque secret resolution, exactly-one-ending normalization, redactor | C18 | 850 |
| 7. Complete check: static offline validation and one-attempt probing with deterministic reports | C26, C27 | 1,800 |
| 8. Lower-only ServerLimits aggregate and caller migration | C23 | 650 |
| 9. Session retention configuration under the namespaced cache child | C24 | 450 |
| 10. Serialized redacting stderr/rotating-file logging sink | C25 | 600 |
| 11. Serve launch authority, immutable LaunchPlan, exit/channel matrix, all-sink secret acceptance | C2, C3, C5, C28, C30, C31, C33 | 2,100 |
| **Sum** | | **13,950** |
| **Churn margin** | 25%, rounded up (3,487.5 → 3,488): plans drift upward; covers conversion-arm drift, fixture growth, and cross-platform test duplication observed in scout reconnaissance | **3,488** |
| **Projected total** | | **17,438** |

The projected total exceeds the exact 4,000-line review-size threshold, so the plan partitions into six independently mergeable PR increments in dependency order.

### PR increment A: strict-model-schema-grants

Slices 1–2 (3,050 lines). Mergeable definition: the bounded strict schema-version-1 profile model, deterministic `resourcefs schema` command, grant lattice, and closed globally-unique catalog validation are complete and directly fenced; the only CLI delta is the additive `schema` subcommand; `serve` keeps its existing CLI-roots behavior. Verifies without increments B–F: `profile_contract`, `schema_contract`, `configuration_contract::nested_grants_are_subsets`, `architecture_contract`, and the `server_profile` fuzz target plus `oracle_schema.py` exercise pure decode/validation against committed fixtures with no serving, process, secret, or source-constructor wiring.

### PR increment B: source-configuration

Slices 3–4 (2,700 lines), stacked on A. Mergeable definition: all ten typed source configurations validate through public `resourcefs-sources::configuration` constructors, and the profile conversion arms route every kind through them; no serve/check/probe wiring exists yet. Verifies without increments C–F: `configuration_contract` matrices fence every constructor against independent URL/scheme/canonical-identity/lattice oracles, and `profile_contract` conversion rows prove DTO boundary and constructors agree; inert `SecretReference`/`CommandSpec`/`ChildEnvironment`/`SchemeClaim` value types are shape-validated only.

### PR increment C: command-secret-substrate

Slices 5–6 (2,600 lines), stacked on B. Mergeable definition: the bounded `CommandExecutor` (direct argv, cleared baseline-plus-explicit environment, dual-pipe bounding, tiered role ceilings, 32-tree admission, Unix process-group/Windows Job Object cleanup) and opaque secret resolution/normalization/redaction land entirely inside `resourcefs-sources`/`resourcefs-core`; no CLI surface changes. Verifies without increments D–F: `process_contract`, `secret_contract`, and `secret_opacity_contract` run against the evidence oracles, including the native Windows VM checkpoint.

### PR increment D: complete-check-reporting

Slice 7 (1,800 lines), stacked on C. Mergeable definition: `resourcefs check --config <path> [--probe]` performs exhaustive static offline validation and one-attempt-per-source probing, emitting the deterministic redacted JSON report with the check-path 0/2/3 exit mapping; `serve` is unchanged. Verifies without increments E–F: `cli_contract::static_check_is_offline_and_deterministic` and `probe_contract::probe_state_and_exit_matrix` run the compiled binary under denied-network and counted-fake-probe fixtures.

### PR increment E: limits-retention-logging

Slices 8–10 (1,700 lines), stacked on D. Mergeable definition: the core `ServerLimits` aggregate, validated `SessionStorageConfig`, and the serialized redacting logging sink land with every existing caller migrated at approved default ceilings so observable behavior is preserved; serve-time application wiring is not yet present. Verifies without increment F: `server_limits_contract`, extended `session_storage_contract`, and `logging_contract` fence each substrate standalone.

### PR increment F: serving-compiled-acceptance

Slice 11 (2,100 lines), stacked on E. Mergeable definition: `serve --config` launch authority with exactly-one-authority conflicts, the immutable `LaunchPlan` feeding `LaunchRootSource::Profile`, the complete 0/2/3/1 exit/channel matrix, telemetry and placement-owner architecture fences, profile immutability, and the compiled all-sink secret matrix complete the change. Verifies end-to-end: `cli_contract`, `stdio_mcp_contract`, and `architecture_contract` run the compiled binary; every earlier increment's fences remain green.

### Spec delivery-seam reconciliation

`spec.md` records six delivery increments; the partition follows them with two seams split at verification boundaries, reconciled as follows (no conflict remains):

1. Spec increment 1 (strict model/schema and static validation) → increment A carries the model/schema/validation machinery; increment D carries the `check` command surface that exposes static validation. A verifies its machinery without the CLI surface; D adds only the surface.
2. Spec increment 2 (source configuration and launch composition) → increment B carries source configuration; increment F carries launch composition. B verifies constructors and conversion without serve wiring; F wires serving.
3. Spec increment 3 (shared secret/command/process substrate) → increment C exactly.
4. Spec increment 4 (check/probe/reporting) → increment D exactly: static and probe check land together in one slice/increment because C26's literal named mutation requires `ProbeRunner` to exist.
5. Spec increment 5 (logging/session/limit application) → increment E carries the validated substrates with all callers migrated at defaults; increment F applies them in `server::serve(LaunchPlan)`. E verifies standalone at approved defaults.
6. Spec increment 6 (compiled-binary and cross-platform acceptance) → increment F, with the native Windows process-cleanup checkpoint already exercised in increment C.

Mergeability is defined against the repository's default upstream branch; each increment stacks on the previous in dependency order.

## Slice 1: Strict bounded schema-version-1 profile model, null-rejecting decode, deterministic schema emission

**Claim IDs:** C1, C4, C29, C32

**Expected behavior:** `ProfileDocument::load` reads at most 1,048,576 bytes and accepts only closed schemaVersion-1 objects with the ten tagged source kinds, rejecting malformed/oversized/non-UTF-8 input, explicit null everywhere, unknown fields/kinds, wrong-kind fields, and any 4,097th nested allowlist entry during bounded decoding before authority construction. `resourcefs schema` writes one deterministic pretty draft-2020-12 document (`$id` `https://resourcefs.dev/schema/server-profile-v1.json`, title `ResourceFS Server Profile v1`, descriptions, closed objects, exactly one trailing newline) to stdout with empty stderr and exit 0, byte-identical across runs and agreeing with the deserializer on every corpus row. The three-crate dependency direction holds and `resourcefs-sources` stays free of serde_json/schemars.

**Oracle:** `.rfs-r9m6/oracle_schema.py` (Python jsonschema 4.25.1 Draft202012 engine) independently validates the emitted schema against the hand-authored expected matrix; hand-authored accept/reject row categories for deserialization; the cargo-metadata expected edge table in `architecture_contract.rs`, independent of Rust module implementation.

**Stress fixture:** Committed corpus `crates/resourcefs-mcp/tests/fixtures/profile_corpus.json`: empty/malformed/non-UTF-8/short-read byte streams; an exact 1,048,576-byte valid profile accepted and a 1,048,577-byte profile rejected before authority construction; schemaVersion missing/null/bool/string/fraction/0/2 rows; explicit-null rows at every optional section; unknown fields at every nesting level; all ten kinds each at list cardinalities 0/1/4,096 accepted and 4,097 rejected during decode (C32). Expected outcome per row is written now: the row's accept/reject verdict and bounded field-identifying error category.

**Regression fence:** `crates/resourcefs-mcp/tests/profile_contract.rs::{accepts_valid_profiles,rejects_invalid_profiles,schema_matches_deserializer,nested_allowlist_cardinality_matrix}` and `crates/resourcefs-mcp/tests/schema_contract.rs` including the compiled-binary schema test (both created in THIS slice); `fuzz/fuzz_targets/server_profile.rs` (created in THIS slice); existing `crates/resourcefs-mcp/tests/architecture_contract.rs::enforces_dependency_direction` (C1, PASS per design run log) plus this slice's new sources-serde_json/schemars-freedom assertions.

**Named mutation:** C4 — replace the custom absent-only decoder with derived `Option<T>` in `profile::model`; explicit-null rows must fail. C32 — change `MAX_ALLOWLIST_ENTRIES` to 4,097; every 4,097-entry corpus row must fail. C29 — hand-edit one schema property name in `profile::schema_json`; golden/differential must fail. C1 — add `clap` to `resourcefs-sources/Cargo.toml`; `enforces_dependency_direction` must fail naming the edge. checkpointed-build applies each, confirms red, restores, and confirms green.

**Complexity/production scale:** Decode is O(profile bytes) with the 1,048,576-byte cap bounding all visitor work; per-list counting visitors are O(entries) ≤ 4,096 per list with total entries byte-bounded by the same cap; schema generation is O(schema nodes), fixed for the version-1 model. Production-scale input: one 1 MiB profile with 256 sources and 4,096-entry allowlists. Maximum accepted cost: full decode-plus-validate of the exact-ceiling corpus profile under 5 seconds in debug and 1 second in release; rationale: one bounded linear pass over a 1 MiB input with constant-per-entry visitor work and no allocation beyond the decoded DTO size.

**Wall budget/phase:** N/A — reason: one-off phase; no wall budget (profile loads once per process invocation; schema emits once per command).

**Files:** Create `crates/resourcefs-mcp/src/profile/mod.rs` (`ProfileDocument::load/check`, `CheckedProfile`), `crates/resourcefs-mcp/src/profile/model.rs` (ten-kind DTOs, absent-only decoding, bounded visitors, `MAX_ALLOWLIST_ENTRIES = 4,096`), `crates/resourcefs-mcp/src/profile/schema.rs` (`schema_json`); modify `crates/resourcefs-mcp/src/lib.rs` (`mod profile`), `crates/resourcefs-mcp/src/cli.rs` (additive `Command::Schema` printing `schema_json` plus one newline; `Serve` unchanged); create `crates/resourcefs-mcp/tests/profile_contract.rs`, `crates/resourcefs-mcp/tests/schema_contract.rs`, `crates/resourcefs-mcp/tests/fixtures/profile_corpus.json`; modify `crates/resourcefs-mcp/tests/architecture_contract.rs` (sources serde_json/schemars freedom); create `fuzz/fuzz_targets/server_profile.rs`; modify `fuzz/Cargo.toml` (resourcefs-mcp path dependency plus `[[bin]]`).

**Estimate:** 2 days.

**Diff estimate:** 2,300 changed lines (≈1,100 implementation, ≈900 tests, ≈300 corpus fixtures).

**PR increment:** A — strict-model-schema-grants.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test profile_contract` → every corpus row's accept/reject outcome and error category agrees item-by-item with the hand-authored expected categories; exact-ceiling byte/list rows are accepted, one-over rows are rejected with bounded field-identifying messages; under the `Option<T>` mutation the explicit-null rows flip to accepted (red), restore → green; under `MAX_ALLOWLIST_ENTRIES = 4,097` the 4,097-entry rows flip (red), restore → green.
- `cargo test -p resourcefs-mcp --test schema_contract` → compiled-binary `resourcefs schema` stdout byte-equals the committed golden (stable `$id`/title, descriptions, closed objects, one trailing newline), stderr empty, exit 0, two invocations byte-identical; every corpus row's deserializer outcome equals the emitted schema's validation outcome; the property-name mutation makes golden/differential red, restore → green.
- `uv run ./.rfs-r9m6/oracle_schema.py` → independent draft-2020-12 validation of the emitted schema reproduces the hand-authored expected matrix with zero row disagreements.
- `cargo fuzz run server_profile -- -max_total_time=60` → arbitrary bytes and every corpus row parse without panic and double-parse deterministically.
- `cargo test -p resourcefs-mcp --test architecture_contract` → dependency direction intact and sources free of serde_json/schemars; adding `clap` to `resourcefs-sources/Cargo.toml` fails naming the edge, restore → green.

## Slice 2: Independent false-by-default mutation grants and closed globally-unique source catalog

**Claim IDs:** C6, C7

**Expected behavior:** `configuration::MutationGrants` decodes absent create/update/delete as false with only literal `true` granting an operation; `MutationSupport` bounds each kind's operations (skills/rules/vault full; github create+update; https/ssh/documents/memory/agentExport/downstreamMcp read-only) and generic nested grants must be subsets of both kind support and parent grants. Profile validation accepts exactly one entry per kind with explicit `required` state, enforces the approved 1–128-byte ID grammar bound before constructing core IDs (core `WorkspaceRootId` has no length bound), and rejects duplicate kinds, duplicate IDs, cross-entry scheme-claim collisions, and 257 sources/roots before serving.

**Oracle:** Hand-authored permission lattice independent of the validator (C6); independent sets built by the test from literal IDs and normalized schemes (C7).

**Stress fixture:** Every kind × operation grant row including nested subset/superset pairs; the complete ten-kind source catalog and 256 roots accepted, an eleventh duplicate-kind source and 257 roots rejected; the otherwise-unreachable 257-source defensive ceiling is exercised as a rejection before duplicate-kind validation. Case-sensitive IDs are accepted; null/non-boolean/`writable` grant spellings are rejected. Expected: accept/reject per lattice row, written now. The earlier “256 distinct sources accepted” wording was impossible under the approved exactly-one-entry-per-ten-kinds invariant; this correction preserves the approved spec rather than fabricating extra source kinds.

**Regression fence:** `crates/resourcefs-mcp/tests/profile_contract.rs::{grant_matrix,source_catalog_matrix}` (extended in THIS slice) and `crates/resourcefs-sources/tests/configuration_contract.rs::nested_grants_are_subsets` (created in THIS slice).

**Named mutation:** C6 — permit GitHub delete in `configuration::MutationSupport`; the named github-delete row must fail. C7 — remove seen-kind insertion in `profile::validate_sources`; the duplicate-kind row must fail. Both restored → green.

**Complexity/production scale:** Catalog validation is O(sources + roots + claims) with defensive 256/256 caps, an effective ten-source maximum under version 1's unique-kind catalog, and ≤4,096 entries per nested list (all byte-capped by S1); uniqueness uses hash sets, O(n); grant lattice checks are O(1) per entry. Production scale: the complete ten-kind source catalog, 256 roots, and the largest nested lists that fit inside the 1 MiB profile cap; a synthetic 257-source duplicate-kind list exercises cap-first rejection. Maximum accepted cost: under 1 second debug for the complete catalog plus 256-root boundary; rationale: linear hash-set membership over byte-bounded input with no I/O.

**Wall budget/phase:** N/A — reason: one-off phase; no wall budget (validation runs once per load/check).

**Files:** Create `crates/resourcefs-sources/src/configuration/mod.rs`, `crates/resourcefs-sources/src/configuration/grants.rs` (`MutationGrants`, `MutationSupport`, subset validation); modify `crates/resourcefs-sources/src/lib.rs` (`mod configuration` plus re-exports); create `crates/resourcefs-sources/tests/configuration_contract.rs`; create `crates/resourcefs-mcp/src/profile/validate.rs` (`validate_sources`: seen-kind/seen-ID/seen-claim sets, 128-byte ID bound, grant support checks); modify `crates/resourcefs-mcp/src/profile/mod.rs`; extend `crates/resourcefs-mcp/tests/profile_contract.rs` and `crates/resourcefs-mcp/tests/fixtures/profile_corpus.json`.

**Estimate:** 0.75 day.

**Diff estimate:** 750 changed lines (≈300 implementation, ≈350 tests, ≈100 fixtures).

**PR increment:** A — strict-model-schema-grants.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test configuration_contract nested_grants_are_subsets` → every lattice row (kind × operation, nested subset/superset) matches the hand-authored permission lattice item-by-item; the github-delete mutation flips its named row (red), restore → green.
- `cargo test -p resourcefs-mcp --test profile_contract` → absent grants decode false and only literal `true` grants; unsupported-per-kind and null/non-boolean/`writable` rows rejected; each kind accepted once; duplicate kind/ID/claim and 257-row catalogs rejected; the seen-kind mutation flips the duplicate-kind row (red), restore → green.

## Slice 3: Network source configuration — HTTPS, GitHub, SSH, downstream MCP

**Claim IDs:** C8, C9, C10, C17

**Expected behavior:** `HttpsConfig::new` admits only non-overlapping allowlisted HTTPS base prefixes with explicit `allowPrivateNetwork` and safe credential-header references, rejecting userinfo/query/fragment/wildcard hosts, non-HTTPS URLs, duplicate/overlapping prefixes, and routing/hop-by-hop credential headers. `GithubConfig::new` admits only an HTTPS API base (public GitHub default), one referenced credential, canonical deduplicated `owner/repo` entries, and repository grants bounded by source grants. `SshConfig::new` requires one validated shared `CommandSpec` and distinct exact SSH config aliases with non-overlapping absolute remote roots, and remains read-only. `DownstreamMcpConfig::new` requires unique server IDs, collision-free lowercase-normalized RFC-scheme claims (≤64 bytes) that cannot shadow built-ins, and strict stdio-or-HTTPS transports, rejecting tool/prompt proxy fields and true grants. The slice lands the inert shape-validated semantic types `SecretReference`, `CommandSpec`, `ChildEnvironment`/`EnvironmentValue`, and `SchemeClaim` (execution is S5's claim; resolution is S6's claim) plus the mcp DTO→constructor conversion arms for these four kinds.

**Oracle:** C8 — independent Python `urllib.parse` component/prefix oracle over the same named rows. C9 — literal `owner/repo` parser table and permission lattice. C10 — component-wise POSIX remote-root table independent of the source constructor. C17 — independent Python URL parsing plus a hand-coded RFC scheme/collision table.

**Stress fixture:** URL corpus with userinfo, query, fragment, wildcard host, non-HTTPS, duplicate/overlapping prefixes, and the exact-prefix boundary where one base is a path-prefix string of another; duplicate `owner/repo` and case variants; relative/empty/overlapping remote roots; scheme claims at 64/65 bytes, case-fold collisions, and built-in shadowing (`rfs`, `artifact`, `local`, `agent`, `history`, `memory`); 4,096/4,097 list boundaries inherited from S1's decode. Expected outcome per oracle table row, written now.

**Regression fence:** `crates/resourcefs-sources/tests/configuration_contract.rs::{https_policy_matrix,github_policy_matrix,ssh_policy_matrix,downstream_mcp_matrix}` (extended in THIS slice).

**Named mutation:** C8 — treat any parsed URL as valid in `configuration::HttpsConfig::new`; the userinfo row must fail. C9 — skip repository dedup in `GithubConfig::new`; the duplicate row must fail. C10 — replace the absolute-root check with a non-empty check in `SshConfig::new`; the relative row must fail. C17 — normalize scheme without collision check in `DownstreamMcpConfig::new`; the case-fold row must fail. All restored → green.

**Complexity/production scale:** Prefix-overlap detection sorts canonicalized prefixes then adjacent-compares: O(n log n) with n ≤ 4,096 origins; repository/host/server dedup is O(n) hash sets; URL parse is O(url bytes). Production scale: 4,096 origins/hosts/servers per kind, byte-capped by S1's 1 MiB profile. Maximum accepted cost: under 2 seconds debug for an exact-ceiling four-kind catalog; rationale: dominated by ≤4,096 URL parses plus one sort per list, all linear-or-n-log-n local work.

**Wall budget/phase:** N/A — reason: one-off phase; no wall budget (construction runs once per load/check).

**Files:** Create `crates/resourcefs-sources/src/configuration/command.rs` (`CommandSpec`, `EnvironmentValue`, `ChildEnvironment`), `crates/resourcefs-sources/src/configuration/secret_reference.rs` (`SecretReference`), `crates/resourcefs-sources/src/configuration/https.rs`, `crates/resourcefs-sources/src/configuration/github.rs`, `crates/resourcefs-sources/src/configuration/ssh.rs`, `crates/resourcefs-sources/src/configuration/downstream_mcp.rs` (including `SchemeClaim`); modify `crates/resourcefs-sources/src/configuration/mod.rs` and `crates/resourcefs-sources/src/lib.rs`; extend `crates/resourcefs-sources/tests/configuration_contract.rs`; create `crates/resourcefs-mcp/src/profile/convert.rs` (conversion arms for the four network kinds); modify `crates/resourcefs-mcp/src/profile/mod.rs`.

**Estimate:** 1.25 days.

**Diff estimate:** 1,300 changed lines (≈600 implementation, ≈700 tests).

**PR increment:** B — source-configuration.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test configuration_contract` → every named HTTPS/GitHub/SSH/downstream-MCP row's accept/reject agrees item-by-item with the Python `urllib.parse`/RFC-scheme/permission-lattice/POSIX-root oracle tables; each named mutation flips exactly its owning row (red), restore → green.
- `cargo test -p resourcefs-mcp --test profile_contract` → profile-level rows for these four kinds route through the conversion arms and reject exactly what the constructors reject, proving no drift between the DTO boundary and the constructors.

## Slice 4: Local source configuration — documents, skills, rules, memory, vault, agent export

**Claim IDs:** C11, C12, C13, C14, C15, C16

**Expected behavior:** `DocumentsConfig::new` requires converters with disjoint lowercase extensions, an explicit stdin/appended-canonical-path input mode, and a validated `CommandSpec`, and remains read-only. `SkillsConfig::new` and `RulesConfig::new` require distinct contained canonical root directories / manifest paths with only the approved source-level grants. `MemoryConfig::new` requires distinct stable names and distinct contained canonical file-or-directory targets, and remains read-only. `VaultConfig::new` requires distinct names/directories with per-vault grants that are subsets of source grants (S2 machinery). `AgentExportConfig::new` requires a non-empty distinct contained manifest-path list, and remains read-only until its compiled adapter is present. Missing paths and escaping links fail; duplicates are detected by canonical identity, never by spelled path. Includes the mcp conversion arms for these six kinds.

**Oracle:** Filesystem canonical identity plus sibling sentinels (C12–C14, C16); permission lattice plus canonical-directory set (C15); test-owned literal lowercase-extension set and explicit mode table (C11); literal read-only lattice (C16).

**Stress fixture:** TempDir trees with symlinked aliases of the same canonical target (must collide), escaping links, missing paths, file-where-directory expected and vice versa, Unicode/space-containing paths, duplicate names with distinct targets and duplicate targets with distinct names, overlapping/mixed-case/empty extension claims, and per-vault superset grants. Expected: accept only distinct contained canonical identities; every alias/escape/missing row rejected, written now per row.

**Regression fence:** `crates/resourcefs-sources/tests/configuration_contract.rs::{document_converter_matrix,skill_root_matrix,rule_path_matrix,memory_root_matrix,vault_matrix,agent_export_path_matrix}` (extended in THIS slice).

**Named mutation:** C11 — remove the overlap check in `DocumentsConfig::new`; the duplicate-extension row must fail. C12 — skip canonical target dedup in `SkillsConfig::new`; the alias row must fail. C13 — skip the contained-file check in `RulesConfig::new`; the escaping-link row must fail. C14 — deduplicate by name only in `MemoryConfig::new`; the canonical-alias row must fail. C15 — drop the per-vault subset check in `VaultConfig::new`; the superset row must fail. C16 — accept a true grant in `AgentExportConfig::new`; the grant row must fail. All restored → green.

**Complexity/production scale:** Path validation is O(paths) canonicalize syscalls plus O(n) hash-set identity dedup with n ≤ 4,096 per list; extension overlap uses an exact set after lowercase-only validation, O(extensions). Production scale: 4,096 converters/roots/manifests per kind, byte-capped by the 1 MiB profile. Maximum accepted cost: under 5 seconds debug for an exact-ceiling six-kind catalog; rationale: one canonicalize per entry plus linear set operations and no recursive scanning (content validation is intended work in rfs-cdlp and rfs-btji).

**Wall budget/phase:** N/A — reason: one-off phase; no wall budget (construction runs once per load/check).

**Files:** Create `crates/resourcefs-sources/src/configuration/paths.rs` (canonicalize/containment helpers; alternatively lift `strip_beneath`/`normalize_platform_path` from `filesystem.rs` to `pub(crate)` — one home only), `crates/resourcefs-sources/src/configuration/documents.rs`, `crates/resourcefs-sources/src/configuration/skills.rs`, `crates/resourcefs-sources/src/configuration/rules.rs`, `crates/resourcefs-sources/src/configuration/memory.rs`, `crates/resourcefs-sources/src/configuration/vault.rs`, `crates/resourcefs-sources/src/configuration/agent_export.rs`; modify `crates/resourcefs-sources/src/configuration/mod.rs` and `crates/resourcefs-sources/src/lib.rs` (and `crates/resourcefs-sources/src/filesystem.rs` only if helpers are lifted); extend `crates/resourcefs-sources/tests/configuration_contract.rs`; modify `crates/resourcefs-mcp/src/profile/convert.rs` (six new arms).

**Estimate:** 1.25 days.

**Diff estimate:** 1,400 changed lines (≈600 implementation, ≈800 tests).

**PR increment:** B — source-configuration.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test configuration_contract` → every document/skill/rule/memory/vault/Agent-Export row's accept/reject agrees item-by-item with the canonical-identity/lattice oracles; sibling sentinels remain untouched; each named mutation flips exactly its owning row (red), restore → green.
- `cargo test -p resourcefs-mcp --test profile_contract` → profile-level local-kind rows route through conversion with outcomes identical to the constructor matrices (no DTO/constructor drift).

## Slice 5: Bounded command executor — direct argv, minimal environment, admission, whole-tree cleanup

**Claim IDs:** C19, C20, C21, C22

**Expected behavior:** A validated `CommandSpec` executes argv[0] directly (no shell, no interpolation) with literal argv and a cleared environment carrying only present `PATH`, Windows `SystemRoot`, and explicit literal/inherit/secret mappings; the helper role rejects secret-tagged environment mappings so helper commands cannot recursively resolve secrets. `CommandExecutor::run(spec, role, input, &OperationGuard)` admits at most the configured 1–32 live trees under one process-wide semaphore (persistent permits held for child lifetime), drains both pipes concurrently stopping at ceiling+1 per stream, enforces the tiered ceilings (secret helper 5,000 ms/65,536 stdout/65,536 stderr; one-shot converter/SSH 30,000 ms/67,108,864/1,048,576; downstream MCP 10,000 ms startup, 30,000 ms per call, 8,388,608 per frame, 65,536 per stderr line) with exact-limit output succeeding, and on timeout or `OperationGuard` cancellation terminates the owned Unix process group or Windows Job Object, force-kills survivors after at most 1,000 ms grace, closes pipes, reaps every child, releases the permit, and returns a bounded typed `limit_exceeded`/`cancelled` error with zero live descendants.

**Oracle:** C19 — `.rfs-r9m6/oracle_environment.py` (Python subprocess with hand-built environment) plus literal expected argv/env sets. C20 — `.rfs-r9m6/oracle_bounded_pipes.py` (Python reader threads) plus independent child counters. C21 — `.rfs-r9m6/oracle_posix_tree.py` heartbeat-growth oracle independent of process-state implementation. C22 — `.rfs-r9m6/oracle_windows.ps1` explicit PID-termination oracle via `windows_evidence_runner.py`.

**Stress fixture:** (a) Echo child with parent environment poisoned by `RFS_PARENT_SECRET_SENTINEL`, argument `literal;$HOME&|<>()` plus Unicode/spaces → byte-exact argument, environment keys exactly {PATH + explicit} (+SystemRoot on Windows), sentinel absent. (b) Dual endless 4,096-byte writers on both pipes → exactly ceiling+1 observed per stream (helper row 65,537/65,537), typed over-limit error, no deadlock under a 5-second watchdog, child reaped (P4 measured 2–34 ms). (c) Ceiling ∈ {1, 32} permits held plus one-over admission → exact ceiling succeeds, one-over waits then returns `limit_exceeded` at owning-timeout expiry, permits release after exit, persistent-permit lifetime row. (d) SIGTERM-ignoring child with heartbeat grandchild ('x' every 20 ms) → both survive grace, both PIDs absent after force kill, heartbeats frozen. All outcomes recorded now.

**Regression fence:** `crates/resourcefs-sources/tests/process_contract.rs::{direct_argv_and_environment,stream_boundaries,process_admission,unix_tree_cleanup}` plus `#[cfg(windows)] process_contract::windows_tree_cleanup` with the native VM checkpoint — all created in THIS slice.

**Named mutation:** C19 — remove `env_clear` in `process::spawn_direct`; the ambient-secret assertion must fail. C20 — drain stdout before stderr in `CommandExecutor::run`; the dual-writer timeout test must fail. C21 — kill the direct PID instead of the process group in `process/unix.rs`; the grandchild heartbeat must fail. C22 — omit Job assignment in `process/windows.rs`; the grandchild heartbeat must fail. All restored → green.

**Complexity/production scale:** Drain is O(stream bytes), stopping at ceiling+1 per stream; retained output per tree is at most the role ceilings+1 (helper 131,073 bytes; one-shot 68,157,441 bytes); admission is an O(1) semaphore acquire with the owning-timeout deadline; cleanup is O(1) syscalls plus ≤1,000 ms grace poll plus one reap. Production scale: 32 concurrent trees. Maximum accepted cost: never read past configured ceiling+1 per stream and never more than 32 live trees; worst-case buffered output 32 × 68,157,441 ≈ 2.18 GB only at simultaneous one-shot ceilings — rationale: the approved spec ceilings are profile-lowerable only, and no unbounded pipe read is permitted.

**Wall budget/phase:** Always-on command-execution phase (every helper resolution, converter read, SSH operation, downstream call): wall per execution ≤ configured role timeout (≤5,000 ms helper; ≤30,000 ms one-shot; ≤30,000 ms downstream call; ≤10,000 ms downstream startup) + ≤1,000 ms grace + reap (2–34 ms empirical) + admission wait bounded by the owning timeout. Rationale: the approved tiered ceilings plus the approved 1,000 ms grace/force/reap decision.

**Files:** Modify workspace `Cargo.toml` (tokio `process` feature; rustix `process` feature on Unix; windows-sys `Win32_System_JobObjects`/`Win32_System_Threading`/`Win32_Security` on Windows); create `crates/resourcefs-sources/src/process/mod.rs` (facade: sole `tokio::process` token site), `crates/resourcefs-sources/src/process/unix.rs` (private `ChildTree`, sole Unix process FFI site), `crates/resourcefs-sources/src/process/windows.rs` (private `ChildTree`, sole Windows Job FFI site); modify `crates/resourcefs-sources/src/lib.rs` (`pub mod process`); create `crates/resourcefs-sources/tests/process_contract.rs` (re-exec `current_exe` fixture children); create `.rfs-r9m6/windows_test_runner.py` (WinRM upload/run of the cross-compiled test binary in `resourcefs-win11`, modeled on `windows_evidence_runner.py`).

**Estimate:** 2.5 days.

**Diff estimate:** 1,750 changed lines (≈700 production facade and adapters, ≈900 contract tests and re-exec fixture children, ≈130 VM runner, ≈20 manifests/exports).

**PR increment:** C — command-secret-substrate.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --all-features --test process_contract` → `direct_argv_and_environment`: argument `literal;$HOME&|<>()` byte-exact, environment keys exactly {PATH + explicit} (+SystemRoot on Windows), ambient sentinel absent, item-by-item agreement with `oracle_environment.py`; `stream_boundaries`: both streams stop at exactly ceiling+1 (65,537/65,537 helper row), typed over-limit, reaped, no deadlock, agreement with `oracle_bounded_pipes.py`; `process_admission`: exact ceiling succeeds, one-over returns `limit_exceeded` within the owning timeout, permits release after exit.
- `cargo test -p resourcefs-sources --all-features --test process_contract unix_tree_cleanup -- --exact --nocapture` → resistant child/grandchild survive SIGTERM through the 1,000 ms grace, then after group SIGKILL both `/proc/<pid>/stat` entries are absent/Z; heartbeat sizes identical before and after a 300 ms observation window (P5 baseline froze at 54/53 bytes); direct child reaped; bounded typed error returned.
- `./.rfs-r9m6/oracle_bounded_pipes.py && ./.rfs-r9m6/oracle_posix_tree.py` → both independent PASS baselines remain unchanged (65,537 bytes per stream reaped; frozen heartbeats).
- `cargo xwin clippy -p resourcefs-sources --all-targets --all-features --target x86_64-pc-windows-msvc -- -D warnings` → the Windows adapter and native contract compile warning-free against the downloaded MSVC CRT/SDK.
- `RUSTFLAGS='-C target-feature=+crt-static' cargo xwin test -p resourcefs-sources --target x86_64-pc-windows-msvc --all-features --test process_contract --no-run && uv run ./.rfs-r9m6/windows_test_runner.py 'target/x86_64-pc-windows-msvc/debug/deps/process_contract-*.exe'` → the runner resolves exactly one newest matching test executable, streams it over WinRM, and `resourcefs-win11` passes the complete five-test contract plus 20 repeated Job-cleanup runs; retained handles for both reported descendants are signaled within the bounded cleanup window, heartbeat sizes remain identical across the 300 ms observation window, and argv/environment rows match `oracle_windows.ps1`.
- Each named mutation turns its owning fence row red on the expected assertion; restore → green.

## Slice 6: Opaque secret resolution, exactly-one-ending normalization, redactor

**Claim IDs:** C18

**Expected behavior:** `SecretReference` accepts only `environment{name}` and `command{CommandSpec}` variants. Environment resolution returns the present value or a bounded typed error (absent/empty). Command resolution runs the helper through S5's `CommandExecutor` under the secret-helper role, requires UTF-8, removes exactly one final LF or CRLF (no wider trim), preserves every other character including embedded and repeated newlines, and rejects empty/NUL/non-UTF-8/oversized/nonzero-exit/timeout/cancelled outcomes; helper command environments permit only literal/inherit values so resolution cannot recurse. Resolved values live only in the opaque `resourcefs-core::Secret` (no Display/Debug/Serialize, compile-fail enforced); `Redactor` scrubs every registered value before any sink write; no raw value appears in `ResourceError`.

**Oracle:** Child emits exact bytes; the test computes only-one-ending removal from a literal byte table independent of `normalize_helper_stdout`; sentinel scans use plain test-side substring search over captured sinks, independent of `Redactor`.

**Stress fixture:** Literal byte matrix: `value` → preserved; `value\n` → `value`; `value\r\n` → `value`; `value\n\n` → `value\n` (kills `.trim()`); embedded `a\nb` preserved; empty → reject; NUL → reject; 0xFF → reject; 65,536 bytes exact → accept; 65,537 → reject; stderr over 65,536 → reject; nonzero exit → reject; slow helper versus configured timeout → bounded error; recursion attempt → rejected at validation; parent ambient sentinel never crosses. Redaction rows include duplicate secrets and prefix-overlap (`abc`, `abcdef`) and require the longest literal to win so no suffix leaks. Per-row outcomes recorded now.

**Regression fence:** `crates/resourcefs-sources/tests/secret_contract.rs::helper_output_matrix` and `crates/resourcefs-core/tests/secret_opacity_contract.rs` (trybuild compile-fail opacity plus `Redactor` behavior) — both created in THIS slice. This is the complete C18 fence: helper-output normalization, opacity, and redactor contracts are fully dischargeable here; the compiled all-sink sentinel matrix is C33's separate fence in S11.

**Named mutation:** Replace `normalize_helper_stdout` with `.trim()`; the repeated-newline row must fail. Restored → green.

**Complexity/production scale:** Resolution is O(helper stdout ≤ 65,536 bytes): one UTF-8 scan plus one ending strip, at most one resolution per reference per process. `Redactor` deduplicates secrets and compiles one `aho-corasick` automaton with leftmost-longest matching in O(total secret bytes) at construction; `scrub` is O(diagnostic bytes + matches) and produces at most one output allocation. Production scale: ≤256 sources plus ≤256 environment mappings per command implies ≤1,024 registered secrets. Maximum accepted cost: constructing the 1,024-secret redactor ≤250 ms and redacting a 65,536-byte diagnostic ≤50 ms with output capacity bounded by the redacted result — rationale: `aho-corasick` 1.1.5 is already locked transitively, leftmost-longest matching prevents prefix leakage, and redaction precedes every sink write.

**Wall budget/phase:** Resolution: N/A — reason: one-off phase; no wall budget (once per reference at authority construction/probe, wall-bounded by the 5,000 ms secret-helper ceiling via S5). Redaction: always-on (every log/report/diagnostic write): ≤50 ms per ≤65,536-byte diagnostic at the 1,024-secret extreme, ≤5 ms typical at ≤4 KiB — rationale: bounded linear byte replacement with no I/O.

**Files:** Modify workspace `Cargo.toml` (promote the already-locked `aho-corasick` 1.1.5 to a pinned workspace dependency) and `crates/resourcefs-core/Cargo.toml` (`aho-corasick` dependency, trybuild dev-dependency); create `crates/resourcefs-core/src/secret.rs` (opaque `Secret`, immutable compiled `Redactor`); modify `crates/resourcefs-core/src/lib.rs` (exports); create `crates/resourcefs-core/tests/secret_opacity_contract.rs` with `tests/ui/` compile-fail fixtures; create `crates/resourcefs-sources/src/secret.rs` (`resolve(reference, &CommandExecutor, &OperationGuard, env_lookup)` with an environment-lookup closure because edition-2024 `std::env::set_var` is unsafe; `normalize_helper_stdout`); modify `crates/resourcefs-sources/src/lib.rs` (`pub mod secret`); create `crates/resourcefs-sources/tests/secret_contract.rs`.

**Estimate:** 1 day.

**Diff estimate:** 850 changed lines (≈300 production, ≈400 contract tests, ≈60 compile-fail fixtures, ≈40 manifests/exports, ≈50 corpus tables).

**PR increment:** C — command-secret-substrate.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --all-features --test secret_contract` → every `helper_output_matrix` row agrees item-by-item with the literal byte table (including `value\n\n` → `value\n`, 65,536 accepted, 65,537 rejected); absent/empty environment, nonzero, timeout, cancellation, and recursion rows return bounded typed redacted errors; the ambient sentinel appears in zero captured sinks.
- `cargo test -p resourcefs-core --test secret_opacity_contract` → Display/Debug/Serialize attempts on `Secret` fail compilation; `Redactor::scrub` removes every registered sentinel, deduplicates equal values, and removes the longer prefix-overlapping value without leaking its suffix.
- `cargo xwin clippy -p resourcefs-core -p resourcefs-sources --all-targets --all-features --target x86_64-pc-windows-msvc -- -D warnings` → the complete secret substrate and native helper contract compile warning-free against the downloaded MSVC CRT/SDK.
- Mutation: swap `normalize_helper_stdout` to `.trim()`, re-run `helper_output_matrix` → the repeated-newline row is red; restore → green.

## Slice 7: Complete check — static offline validation and one-attempt probing with deterministic reports

**Claim IDs:** C26, C27

**Expected behavior:** `resourcefs check --config <path>` loads and fully validates the profile (schema, paths, grants, claims, environment-variable presence for environment secret references, executable resolution for command argv[0] — bare names against the configured child PATH, separator-containing paths from the profile directory), performs no network access, no source mutation, and no helper execution, and emits one deterministic redacted JSON report `{ok, schemaVersion, probe:false, sources:[{id,kind,required,state:"notProbed"}]}` with stable field ordering and one trailing newline on stdout, empty stderr, exit 0; any usage/load/static failure produces empty stdout, a bounded field-identifying stderr diagnostic with no credential material, and exit 2. With `--probe`, the source-neutral `resourcefs-core::probe` seam and `resourcefs-sources::ProbeRunner` attempt every configured source exactly once within serving limits (local kinds open/validate paths/manifests, secret references resolve through S6, network kinds perform bounded read-only connectivity, SSH runs controlled remote `true`, downstream MCP initializes and lists Resources, document probes resolve executables without conversion), and the report carries each `available`/`degraded`/`failed`/`unsupported` state with an optional redacted diagnostic; optional degradation keeps `ok` true and exit 0; any required failure or adapter unsupported by the binary makes `ok` false and exit 3. Static and probe concerns land in one slice because C26 and C27 are atomic: the literal C26 mutation calls the `ProbeRunner` created here.

**Oracle:** C26 — denied-network listener accept counter (must stay zero) plus the literal ordered JSON report shape. C27 — test-owned state truth table and attempt files/counters independent of `ProbeRunner`.

**Stress fixture:** Valid zero/one/many-source profiles and invalid path/grant/claim/env-name/executable variants run under a local TCP listener wired as the network sources' endpoints; two consecutive runs of the same valid profile must produce byte-identical reports; a profile referencing a missing environment variable and an unresolvable executable fails with exit 2 naming the field; counted fake probe adapters produce every `available`/`degraded`/`failed`/`unsupported` state across optional/required combinations, zero/many sources, and diagnostic absent/present rows, with per-adapter attempt counters. Expected: zero accepted connections, deterministic bytes, exactly one attempt per source, correct exit class per truth-table row — all written now.

**Regression fence:** `crates/resourcefs-mcp/tests/cli_contract.rs::static_check_is_offline_and_deterministic` (created in THIS slice) and `crates/resourcefs-mcp/tests/probe_contract.rs::probe_state_and_exit_matrix` (created in THIS slice, backed by counted fake probe adapters in `crates/resourcefs-sources/tests/probe_runner_contract.rs`).

**Named mutation:** C26 — call `ProbeRunner` from static `cli::check`; the network counter must fail. `ProbeRunner` is created in this same slice, so the literal mutation applies at this slice's checkpoint. C27 — add retry-on-failure in `ProbeRunner`; the attempt count must fail. Both restored → green.

**Complexity/production scale:** Static check is the S1–S4 one-off pipeline plus O(environment mappings + commands) lookups (≤256 mappings and ≤256 argv elements per command, ≤256 sources); executable resolution is one PATH search per distinct bare argv[0] with memoization, O(unique executables × PATH entries). Probing is O(sources) bounded attempts, each bounded by its role ceilings. Production scale: exact-ceiling 1 MiB profile with 256 sources. Maximum accepted cost: static check under 5 seconds debug / 2 seconds release end-to-end for the exact-ceiling profile; rationale: bounded linear validation plus at most 256 bounded PATH searches and zero network I/O by construction; probe wall is bounded by the per-source role ceilings already budgeted in S5.

**Wall budget/phase:** N/A — reason: one-off phase; no wall budget (one check run per command invocation; each probe attempt is wall-bounded by the S5 role ceilings).

**Files:** Create `crates/resourcefs-core/src/probe.rs` (source-neutral `SourceProbe` seam, four-state `ProbeState`, redacted diagnostic); modify `crates/resourcefs-core/src/lib.rs`; create `crates/resourcefs-sources/src/probe.rs` (`ProbeRunner`, one bounded attempt per configured source, `unsupported` without fabricated adapters); modify `crates/resourcefs-sources/src/lib.rs`; create `crates/resourcefs-sources/tests/probe_runner_contract.rs`; modify `crates/resourcefs-mcp/src/cli.rs` (additive `Command::Check { config, probe }`), `crates/resourcefs-mcp/src/main.rs` (typed outcome → 0/2/3 exit mapping for the check path; serve path and the complete 0/2/3/1 matrix are S11's C3 claim), `crates/resourcefs-mcp/src/profile/mod.rs` (check entry composing load → validate → convert → env/executable validation); create `crates/resourcefs-mcp/src/profile/report.rs` (deterministic redacted report rendering through S6's `Redactor`), `crates/resourcefs-mcp/tests/cli_contract.rs`, `crates/resourcefs-mcp/tests/probe_contract.rs`, `crates/resourcefs-mcp/tests/fixtures/check/` (valid/invalid profile fixtures).

**Estimate:** 1.75 days.

**Diff estimate:** 1,800 changed lines (≈600 implementation, ≈900 tests, ≈300 fixtures).

**PR increment:** D — complete-check-reporting.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test cli_contract static_check_is_offline_and_deterministic` → compiled binary under the denied-network listener: zero accepted connections across all fixtures; valid rows emit the literal ordered `notProbed` report byte-identically on repeat runs with empty stderr and exit 0; invalid rows emit empty stdout and a bounded field-identifying stderr diagnostic without credential material, exit 2; the literal ProbeRunner-in-static-check mutation drives the accept counter above zero (red), restore → green.
- `cargo test -p resourcefs-mcp --test probe_contract probe_state_and_exit_matrix` → every configured source is attempted exactly once (attempt counters/files), the exhaustive ordered redacted report covers all four states across optional/required combinations, exit is 0 when only optional sources are degraded and 3 for any required failure or unsupported adapter, completed probe reports keep stderr empty even when `ok` is false; the retry-on-failure mutation inflates the attempt count (red), restore → green.
- `cargo test -p resourcefs-sources --test probe_runner_contract` → counted fake probes agree item-by-item with the test-owned state truth table; no source is skipped or retried; unsupported kinds report `unsupported` without fabricated adapters.
- `cargo test -p resourcefs-mcp --test profile_contract --test schema_contract --test architecture_contract && cargo test -p resourcefs-sources --test configuration_contract --test process_contract --test secret_contract && cargo test -p resourcefs-core --test secret_opacity_contract` → every earlier slice fence remains green after additive check/probe/report wiring.

## Slice 8: Lower-only ServerLimits aggregate and caller migration

**Claim IDs:** C23

**Expected behavior:** A new core aggregate `ServerLimits` validates the nested `limits.text`, `limits.discovery`, `limits.imageBytes`, and `limits.storage` groups against the exported binary ceiling constants (49,152 text bytes/3,000 lines/512 columns; 1,000 search matches/glob entries/listing entries; 5,242,880 image bytes; 67,108,864 object bytes; 268,435,456 session bytes): omitted fields use binary ceilings, present values must be positive, exact ceilings succeed, one-over values fail rather than clamp, and `storage.objectBytes` never exceeds `storage.sessionBytes`. `ReadEngine`/`DiscoveryEngine`/`PathSession` constructors take base limits with effective = min(base, per-call) lowering; every existing caller migrates at default ceilings so observable behavior is preserved. `MAX_IMAGE_BYTES = 5,242,880` is newly exported.

**Oracle:** Literal maxima table (49,152 bytes/3,000 lines/512 columns; 1,000 search/glob/listing entries; 5,242,880 image bytes; 67,108,864 object bytes; 268,435,456 session bytes) plus signed inequalities, independent of the constructors.

**Stress fixture:** Every nested group absent/empty/present; signed rows -1/zero/exact binary maximum/one-over for every field; objectBytes less/equal/greater than sessionBytes. Expected: omitted → binary ceiling, exact → accept, -1/zero/one-over → fail with the field named, objectBytes > sessionBytes → fail; written now per row.

**Regression fence:** `crates/resourcefs-core/tests/server_limits_contract.rs` (created in THIS slice) plus the profile limits mapping corpus added to `crates/resourcefs-mcp/tests/profile_contract.rs` in THIS slice.

**Named mutation:** Clamp `text.bytes` in `ServerLimits::new`; the one-over row must fail. Restored → green.

**Complexity/production scale:** Validation is O(limit fields), a fixed small constant; per-call effective-limit merge is O(1) scalar minima. Production scale: one aggregate per process plus one merge per read/discovery call. Maximum accepted cost: construction under 1 ms; rationale: constant-field comparisons against exported constants with no I/O or allocation beyond the aggregate.

**Wall budget/phase:** Always-on per-call effective-limit merge (every read/discovery call): wall budget ≤1 ms per call — rationale: a fixed number of integer comparisons, no I/O. Construction is one-off at startup.

**Files:** Create `crates/resourcefs-core/src/limits.rs` (`ServerLimits`, `MAX_IMAGE_BYTES` export); modify `crates/resourcefs-core/src/read.rs`, `crates/resourcefs-core/src/discovery.rs`, `crates/resourcefs-core/src/session.rs` (base limits/storage quotas replace hard-coded constants), `crates/resourcefs-core/src/lib.rs`; create `crates/resourcefs-core/tests/server_limits_contract.rs`; extend `crates/resourcefs-mcp/tests/profile_contract.rs` (limits mapping corpus); migrate all caller sites at defaults (`crates/resourcefs-sources/src/session_storage.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/src/server.rs`, core tests `path_session`/`read_engine`/`discovery_engine`, sources tests `artifact_adapter`/`compiled_sources`/`filesystem_adapter`/`filesystem_resource_limits`).

**Estimate:** 1 day.

**Diff estimate:** 650 changed lines (≈250 implementation, ≈300 tests, ≈100 caller migrations).

**PR increment:** E — limits-retention-logging.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test server_limits_contract` → every signed/exact/one-over row and the objectBytes ≤ sessionBytes rows agree item-by-item with the literal maxima table; omitted groups yield the exported binary ceilings; the clamp mutation flips the one-over row (red), restore → green.
- `cargo test -p resourcefs-mcp --test profile_contract` → the limits mapping corpus decodes to `ServerLimits` values identical to direct constructor outcomes (no DTO/constructor drift).
- `cargo test --workspace --all-targets` → every migrated caller is green at default ceilings with no observable behavior change.

## Slice 9: Session retention configuration under the namespaced cache child

**Claim IDs:** C24

**Expected behavior:** `SessionStore` deepens with validated `SessionStorageConfig { cache_root, retention_ttl }` and `open_with`: TTL accepts 0 through 86,400 seconds as an elapsed whole-second duration with UTC persisted timestamps (-1 and 86,401 rejected), retained state lives only under ResourceFS's canonicalized namespaced cache child, TTL zero deletes session state at disconnect after lease release, and cleanup never traverses or deletes sibling paths. All callers migrate in this slice.

**Oracle:** Literal signed TTL interval, explicit timestamp arithmetic, and an independent directory snapshot, independent of the storage implementation.

**Stress fixture:** TTL -1/0/86,400/86,401 boundary rows; relative and absolute cache bases; permission denial; injected elapsed timestamps (no local-time/DST arithmetic); a sibling marker tree beside the namespaced child. Expected: -1 and 86,401 rejected; 0 deletes at disconnect; 86,400 retains until elapsed expiry; sibling snapshot byte-identical before and after every cleanup; written now per row.

**Regression fence:** Extended `crates/resourcefs-sources/tests/session_storage_contract.rs` (extended in THIS slice).

**Named mutation:** Remove the namespace append in `SessionStore::open_with`; the sibling marker test must fail. Restored → green.

**Complexity/production scale:** Cleanup is O(retained session entries) file operations under one namespaced directory; timestamp comparison is O(1) per entry. Production scale: retained state bounded by the 268,435,456-byte session ceiling. Maximum accepted cost: cleanup of a full retained set under 5 seconds debug; rationale: bounded linear directory traversal of ResourceFS-owned entries only, no recursive traversal outside the namespace.

**Wall budget/phase:** N/A — reason: one-off phase; no wall budget (cleanup runs once per disconnect/startup event, not on a background tick).

**Files:** Modify `crates/resourcefs-sources/src/session_storage.rs` (`SessionStorageConfig`, `SessionStore::open_with`, TTL-parameterized elapsed cleanup, TTL-0 disconnect deletion), `crates/resourcefs-sources/src/lib.rs` (export); extend `crates/resourcefs-sources/tests/session_storage_contract.rs`; migrate all `SessionStore` caller sites (mcp server/render and sources/core tests) to the validated config at approved defaults.

**Estimate:** 0.75 day.

**Diff estimate:** 450 changed lines (≈150 implementation, ≈250 tests, ≈50 caller migrations).

**PR increment:** E — limits-retention-logging.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --all-features --test session_storage_contract` → TTL -1 rejected; 0/86,400/86,401 rows behave per the literal signed interval with injected elapsed timestamps; TTL zero deletes session state at disconnect; the sibling snapshot is untouched; the namespace-append mutation flips the sibling marker row (red), restore → green.
- `cargo test --workspace --all-targets` → all migrated callers green at the approved default cache base and 86,400-second TTL.

## Slice 10: Serialized redacting stderr/rotating-file logging sink

**Claim IDs:** C25

**Expected behavior:** A new internal `resourcefs-mcp::logging` module provides a serialized sink: omitted logging means `info` to stderr; a `stderr` or rotating `file` destination with `error`/`warn`/`info`/`debug` levels; file rotation at or before 10,485,760 bytes (default 10,485,760, lower-only) retaining exactly the configured 1–10 files (default 3) without touching sibling paths; relative file paths resolve from the profile directory; every message passes S6's `Redactor` before any sink write; nothing reaches protocol stdout. The sink is fenced standalone in this slice; serve-time application lands with S11's `server::serve(LaunchPlan)`.

**Oracle:** Literal signed interval plus an independent directory snapshot and byte scan, independent of the sink implementation.

**Stress fixture:** Signed size/retention rows -1/0/1/10/11 and exact/over rotation boundaries; concurrent writers forcing rotations; secret-bearing diagnostic messages with a unique sentinel; sibling files beside the log family. Expected: -1/0/11 retention and -1 size rejected; rotation occurs at or before the configured size; exactly the configured file count retained; concurrent messages serialized without interleaving loss; sibling files byte-identical; the sentinel appears in zero log bytes; written now per row.

**Regression fence:** `crates/resourcefs-mcp/tests/logging_contract.rs` (created in THIS slice).

**Named mutation:** Write before `Redactor::scrub` in `LogSink::write`; the sentinel test must fail. Restored → green.

**Complexity/production scale:** Each write is O(message bytes × registered secrets) redaction plus one bounded append; rotation is O(retained files ≤ 10) renames, serialized under one lock. Production scale: messages bounded by redacted diagnostic size (≤65,536 bytes), 10,485,760-byte files, 10 retained files. Maximum accepted cost: ≤50 ms per ≤65,536-byte message at the 1,024-secret extreme (S6 budget), rotation serialized and bounded — rationale: bounded linear redaction plus constant-file-count rotation, never touching paths outside the configured log family.

**Wall budget/phase:** Always-on logging phase (every diagnostic write): ≤5 ms per typical ≤4 KiB message, ≤50 ms at the 65,536-byte/1,024-secret extreme including one serialized rotation — rationale: bounded linear byte work plus one bounded file append; no network or unbounded I/O.

**Files:** Create `crates/resourcefs-mcp/src/logging.rs` (serialized stderr/rotating-file `LogSink` consuming `Redactor`); modify `crates/resourcefs-mcp/src/lib.rs` (`mod logging`); create `crates/resourcefs-mcp/tests/logging_contract.rs`.

**Estimate:** 1 day.

**Diff estimate:** 600 changed lines (≈250 implementation, ≈350 tests).

**PR increment:** E — limits-retention-logging.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test logging_contract` → defaults are info-on-stderr; signed boundary rows rejected per the literal interval; rotation occurs at or before 10,485,760 bytes and retains exactly the configured 1–10 files; concurrent writers stay serialized; sibling files untouched; the secret sentinel appears in zero captured log bytes; the write-before-scrub mutation flips the sentinel row (red), restore → green.
- `cargo test -p resourcefs-mcp --test architecture_contract` → no telemetry/exporter dependency introduced by the logging module (dependency scan remains green).

## Slice 11: Serve launch authority, immutable LaunchPlan, exit/channel matrix, compiled all-sink acceptance

**Claim IDs:** C2, C3, C5, C28, C30, C31, C33

**Expected behavior:** The CLI exposes only `serve`, `check`, and `schema`; `serve` accepts exactly one explicit `--config` profile or one-or-more CLI roots (rejecting any `--config`/`--root` or `--config`/`--primary-root` mix), resolving profile paths from the canonical profile directory and CLI paths from the launch directory. An immutable `LaunchPlan` built once from the `CheckedProfile` feeds the existing `LaunchRootSource::Profile` variant; serving supports profile Workspace Roots and scratch-only operation with no implicit launch-directory authority, and rejects every configured source kind lacking a compiled adapter with exit 3 before stdio starts. Every invocation maps to the approved 0/2/3/1 exit classes with exact stdout/stderr contracts and no telemetry or diagnostic bytes in MCP stdout. Profile Workspace Roots reuse the existing canonical root set, hidden-default visibility, exact Primary Root, and client-replaces-launch behavior, including an empty scratch-only launch set. Serving never reloads its profile while independent subsequent invocations reread the file, and Path Sessions remain connection-isolated. Operator JSON/schema/logging stays in `resourcefs-mcp`, configuration/secret/process code in `resourcefs-sources`, source-neutral limit/probe/secret types in `resourcefs-core`, and platform process FFI behind the sources process facade. The compiled all-sink matrix proves no resolved secret reaches process stderr, logging stderr/files, static or probe reports, CLI diagnostics, or raw MCP protocol stdout.

**Oracle:** C2 — hand-built temp directories with disjoint sentinels and the expected clap matrix. C3 — the literal approved 0/2/3/1 byte table plus a denied-network fixture. C5 — canonical temp-directory identities and the existing CLI-root contract. C28 — MCP initialize/read transcript and the literal compiled-support table. C30 — file version sentinels and independent process transcripts. C31 — literal allowed-path matrix independent of module visibility and call behavior. C33 — test-owned unique sentinels and raw byte substring scans independent of renderer, logger, protocol, and error types.

**Stress fixture:** Distinct profile/CWD temp roots with disjoint sentinel files (a mixed or bare invocation, or a read returning the wrong sentinel, fails); the full CLI outcome matrix across success/config/unavailable/internal rows; profile-launch MCP reads before and after non-empty client Roots; scratch-only profile plus a launch-directory sentinel that must stay unreachable; one configured entry per uncompiled source kind; a profile mutated and deleted after a gated startup with a version-sentinel change observed only by the next invocation; one unique secret sentinel per sink row (process stderr, logging stderr/files, static report, probe report, CLI diagnostic, MCP stdout) across success and every fixture failure. Expected outcomes written now per row.

**Regression fence:** `crates/resourcefs-mcp/tests/cli_contract.rs::{selects_exactly_one_launch_authority,exit_and_channel_matrix,profile_workspace_authority,profile_serve_matrix,profile_is_immutable_per_process,secret_never_reaches_observable_channels}` (extended/created in THIS slice), `crates/resourcefs-mcp/tests/architecture_contract.rs::{forbids_telemetry_dependencies,profile_capabilities_stay_in_owning_modules}` (extended in THIS slice), the stdio MCP profile launch test in `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` (extended in THIS slice), and the existing root-refresh contracts.

**Named mutation:** C2 — remove the `--config` conflict in `cli.rs::ServeArgs`; the mixed-authority row must fail. C3 — restore the `main.rs` all-errors-to-1 branch; the exit matrix must fail. C5 — change `filesystem.rs` refresh to union launch/client; the profile-root read after client replacement must fail. C28 — skip unsupported entries in `LaunchPlan`; the unsupported-source row must fail. C30 — re-read the profile inside `server` root refresh; the current-process sentinel must fail. C31 — move a `tokio::process::Command` use into `resourcefs-mcp::launch`; the owner scan must fail naming that file. C33 — replace `Redactor::scrub` with an input clone; every secret-bearing sink row must fail. All restored → green.

**Complexity/production scale:** Launch composition is O(profile sources + roots), already byte-capped by S1; the unsupported-kind registry check is O(sources); the sentinel matrix is O(sink bytes) substring scans. Production scale: exact-ceiling profile, 256 sources, 32 live process trees. Maximum accepted cost: launch composition under 5 seconds debug for the exact-ceiling profile — rationale: one bounded linear pass over already-validated structures plus one immutable snapshot handoff; steady-state serving adds no new per-request work beyond the existing root-refresh path.

**Wall budget/phase:** N/A — reason: one-off phase; no wall budget (launch composition runs once per process; the only steady-state paths are the pre-existing root refresh and the S5/S6/S10 budgets already recorded).

**Files:** Modify `crates/resourcefs-mcp/src/cli.rs` (`--config` on `serve` with clap conflicts against roots/primary-root; exactly-one-authority validation), `crates/resourcefs-mcp/src/main.rs` (complete typed `CliFailure` → 0/2/3/1 mapping replacing the single FAILURE branch); create `crates/resourcefs-mcp/src/launch.rs` (immutable `LaunchPlan` from `CheckedProfile` or CLI roots, profile-relative base resolution, compiled-registry support check before stdio); modify `crates/resourcefs-mcp/src/server.rs` (`serve(LaunchPlan)` applies `ServerLimits`/`SessionStorageConfig`/logging and reuses the existing client-root replace-not-union refresh unchanged), `crates/resourcefs-mcp/src/lib.rs` (`mod launch`); extend `crates/resourcefs-mcp/tests/cli_contract.rs`, `crates/resourcefs-mcp/tests/architecture_contract.rs`, and `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` (harness `start_config` gains `--config` mode; profile launch test; shared session-root isolation proof).

**Estimate:** 2 days.

**Diff estimate:** 2,100 changed lines (≈800 implementation, ≈1,100 tests, ≈200 fixtures).

**PR increment:** F — serving-compiled-acceptance.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test cli_contract selects_exactly_one_launch_authority` → compiled binary in distinct profile/CWD sentinel roots: only `serve`/`check`/`schema` exist; mixed `--config`/`--root` and `--config`/`--primary-root` rows and bare modes fail with exit 2 and empty stdout; profile-relative and launch-relative reads return their own disjoint sentinels; the conflict-removal mutation flips the mixed-authority row (red), restore → green.
- `cargo test -p resourcefs-mcp --test cli_contract exit_and_channel_matrix` → every outcome row matches the literal approved 0/2/3/1 byte table with exact stdout/stderr contracts; the denied-network fixture observes zero outbound connections; the all-errors-to-1 mutation flips the matrix (red), restore → green.
- `cargo test -p resourcefs-mcp --test cli_contract profile_workspace_authority` → profile-launch reads before/after non-empty client Roots follow replace-not-union with unique IDs/canonical paths, hidden-default visibility, and exact-single-root relative references; the union mutation flips the post-replacement read row (red), restore → green.
- `cargo test -p resourcefs-mcp --test cli_contract profile_serve_matrix && cargo test -p resourcefs-mcp --test stdio_mcp_contract` → scratch-only initialize succeeds without launch-directory authority; every uncompiled configured kind exits 3 before stdio; profile root read returns the profile sentinel; client non-empty Roots replace rather than union launch roots; connection isolation remains green.
- `cargo test -p resourcefs-mcp --test cli_contract profile_is_immutable_per_process` → mutating and deleting the profile after a gated startup leaves the current process's behavior on the old version sentinel while the next invocation observes the change; the refresh-re-read mutation flips the current-process sentinel row (red), restore → green.
- `cargo test -p resourcefs-mcp --test architecture_contract` → `forbids_telemetry_dependencies` and `profile_capabilities_stay_in_owning_modules` green; the `tokio::process`-in-`launch` mutation fails naming that file, restore → green.
- `cargo test -p resourcefs-mcp --all-features --test cli_contract secret_never_reaches_observable_channels` → the compiled binary runs every sink row (process stderr, logging stderr/files, static report, probe report, CLI diagnostics, raw MCP stdout) with one unique sentinel per row across success and every fixture failure; raw byte scans find zero complete or partial sentinel occurrences; the `Redactor::scrub` input-clone mutation flips every secret-bearing sink row (red), restore → green.
- `cargo test --workspace --all-targets` → every fence from slices 1–10 remains green.

## Execution record

- Slices 1–10: all assigned C1/C4/C6–C27/C29/C32 fences remain PASS from their 2026-08-21 checkpoints.
- Slice 11: C2/C3/C5/C28/C30/C31/C33 completed 2026-08-22. Named mutations for C2/C3/C5/C28/C31/C33 turned their assigned fences red and were restored green; C30 is enforced by the immutable `LaunchPlan` handoff plus current/next-process profile sentinels.
- Final Linux gates: `cargo fmt --all -- --check`, workspace all-target/all-feature check and Clippy with warnings denied, 243 all-target tests, and all doctests passed. The compiled stdio smoke negotiated MCP `2026-07-28`.
- Final differential/fuzz gates: generated profile schema hash matched the checked-in fixture (`fb9514b7cd99f570908b9e027e4a2e6ab4fe82ace9b2d1370c1c417253d2ee48`); all independent profile oracles passed; the nightly `server_profile` target completed 1,669,839 executions in 61 seconds without a crash.
- Final native Windows gate: the statically linked MSVC `cli_contract` binary passed all seven compiled CLI/profile tests under a native Windows 11 VM. Windows process-tree evidence remained PASS.
- Final macOS gate: `cargo zigbuild --target x86_64-apple-darwin --workspace --all-features --tests` compiled every crate and test target successfully. Packaged native release acceptance remains assigned to `rfs-5os7` and `rfs-58r1`.

## Tracker taxonomy

Deferral-phrase scan of this plan ("deferred", "out of scope", "follow-up", "future work", "later", "revisit if", "tracked at", "next PR", "as part of"):

- SKILL content projection/mutation and native rule manifest content validation (referenced in slice 4): intended future work, verified tracker ID rfs-cdlp.
- Agent Export manifest content, global agent-ID reconciliation, and projections (referenced in slice 4): intended future work, verified tracker ID rfs-btji.
- Converter execution consumption (slices 3/4 define command configuration only): intended future work, verified tracker ID rfs-9o5v.
- SSH adapter behavior beyond configuration/probe (slice 3): intended future work, verified tracker ID rfs-vl7z.
- HTTPS runtime/required-versus-degraded connectivity behavior (slice 3): intended future work, verified tracker ID rfs-g2z9.
- Memory read/search behavior (slice 4): intended future work, verified tracker ID rfs-uo2w.
- Downstream MCP Resource proxy behavior (slice 3): intended future work, verified tracker ID rfs-azd3.
- GitHub source behavior and allowed mutations (slice 3): intended future work, verified tracker IDs rfs-45ww and rfs-by2z.
- Vault projection/mutation behavior (slice 4): intended future work, verified tracker ID rfs-nb4s.
- Workspace mutation behavior and atomic writes (grant representation only here): intended future work, verified tracker ID rfs-73dz.
- Kiro release acceptance and packaged distribution of this CLI: intended future work, verified tracker IDs rfs-5os7 and rfs-58r1.
- Intra-plan ordering phrasing between slices and increments — "stacked on", "lands with S11", "C33's separate fence in S11", "serve path and the complete 0/2/3/1 matrix are S11's C3 claim", "serve-time application wiring is not yet present", "execution is S5's claim; resolution is S6's claim" — names dependency order inside this plan; it defers nothing outside the change and is not a tracker item.
- Permanent non-goals (no interactive profile manager, discovery, layering, includes, hot reload, dynamic plugins, telemetry, remote transport, tool/prompt proxying, automatic retry, profile grants over Session Scratch, ceiling increases/clamping; soft deletion, replication lag, and multi-tenancy N/A): rationale recorded in approved `design.md` negative space; no tracker issues.

## Self-review

- [x] Every design row C1–C33 is assigned to exactly one slice; every slice's Claim IDs exist in the design table; every `PENDING — checkpointed-build` falsifier is discharged by the slice implementing its claim, with the exact falsifier experiment and expected outcome in that slice's Commands field (C1's PASS fence is re-run and mutated in S1).
- [x] Every slice has all thirteen mandatory fields, with `N/A — reason` in conditional cells.
- [x] Every claim's fence is created in the slice implementing it; every new fence carries its design-approved named mutation; no fence-less claim exists (design approved zero risk acceptances).
- [x] Every new loop states asymptotic cost, production-scale input sizes, resulting bound, and explicit maximum accepted cost with rationale; every always-on phase (S5 command execution, S6 redaction, S8 per-call limit merge, S10 logging) has a wall budget with rationale; one-off phases record `N/A — reason: one-off phase; no wall budget`.
- [x] The partition rule was applied exactly: sum 13,950 + documented 25% rounded-up churn margin 3,488 = 17,438 > 4,000, producing six independently mergeable increments; every slice names its increment; every increment has a mergeable definition and verifies without the increments after it; spec delivery seams are reconciled explicitly.
- [x] The tracker taxonomy is applied: every deferral phrase above is classified, and every intended-future-work item cites a tracker ID verified via `rivets show`.
- [x] The plan declares no slice complete; `checkpointed-build` exclusively judges completion.
