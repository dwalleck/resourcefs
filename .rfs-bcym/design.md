# Design: rfs-bcym

## Route and inputs

- Route: **Empirical** (`.rfs-bcym/route.md`).
- Behavior source: `spec.md` is `N/A — behavior fully explicit`; `.rfs-bcym/route.md` T4 is the complete given/when/then set. It requires a checked-in non-secret manifest; at least two Jira projects and two Confluence spaces; issues/pages/comments, hierarchy, ADF/storage, forced pagination, and a reader-concealed object; idempotent `bootstrap`, exact `verify`, owned repeat-safe `cleanup`; execution-only provisioner/reader credentials; stable ID/reference-only local state; and bounded non-secret failures.
- Empirical source: `.rfs-bcym/evidence.md` P1–P5 and `.rfs-bcym/probe_openapi.py`. Current Jira and Confluence REST contracts expose the required lifecycle, identity, representation, pagination, private-space, and deletion shapes. Confluence space deletion is asynchronous and must be polled; every consumed response member remains untrusted and is validated at point of use.
- Spec edge cases and delivery increments: `N/A — no spec.md`; the route behavior, parent decisions, evidence learnings, and input shapes below are authoritative.

## Input shapes

| ID | Production-reachable input shape | Status |
|---|---|---|
| S1 | Mode is absent, `bootstrap`, `verify`, `cleanup`, `--help`, or an unknown/extra argument. | Covered by C2, C4, C10 |
| S2 | Manifest/state path is default, relative, absolute, missing, unreadable, a directory, or has an unwritable parent. | Covered by C3, C6, C10 |
| S3 | Tenant is a canonical HTTPS origin, has one trailing slash, or is empty/HTTP/userinfo/path/query/fragment/malformed/Unicode-host input. | Covered by C4 |
| S4 | Provisioner credentials are both present, one missing, empty, or contain CR/LF; reader credentials have the same matrix and are required only for `verify`. | Covered by C4, C10 |
| S5 | Manifest collections are empty, single, valid multi, or contain duplicate logical IDs/project keys/space keys/titles/parent references. | Covered by C3 |
| S6 | Manifest objects include root and child pages, ordinary and private spaces, JSON and complete ADF issue values, exact storage markup, zero/one/multiple comments, ASCII/Unicode/embedded-space content, and optional parents. | Covered by C3, C5, C7, C9 |
| S7 | Local state is absent, valid/current, valid/stale, malformed, a symlink, or left from a partial prior run. | Covered by C5, C6, C8, C10 |
| S8 | Upstream object is absent, marker-owned and matching, marker-owned but drifted, foreign with a reserved key/title, duplicated under one logical marker, or hidden from the reader. | Covered by C5, C7, C8 |
| S9 | HTTP result is expected success, documented absent, permission/auth failure, validation failure, rate/server failure, transport timeout, malformed JSON, missing identity/parent, or mismatched identity/parent. | Covered by C5, C7, C8, C10 |
| S10 | Delete is synchronous 204, already absent 404, or Confluence asynchronous 202 progressing to success/failure/timeout/malformed/repeated status. | Covered by C8, C10 |
| S11 | List/search is empty, one page, two pages with a valid next offset/token/cursor, or returns a missing/malformed/repeated/foreign continuation. | Covered by C7, C9, C10 |
| S12 | State contains zero, one, or multiple stable IDs/references, and an attempted state value contains a secret or mutable response field. | Covered by C6 |
| S13 | Two operator processes target the same state path concurrently. | Covered by C6, C10 |
| S14 | Bootstrap or cleanup is rerun after success or after a bounded partial failure. | Covered by C5, C8 |
| S15 | Fixed polling/page bounds hit zero progress, the last permitted attempt/page, or exceed the bound. | Covered by C8, C9, C10 |

Core move: purely additive. It adds an independent operator module, manifest, state contract, and tests; it removes no existing guard, serialization point, validation, precondition, ordering, or uniqueness invariant.

## Placement

### Operator lifecycle

