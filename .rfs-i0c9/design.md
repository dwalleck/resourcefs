# Falsifiable design: rfs-i0c9

## Route and inputs

- Route: **Structural**, from `.rfs-i0c9/route.md`.
- Behavior source: `route.md` T4. `spec.md`: N/A — the ticket and T4 contain the complete behavior set, and the requester explicitly directed that interrogation be skipped.
- Empirical inputs: N/A — T1 found no external or existing-system premise; no `evidence.md` or probe applies.
- Behavior set: eligible idempotent GETs get at most one retry for valid delta-seconds or non-past HTTP-date guidance; the wait adds 0–250 ms jitter and both wait and second attempt remain inside the original logical deadline; invalid, absent, negative/past, or non-fitting guidance does not wait or retry and reaches the owning Source's stable `source_unavailable` classification; cancellation during the wait returns `cancelled` promptly; non-retryable responses and non-idempotent requests remain single-attempt; GitHub's existing one transport retry remains observable.

## Input shapes

| Input | Production-reachable shapes | Status |
|---|---|---|
| HTTP method | GET; POST; PATCH | Covered by C1, C5, C6 |
| Attempt | first; one permitted follow-up; response/failure after the follow-up | Covered by C1, C5, C6 |
| Response status | 429; 503; every other status | Covered by C1, C2, C5 |
| Retry-After presence | absent; one present value | Covered by C1, C2 |
| Retry-After text | unreadable/non-ASCII; empty; ASCII delta-seconds; ASCII HTTP-date; other ASCII | Covered by C1, C2 |
| Delta-seconds | zero; positive; negative spelling; overflowing integer | Covered by C1, C2 |
| HTTP-date relation | future; equal at parser resolution; past; syntactically invalid | Covered by C1, C2 |
| Duplicate/comma-combined Retry-After | multiple field values represented by the client as one non-standard value | Covered by C2 — ambiguous guidance is invalid and never guessed |
| Jitter | 0 ms; interior; 250 ms; rejected random bytes 251–255; entropy-source failure | Covered by C3 |
| Deadline relation | wait plus jitter strictly below remaining; equal; greater; already elapsed | Covered by C2, C3, C7 |
| Cancellation | before egress; during request/body; during retry wait | Existing substrate contracts cover the first two; C4 covers the new wait branch |
| Transport result | first attempt succeeds; first attempt conclusively retryable; follow-up fails | Covered by C6 |
| Logical read extent | one fetch; multiple GitHub page fetches | Covered by C7 |
| Request collection sizes | one request header set; bounded zero/single/multiple headers | N/A — retry reuses the already-validated `HttpRequest`; header collection semantics and ceilings do not change |
| Request URL strings | all validated absolute URLs accepted by `HttpRequest` | N/A — URL parsing, Unicode, spaces, and origin authorization do not change |

## Removed-invariant sweep

The move removes GitHub's private retry loop as the enforcement point. That loop silently guaranteed at-most-one retry, used the Source operation's single deadline, and kept mutation requests out by existing call placement. C1/C5/C6/C7 preserve those facts in the shared owner. Request authorization, body ceilings, cache atomicity, and mutation unknown-outcome handling remain behind the same attempt function and are still safe because the shared retry layer calls that function rather than bypassing it.

## Placement

### Bounded logical read and retry execution

- **Owner:** `crates/resourcefs-sources/src/http/mod.rs`. It already owns `HttpRequest`, request method, `HttpFetchFailure`, Retry-After response metadata, cancellation-aware egress, and HTTP ceilings; placing the state machine here gives GitHub, HTTPS, and later Atlassian adapters leverage through one interface.
- **New seam:** no new cross-crate seam. Add a crate-private typed logical-read value created by `HttpSubstrate` that binds an `OperationGuard` to one immutable deadline and performs source-neutral bounded GETs. Existing public `HttpSubstrate::fetch` remains the single-fetch convenience and delegates through the same implementation; GitHub holds one logical-read value across all page fetches. A rejected alternative is an adapter callback/policy trait: statuses and Retry-After are HTTP semantics shared by every adapter, so a callback would make callers restate policy and produce a shallow module. A second rejected alternative is adding deadline/jitter parameters to every `fetch` call: that exposes implementation details and permits callers to extend deadlines.
- **Forbidden:** adapters may not parse Retry-After, generate jitter, sleep, or own attempt counters; no Atlassian-specific retry path; retries may not call mutation requests; retry may not create a fresh deadline.

