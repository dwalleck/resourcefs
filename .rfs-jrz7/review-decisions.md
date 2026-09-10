# S1 review decisions

| finding-id | finding | reviewer | evidence-state | evidence | decision | fix | note |
|---|---|---|---|---|---|---|---|
| F1 | Public GithubAddress::parse bypasses the bounded PathReference parser; remove the unused alternate entry point | AddressingIncrementReview | Verified | Direct code check: github.rs:107 invokes parse_github_reference without validate_reference_input; the child accepts arbitrarily long unreserved path input. The existing public PathReference parser enforces MAX_PATH_REFERENCE_BYTES. LSP sibling-worktree query reports no references; independent grep finds no GithubAddress::parse calls or renamed imports in crates. | Accept | Remove the redundant public full-reference parser; retain PathReference as the full-reference boundary | No compatibility alias: this API is new, uncommitted and unused. |

## F1 atomic repair

- Ownership: C2; plan.md S1. Inherit grammar, canonicalization, budgets and module ownership unchanged.
- Root cause: a second public full-reference parser omitted the established whole-reference boundary. Remove GithubAddress::parse from core/src/reference/github.rs rather than duplicating validation or introducing a forwarding alias. No caller migration is necessary.
- Expected checks: core immutable-reference contracts retain exact65536 acceptance/65537 refusal and canonicalization; public-parser oracle remains39/39; workspace all-target compilation and placement census remain valid. Current full repository gate is allowed to finish on its unchanged snapshot before this edit.
- Evidence disposition: existing double-decode mutation remains applicable to the unchanged private segment decoder; decoded-control and placement mutations are unchanged. Parser/limit budget measurements exercise PathReference, not the removed method, and remain applicable. Refresh affected tests/compiler/placement after repair. Final S2/S3 integration, platform and live proof remain Main-owned and pending.
- Returned checkpoint result: PASS, all nine S1 gates reconciled in plan.md. F1 method removed. Post-repair full runner passed clippy, all-feature debug/release tests, all production budgets, placement and dependency vetting; its sole import-format failure was corrected and the exact formatting check passed. Other leg results remain valid after import-only formatting. qualification-s1.txt retains the exact disposition; no aggregate exit0 is fabricated.
