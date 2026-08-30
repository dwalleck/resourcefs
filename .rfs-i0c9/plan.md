# Budgeted plan: rfs-i0c9

## Partition arithmetic

- Slice estimates: 850 changed lines (implementation 300, contracts/unit tests 500, dependency/configuration 20, artifact updates 30).
- Churn margin: 25% = 213 lines, because deterministic time/cancellation fixtures commonly need one revision after the first compile.
- Projected total: 1,063 changed lines.
- Review-size gate: PASS — 1,063 <= 4,000.
- PR increments: one.

### Increment: shared-bounded-read-retry

Contains Slice 1. Mergeable definition: the shared HTTP module owns the complete bounded-read retry state machine; GitHub and HTTPS callers compile against it; all retry, status, cancellation, deadline, mutation-safety, and existing adapter contracts pass without any later increment.

## Slice 1: Move all bounded idempotent-read retry behavior behind the shared HTTP interface

**Claim IDs:** C1, C2, C3, C4, C5, C6, C7

**Expected behavior:** A source-neutral logical read performs at most two GET attempts; valid delta-seconds or HTTP-date guidance on an initial 429/503 waits for that delay plus inclusive 0–250 ms jitter only when the total fits strictly before the immutable logical deadline; unusable guidance is terminal without waiting; cancellation wins during the wait; other statuses and mutation methods never replay; conclusively retryable transport failures retain one immediate follow-up; GitHub pagination cannot refresh its deadline.

**Oracle:** TLS listener request counts and captured methods; fixed RFC HTTP-date epochs computed independently of the parser; explicit integer/duration arithmetic for jitter and deadline admission; Tokio paused/outer timeout observations; stable `ErrorCategory` values at the Source interface.

**Stress fixture:** A matrix containing delta 0 and `u64::MAX`, fixed future/equal/past HTTP dates, malformed/empty/negative/comma-combined/non-ASCII values, deadline relations just below/equal/above delay+jitter, all eight sampled bytes in 251–255, cancellation during a 60-second advised wait, 429/503 GETs, a non-retry status, and POST/PATCH retry-looking responses. Expected: only valid strictly fitting GET 429/503 rows make exactly one follow-up; all others make one attempt and return their specified stable outcome.

**Regression fence:** `crates/resourcefs-sources/src/http/mod.rs` unit contracts; `crates/resourcefs-sources/tests/http_substrate_contract.rs`; `crates/resourcefs-sources/tests/https_status_contract.rs`; existing `crates/resourcefs-sources/tests/github_adapter_contract.rs`; existing `crates/resourcefs-sources/tests/github_mutation_contract.rs`.

**Named mutation:** Apply each approved design mutation at the slice checkpoint: remove 503/date handling (C1); map invalid guidance to zero or admit deadline equality (C2); replace bounded rejection with modulo, omit jitter from admission, or default entropy failure (C3); replace cancellation select with bare sleep (C4); remove GET/status eligibility (C5); drop transport retry/reset the attempt counter (C6); construct deadline per fetch (C7). Each must turn its named focused fence red, then restoration must return it green.

**Complexity/production scale:** The attempt loop is capped at two iterations, so O(1) attempts. Retry-After parsing is O(H) for one retained header with H <= 16 KiB. Jitter consumes one fixed eight-byte entropy sample and at most eight comparisons, O(1); no unbounded rejection loop. Request/header cloning is permitted only for the bounded GET retry copy (<=16 headers and 16 KiB), never a mutation body. Production cost is at most one extra bounded HTTP attempt and one guided wait plus <=250 ms jitter, all inside the pre-existing configured logical timeout (maximum 30 seconds). Maximum accepted cost: two attempts, 16 KiB duplicated request metadata, eight jitter-byte inspections, and total wall time <= the original logical timeout; any excess fails the slice.

**Wall budget/phase:** Always-on read phase. No-guidance responses add at most one method/status branch and no entropy call; retry-guidance responses add one bounded parse and entropy sample. Accepted wall budget: every logical read, including wait and retry, completes or fails by its original configured deadline (<=30 seconds), and deadline-refusal/cancellation fixtures complete before an advised 60-second wait. Rationale: the ticket may consume remaining deadline but may never extend the caller's existing latency ceiling.

**Files:** `Cargo.toml`, `Cargo.lock`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/http/mod.rs`, `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-sources/tests/http_substrate_contract.rs`, `crates/resourcefs-sources/tests/https_status_contract.rs`, `crates/resourcefs-sources/tests/github_adapter_contract.rs`, `.rfs-i0c9/design.md`, `.rfs-i0c9/plan.md`, `.rfs-i0c9/route.md`, `.rivets/issues.jsonl`.

**Estimate:** 3–5 hours.

**Diff estimate:** 850 changed lines.

**PR increment:** shared-bounded-read-retry

**Commands and expected results:**

- `cargo test -p resourcefs-sources --lib http::tests` → fixed date arithmetic, unusable guidance, inclusive jitter bounds/fail-closed sampling, strict deadline equality, and immutable-deadline checks agree item-by-item with explicit arithmetic; under each C2/C3/C7 named mutation its labelled row goes red, then returns green after restoration.
- `cargo test -p resourcefs-sources --test http_substrate_contract` → source-neutral GET fixtures make exactly two attempts only for valid 429/503 delta/date rows; wait cancellation returns `cancelled` with one request; GET/non-GET eligibility matches listener counts; under C1/C4/C5 mutations the named row goes red, then green after restoration.
- `cargo test -p resourcefs-sources --test https_status_contract` → malformed/missing/negative/past/overflow/ambiguous/deadline-non-fitting guidance makes one request and returns `source_unavailable` without the forbidden wait; C2 mutations turn the named row red, restoration returns it green.
- `cargo test -p resourcefs-sources --test github_adapter_contract` → existing rate and transport positive controls remain exactly two attempts, deadline refusal remains one prompt attempt, and HTTP-date/cancellation rows meet the same source-neutral policy; C6's mutation turns its existing fence red, restoration returns it green.
- `cargo test -p resourcefs-sources --test github_mutation_contract` → PATCH/POST redirect, unknown-outcome, and retry-looking responses retain exact one-transmission behavior.
- `cargo fmt --all -- --check` → all touched Rust and Cargo-adjacent source is formatted.
- `cargo clippy -p resourcefs-sources --all-targets -- -D warnings` → no warnings, including error suppression, needless allocation, or retry-state lint findings.

## Tracker taxonomy

- No implementation, test, or documentation work is deferred.
- Permanent non-goals remain those approved in `design.md`: more than one follow-up, mutation replay, and statuses other than 429/503. No tracker issue applies because those behaviors are deliberately excluded from this contract.
- Intended future work: none.

## Self-review

- [x] C1–C7 are each assigned exactly once, to Slice 1; every PENDING falsifier is discharged there.
- [x] All thirteen mandatory slice fields are populated.
- [x] Every claim's deterministic fence and named mutation are in the owning slice; no risk acceptance is used.
- [x] The capped attempt loop, fixed jitter sampling, header bound, production cost, and maximum accepted cost are explicit.
- [x] The always-on phase retains the original <=30-second wall ceiling.
- [x] 850 + 213 = 1,063 changed lines; one increment is below the 4,000-line review-size gate.
- [x] Tracker taxonomy is applied; no future-work deferral exists.
- [x] This plan declares no slice complete; checkpointed-build owns completion.