### Retry-After parsing and jitter

- **Owner:** private typed sums and helpers inside the HTTP module. Retry-After is parsed once at the untrusted header boundary into usable delay versus unusable guidance; jitter uses rejection sampling to produce an inclusive uniform millisecond value without modulo bias.
- **New seam:** no production seam. Pure private helpers accept explicit wall time/random-byte input so unit contracts can deterministically exercise date arithmetic, bounds, rejection, and entropy failure without making clocks or randomness part of the Source interface.
- **Forbidden:** callers may not re-parse retained header strings; malformed guidance may not become zero; entropy failure may not silently disable jitter; HTTP-date parsing may not use body prose or a Source-specific clock.

Mechanical placement falsifier: the source-neutral substrate contract drives retry through `HttpSubstrate` without constructing GitHub/HTTPS types, while the GitHub positive control proves its adapter consumes the same implementation. Moving behavior back into an adapter makes the substrate contract fail.

## Claims

- **C1:** A first 429 or 503 GET response with valid delta-seconds or non-past HTTP-date guidance waits for guidance plus 0–250 ms jitter and sends exactly one follow-up.
- **C2:** Missing, unreadable, malformed, negative/past, overflowing, ambiguous, or deadline-non-fitting guidance sends no follow-up and reaches the owning Source as `source_unavailable` without consuming the forbidden wait.
- **C3:** Jitter is always an inclusive 0–250 ms value, is included in deadline admission, and entropy failure returns `source_unavailable` rather than silently weakening policy.
- **C4:** Cancellation during the retry wait wins promptly, returns `cancelled`, and prevents the follow-up request.
- **C5:** Non-429/503 responses and POST/PATCH requests remain single-attempt regardless of Retry-After.
- **C6:** A conclusively retryable GET transport failure gets exactly one immediate follow-up, while a follow-up failure is terminal; GitHub retains its current observable behavior.
- **C7:** Every fetch and retry in a multi-fetch Source read uses the immutable deadline created at the start of that logical read.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Valid delta/date guidance retries exactly once. | GET × {429,503} × {delta 0/positive, equal/future HTTP-date} × first/follow-up | Serve each first response with valid guidance then 200; any request count other than 2, wrong body, or a third request falsifies C1. | Fixture listener request count plus a fixed RFC HTTP-date epoch computed independently from the production parser. | In `http/mod.rs`, remove the 503 arm or HTTP-date parse branch; `http_substrate_contract::idempotent_reads_honor_delta_and_http_date_once` goes red for its labelled row. | `http_substrate_contract::idempotent_reads_honor_delta_and_http_date_once` | <1 min | PENDING — checkpointed-build, retry-policy slice |
| C2 | Unusable/non-fitting guidance never waits or retries and yields stable failure. | Missing/unreadable/empty/negative/past/overflow/combined guidance; deadline equal/greater/elapsed | Table-drive HTTPS reads returning 429/503; a second listener request, consumption of the forbidden delay, or category other than `source_unavailable` falsifies C2. | Listener count, a deadline shorter than the advised wait, and `ErrorCategory` from the Source interface are independent of header parsing. | In `http/mod.rs`, map parse failure to zero or change strict deadline admission to allow equality; `https_status_contract::unusable_retry_guidance_is_terminal_without_wait` goes red. | `https_status_contract::unusable_retry_guidance_is_terminal_without_wait` | <1 min | PENDING — checkpointed-build, retry-policy slice |
| C3 | Jitter is closed-bound, deadline-accounted, and fail-closed. | 0/interior/250; rejected 251–255 bytes; entropy failure; deadline below/equal/above total delay | Feed deterministic bytes and remaining durations into the private helpers; output outside 0..=250, acceptance when total is >= remaining, modulo use, or success on entropy failure falsifies C3. | Integer range/arithmetic assertions over supplied bytes and durations, independent of production entropy and wall clock. | In `http/mod.rs`, replace rejection sampling with `% 251`, omit jitter from admission, or default an entropy error to zero; `http::tests::retry_jitter_is_bounded_unbiased_and_deadline_checked` goes red. | `http::tests::retry_jitter_is_bounded_unbiased_and_deadline_checked` | seconds | PENDING — checkpointed-build, retry-policy slice |
| C4 | Wait cancellation is prompt and sends no follow-up. | cancellation during positive retry wait | Start a long guided wait, observe request one, cancel its guard, and bound completion; a timeout, non-`cancelled` category, or request two falsifies C4. | Guard cancellation plus listener count and an outer short timeout use different failure mechanisms from the select branch. | In `http/mod.rs`, replace the cancellation-aware select with bare sleep; `http_substrate_contract::retry_wait_is_promptly_cancelled` times out or observes request two. | `http_substrate_contract::retry_wait_is_promptly_cancelled` | <1 min | PENDING — checkpointed-build, retry-policy slice |
| C5 | Ineligible status/method shapes remain single-attempt. | GET other status; POST/PATCH with 429/503 and valid guidance | Serve retry-looking responses to each ineligible shape; any listener count above 1 falsifies C5. | Listener request count and method-specific fixture routes are independent of retry eligibility code. | In `http/mod.rs`, remove the GET/status eligibility guard; `http_substrate_contract::retry_is_limited_to_idempotent_429_and_503_reads` observes a second request. | `http_substrate_contract::retry_is_limited_to_idempotent_429_and_503_reads` | <1 min | PENDING — checkpointed-build, retry-policy slice |
| C6 | Eligible transport retry remains exactly one. | first transport failure; successful or failed follow-up | Run the existing GitHub transport and rate positive controls; fewer/more than 2 attempts or changed success/failure category falsifies C6. | TLS fixture abort count is independent of `HttpFetchFailure::is_retryable`. | In `http/mod.rs`, drop transport retry or reset the attempt counter after response retry; `github_adapter_contract::retry_is_single_and_machine_signaled` goes red. | `github_adapter_contract::retry_is_single_and_machine_signaled` | <1 min | PASS |
| C7 | One logical read keeps one immutable deadline across fetches. | single/multiple fetches; remaining below/equal/above wait+jitter | Serve a paginated GitHub read whose first page consumes one guided wait and whose second page advises another wait that fits only a refreshed deadline; a fourth request, success, or generic outer timeout falsifies C7. | Listener request count and a Source error required to name the refused `Retry-After` compare independently against the production deadline field. | In `http/mod.rs`, refresh `BoundedRead`'s deadline inside each fetch; `github_adapter_contract::pagination_retries_share_the_original_logical_deadline` goes red. | `github_adapter_contract::pagination_retries_share_the_original_logical_deadline` | <1 min | PENDING — checkpointed-build, retry-policy slice |

## Non-goals and future work

- Permanent non-goal: more than one retry per logical request. The bounded one-follow-up rule is the requested safety contract, not an initial backoff strategy.
- Permanent non-goal: retrying POST or PATCH. Unknown mutation outcomes must remain visible and must never be replayed by read policy.
- Permanent non-goal: retry statuses other than 429 and 503. The ticket names machine-signaled rate/service throttling, not a general status retry matrix.
- Intended future work: none.

## Falsifier run log

- 2026-08-29 — `cargo test -p resourcefs-sources --test github_adapter_contract retry_is_single_and_machine_signaled -- --exact` — **PASS**: 1 passed; the current GitHub delta-seconds and transport positive controls each made exactly two attempts.

## Approval

Requester approval: "Approve revised fences"

Date: 2026-08-29

Risk acceptances: None.