- **Owner:** `scripts/atlassian-fixture-bootstrap.sh`. One executable module owns argument parsing, manifest validation, credential handling, REST calls, reconciliation, verification, cleanup, atomic state, locking, and bounded diagnostics. This keeps lifecycle knowledge local and independent of ResourceFS mutation modules.
- **New seam:** Chosen interface: one command, `scripts/atlassian-fixture-bootstrap.sh <bootstrap|verify|cleanup> --site <https-origin> [--manifest <path>] [--state <path>]`, with provisioner/reader credentials supplied through named environment variables. Three subcommands behind one interface provide more depth and locality than three scripts. Rejected interface: separate bootstrap/verify/cleanup executables plus sourceable environment output; it duplicates transport/validation/redaction logic and makes callers coordinate several shallow modules.
- **Forbidden:** no `rfs` binary invocation, ResourceFS crate import, ResourceFS mutation call, ACLI, second network client, shell tracing, secret on the command line, raw response/error echo, or ad hoc per-mode transport implementation.

### Fixture description

- **Owner:** `fixtures/atlassian-live/manifest.json`. It owns stable logical fixture shape and marker values, never tenant IDs or credentials.
- **New seam:** no additional interface. The command accepts one versioned strict JSON manifest and hides reconciliation behind its lifecycle interface. A shell-fragment manifest was rejected because sourcing data executes code and makes validation/escaping shallow and unsafe.
- **Forbidden:** no secret, tenant URL, upstream stable ID, mutable timestamp, generated state, or product response body in the checked-in manifest.

### Local fixture state

- **Owner:** `.resourcefs/atlassian-fixture-state.json`, ignored by exact path in `.gitignore`; the operator writes it atomically after successful provisioner validation.
- **New seam:** one versioned JSON document containing only schema version, Site Mount ID/origin, manifest logical IDs, stable Jira/Confluence IDs/keys, and canonical `jira://`/`confluence://` profile references. A sourceable shell file was rejected because quoting becomes part of the interface and live tests can read JSON through existing `jq` tooling.
- **Forbidden:** no account email, token, Authorization value, request/response body, mutable title/summary/timestamp, raw continuation, or temporary file survives a completed command.

Review repair F3 keeps a separate ignored `.resourcefs/atlassian-fixture-state.json.pending` receipt under the same lock. It contains only the validated tenant identity, logical fixture kind/ID, stable container/parent IDs, and a nullable created ID. Before POST, a null ID records an unresolved creation attempt; after an authoritative receipt, the stable ID is atomically recorded before ownership publication. A known-ID recovery validates the content against its manifest and current parent/container authority before publishing a missing ownership property. A conflicting property is never overwritten. An unknown outcome is a bounded explicit error with repair guidance, not permission to repeat POST or adopt an unmarked title collision. Verify never repairs or mutates a pending creation.

### Verification

- **Owner:** `crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs` owns the permanent Unix integration fence and invokes the shipped script through its public interface with a PATH-injected fake `curl`; private test fixtures own production-shaped responses and request logs.
- **New seam:** no production seam. Executable lookup through `PATH` is the existing process seam; production and tests call the same script interface.
- **Forbidden:** tests may not import or duplicate operator reconciliation logic, inspect incidental shell function names, or substitute ResourceFS adapter behavior for REST observations.

## Claims

- **C1.** The current documented Atlassian REST lifecycle is sufficient to create, rediscover, verify, paginate, and remove every manifest object, with bounded polling for Confluence space deletion.
- **C2.** One script exposes exactly three lifecycle modes behind one strict interface and remains independent of ResourceFS production mutation code.
- **C3.** The versioned manifest is rejected before egress unless it contains the complete required multi-project/multi-space shape with unique logical identities and valid parent references.
- **C4.** The command accepts only a normalized HTTPS tenant origin, requires credentials by mode, and keeps both principals' credentials out of argv, tracing, diagnostics, temporary response names, and state.
- **C5.** Bootstrap converges by adopting exact marker-owned objects, creating missing objects, replacing drifted marker-owned disposable containers, and rejecting every foreign or ambiguous collision.
- **C6.** State is lock-serialized, symlink-safe, mode-0600, atomically replaced only after validation, and contains only stable fixture identities and canonical profile references.
- **C7.** Verify independently checks the provisioner graph and reader graph, including exact stable identity/parentage/representations, two-record pagination, visible fixtures, and one provisioner-only private Confluence space.
- **C8.** Cleanup deletes only reserved marker-owned containers, treats documented absence as success, polls Confluence deletion to a bounded terminal result, and is safe after partial or repeated runs.
- **C9.** Two fixtures plus explicit page size one force Jira offset/token and Confluence cursor/Link progression without rate-heavy object counts.
- **C10.** Every invalid input or upstream failure exits nonzero with bounded allowlisted operation/product/logical-object/status context and never emits credentials or raw upstream prose.
- **C11.** One deterministic fake-upstream contract exercises bootstrap twice, verify, cleanup twice, partial recovery, collision refusal, malformed authority, redaction, pagination, and asynchronous deletion through the shipped interface.
- **C12.** A real disposable-tenant smoke runs bootstrap twice, verify, cleanup twice, and a final absence check before the ticket can close.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Published REST lifecycle suffices. | S8–S11, S15 | Traverse current Jira/Confluence OpenAPI; missing any selected create/read/list/delete/private-space/storage/pagination/long-task fact falsifies C1. | Separately rendered Atlassian operation prose and response examples, manually compared. | In `.rfs-bcym/probe_openapi.py`, replace Confluence space delete status `202` with `204`; P4 comparison becomes red. | `.rfs-bcym/probe_openapi.py` plus recorded evidence comparison P1–P5. | <1 minute, public docs | PASS |
| C2 | One independent strict operator interface. | S1 | Invoke help, each valid mode, and invalid argument with no `rfs` executable available; extra modes, successful invalid input, or an `rfs` call falsifies C2. | Rust harness exit/status/request log, independent of shell parsing. | In `scripts/atlassian-fixture-bootstrap.sh`, accept mode `apply`; `operator_interface_is_strict_and_independent` fails. | `operator_interface_is_strict_and_independent` | <1 minute | PASS |
| C3 | Manifest is complete, strict, and pre-egress validated. | S2, S5, S6 | Run malformed/empty/single/duplicate/bad-parent manifests with a fake curl that fails on invocation; any egress or acceptance falsifies C3. | Rust constructs invalid JSON cases and checks zero request-log rows. | Remove duplicate Jira project-key rejection; `invalid_manifest_never_reaches_egress` fails. | `invalid_manifest_never_reaches_egress` | <1 minute | PASS |
| C4 | Tenant and credentials are strict and secret-safe. | S3, S4 | Inject canary emails/tokens across every mode and capture process args, stdout, stderr, response temp names, and state; invalid origins accepted or any canary occurrence falsifies C4. | Direct byte search over captured channels plus fake-curl stdin/argv observation. | Add `set -x`; `credentials_never_escape_execution_boundary` fails on stderr canary. | `credentials_never_escape_execution_boundary` | <1 minute | PASS |
| C5 | Bootstrap converges and refuses collisions. | S7–S9, S14 | Run absent, matching, drifted-owned, foreign-collision, duplicate-marker, and partial-state cases; non-convergence, second-run writes, or foreign adoption falsifies C5. | Fake upstream's independent object store and request-count log. | Skip lookup before POST; `bootstrap_is_idempotent_and_collision_safe` fails on second-run POST count. | `bootstrap_is_idempotent_and_collision_safe` | <1 minute | PASS |
| C6 | State is minimal, serialized, symlink-safe, and atomic. | S2, S7, S12, S13 | Interrupt before commit, target a symlink, run concurrent operators, and inject mutable/secret fields; partial replacement, followed symlink, dual writer, wrong mode, or forbidden key/canary falsifies C6. | Filesystem metadata plus independent JSON allowlist comparison. | Replace temp+rename with direct write; `state_commit_is_atomic_and_minimal` observes truncated prior state. | `state_commit_is_atomic_and_minimal` | <1 minute | PASS |
| C7 | Verify checks both visibility graphs and exact authority. | S6, S8, S9, S11 | Feed wrong parent/ID/body, reader-visible private object, reader-hidden public object, and malformed pages; any success falsifies C7. | Manifest-derived expected graph computed in Rust and compared to independent actor-specific fake responses. | Run all verify reads with provisioner credentials; `verify_enforces_reader_visibility_boundary` succeeds incorrectly and fails the test. | `verify_enforces_reader_visibility_boundary` | <1 minute | PASS |
| C8 | Cleanup is owned, bounded, async-aware, and repeat-safe. | S7, S8, S10, S14, S15 | Exercise synchronous delete, 404, 202→running→complete, terminal failure, repeated status, timeout, partial state, foreign collision, and second cleanup; wrong deletion or false success falsifies C8. | Fake object store plus long-task state machine and request log. | Treat Confluence 202 as completion; `cleanup_waits_for_owned_space_deletion` fails because the space remains. | `cleanup_waits_for_owned_space_deletion` | <1 minute | PASS |
| C9 | Small fixtures force real pagination. | S6, S11, S15 | Return exactly two rows at page size one for each paging family; missing second request, reused continuation, reordered native query, or unbounded loop falsifies C9. | Fake request log and hand-authored two-row expected sequence. | Ignore Confluence `_links.next`; `verify_forces_each_pagination_family` fails on one observed row. | `verify_forces_each_pagination_family` | <1 minute | PASS |
| C10 | Failures are bounded, actionable, and secret-free. | S1–S4, S7–S11, S13, S15 | Inject every local/HTTP/transport/decode/authority failure with canaries and oversized prose; zero exit, raw prose, full URL, unbounded output, missing allowlisted context, or canary falsifies C10. | Rust capture applies an explicit diagnostic token/byte allowlist. | Echo the captured response body on HTTP 400; `failures_are_bounded_and_redacted` fails on raw-prose canary. | `failures_are_bounded_and_redacted` | <1 minute | PASS |
| C11 | Deterministic contract covers complete lifecycle. | S1–S15 | Run the complete fake lifecycle and require every named scenario/request transition; any uncovered branch or missing terminal state falsifies C11. | Scenario table in Rust compared against fake-curl request/state log, not shell internals. | Delete the second cleanup scenario; `operator_contract_covers_lifecycle_matrix` fails its required-scenario set. | `operator_contract_covers_lifecycle_matrix` | <2 minutes | PASS |
| C12 | Real disposable tenant completes full lifecycle. | S6–S11, S14, S15 | Against the approved disposable tenant, run bootstrap twice, verify, cleanup twice, and final provisioner/reader absence checks; any command failure, second-run write, visibility mismatch, residue, or secret output falsifies C12. | Direct Atlassian UI or separately authenticated read-only `curl`/`jq` queries using the provisioner and reader, not the operator's state. | Change v1 space-delete polling to one attempt; deterministic `cleanup_waits_for_owned_space_deletion` fails before the live gate. | `cleanup_waits_for_owned_space_deletion` and `operator_contract_covers_lifecycle_matrix` | Disposable tenant and two principals | PASS (2026-09-05, after live-contract repairs recorded in evidence.md) |

## Non-goals and future work

Permanent non-goals for this ticket:

- Principal creation, product licensing, and organization-wide account administration. The operator consumes an already disposable tenant and two independently provisioned principals; automating Atlassian organization administration would expand authority beyond fixture ownership.
- ResourceFS source configuration, Confluence adapter implementation, live read rows, OAuth, and ResourceFS mutation. This operator is deliberately an external oracle and must not depend on those paths.
- Attachments, rate-limit exhaustion, bulk mutation, Jira workflow schemes beyond the selected built-in business template, and destructive permanent live tests. None is required to prove the read fixture shapes, and each adds unrelated authority or tenant load.
- Repairing or deleting a reserved-key collision without the exact checked-in owner marker. Refusal preserves foreign tenant state.
- Concurrent reconciliation of one state path. The command serializes with an atomic lock and the second process fails explicitly rather than merging two operator runs.

Intended future work: none introduced by this design.

## Falsifier run log

- 2026-08-30 — `python .rfs-bcym/probe_openapi.py` — **PASS**. Current pinned vendor schemas expose every selected lifecycle, identity, storage, private-space, pagination, and long-task fact; `.rfs-bcym/evidence.md` records independent oracle agreement for P1–P5. C1 survived the cheapest falsifier.
- 2026-08-30 — C2–C11 named mutations — **PASS**. Each approved mutation made its named Rust regression fence red; every mutation was restored, then the complete ten-test contract passed.
- 2026-08-30 — `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract -- --nocapture` — **PASS** (10 passed). Deterministic lifecycle: bootstrap twice, exact provisioner/reader verify, cleanup twice, final empty store; pagination, collision, atomic state, async deletion, and redaction scenarios passed.
- 2026-09-05 — C12 live gate, first pass — **FAIL (contract repairs required)**. The disposable tenant exposed four production behaviors the fake did not imitate: Confluence strips leading HTML comments from stored page/comment bodies and injects `ac:schema-version`/`ac:macro-id` into macros; root pages report the space homepage as `parentId`; `GET /rest/api/3/project/search` omits `description` unless `expand=description`; and long-task terminal status is `FINISH_SUCCESS`. Ownership markers moved from body comments to v1 content properties (`rfs-owner`), the manifest pre-encodes entities, body comparison normalizes provider-injected macro attributes, cleanup delegates permission-denied issue deletes to the project cascade, and the fake plus deterministic suite were updated to fence every finding.
- 2026-09-05 — C12 live gate, second pass — **PASS**. Bootstrap twice (second converged with no error), verify (provisioner + reader authority, pagination families, reader-concealed boundary), cleanup twice (first surfaced the `FINISH_SUCCESS` poll vocabulary gap, fixed; second idempotent), and independent provisioner/reader absence checks returned zero fixture projects/spaces and HTTP 404 on the reader project probe. Deterministic suite 10/10 after the repairs.
- C12 is **PASS** as of 2026-09-05; the live-contract findings and repairs are recorded in `.rfs-bcym/evidence.md`.

### Review-fix falsifiers — 2026-09-06

F1/F2 strengthen C4's observation to every subprocess argv/environment and temporary file while execution is in progress, not merely curl's arguments and final outputs. F6 extends C2/C10 to an explicitly empty state path before all side effects. F7/F8 strengthen C11's independent oracle by rejecting a wrong request origin and checking both output streams.

F3 extends the C5/C6/C8 partial-run fixture with failed ownership publication before and after upstream commit, known-ID recovery, an unknown create outcome, and mismatched receipt authority. F4 directly checks saved stable IDs after project/space/title drift and requires conclusive absence before state removal. F5 uses the documented root and children collections separately, with two paginated replies, and asserts second-run write absence. The original implementations are the named mutations for these new regression fences; Slice 3/4 checkpoints own their red/green results.

Published current Confluence create schemas do not expose atomic content-property creation: [page create](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-post), [footer-comment create](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/#api-footer-comments-post), and [post-ID content properties](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-content-properties/). Recovery therefore uses a stable-ID receipt rather than an undocumented legacy create payload.

The 2026-09-05 C12 record is historical convergence/visibility/absence evidence only. Review-fix C12 must additionally capture actual live write requests and compare independent before/after object identities and versions; the corrected-run result belongs in `evidence.md`.

The corrected Jira lifecycle distinguishes live absence from permanent absence. Normal DELETE defaults to recyclable deletion ([JRACLOUD-94802](https://jira.atlassian.com/browse/JRACLOUD-94802)); owned replacement and cleanup explicitly request `enableUndo=false`. Cleanup searches deleted projects as well as live projects, revalidates hidden trash by stable ID and exact marker, and verifies absence from both visibility states. A direct GET 404 is not deletion authority for a hidden tombstone. The disposable-tenant probe confirmed direct permanent deletion of an exact owned tombstone; no undocumented restore fallback is introduced.

Jira readback identity/ownership is checked separately from replaceable name drift. Losing the owner marker, key, or ID is a foreign-collision failure, not permission to delete and recreate the object.

## Approval

- Requester words: “Approve design”
- Date: 2026-08-30
- Approved risk acceptances: None.
