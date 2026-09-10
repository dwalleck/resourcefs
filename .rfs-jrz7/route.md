# Route: rfs-jrz7

Change: Read immutable commit metadata and exact commit/path GitHub Fact Resources.
Date: 2026-09-09

Source state: `635ab170ab57542c18272921298d575da2f8b08a`; isolated branch `feat/rfs-jrz7`. Request: “claim and implement rfs-jrz7”. Behavioral authority: full rfs-jrz7 description and sibling `resourcefs-wt-github-resource-contract/docs/github-resource-contract-spec.md`, stories 9–11, 17–21, 38–42 and applicable shared requirements. rfs-0n97 landed via PR #9; tracker status is stale, not evidence that the prerequisite is absent. No authorization to close unrelated tickets or publish a stack is inferred.

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | P1: selected REST 2022-11-28 commit and non-recursive Git tree/blob APIs can acquire a pinned nested regular file with matching commit, tree/parent/object identities, modes, sizes and exact bytes. Compare current pinned repository objects against local Git plumbing, not another REST decoder. Retained synthetic probes do not cover this provider seam. P2 correction: future github:// escaping/selector behavior is a feature claim, not an empirical premise; the scheme does not exist in current ResourceAddress/PathReference::parse (reference.rs:681–691,916–1027). Its exact spelling and refusal proof belong to design/build; do not pretend existing PR whole-body percent decoding implements the proposed segment rules. | yes |
| 2 | Structural module shape | Core currently owns validated Path References/GitHub addressing/canonical resource identity; new github:// commit/source variants extend that public address sum. Sources GitHub owns HTTP acquisition and projection and will own pinned object resolution; existing shared HTTP policy/cache/budget ownership is unchanged. MCP owns configured source/catalog/profile/read integration and must expose the new grammar through existing tools. Candidate owners remain those layers, with dedicated GitHub object/address family modules instead of accumulating object traversal in shared parent facades. SourceAdapter and SourceResource remain the public seam; no new provider trait, second HTTP client, core Git object decoding or Cyril client. | yes |
| 3 | Production-scale risk | Object traversal competes for 10 attempts/30 seconds; blobs can approach 8 MiB responses, 16 MiB cumulative accepted bodies/serialized JSON and 4 MiB decoded bytes. Independent decoded bound, path depth/tree width, deadline and allocation measurements require implementation gates, not inherited synthetic evidence. | yes |
| 4 | Explicit behavior | Complete observable given/when/then contract below. Proposed exact spellings are design decisions, not missing behavioral policy. | yes |

### T4 observable contract

1. Given an explicitly authorized repository and full immutable commit operand, when commit facts are read, then retain requested/observed commit SHA, tree and parent SHAs/links, message, native author/committer identities/times and supplied GitHub accounts; never substitute a branch tip or synthetic merge.
2. Given a commit/path, when source facts are acquired, then validate repository/commit/tree/path/mode/blob relationships before accepting content. Regular binary, non-UTF-8 and newline-sensitive bytes round-trip exactly through base64, including executable mode.
3. Given decoded sizes of 4 MiB and 4 MiB+1, when read under hard limits, then independently enforce decoded size despite equal base64 lengths; oversized facts retain verified identity/size with no accepted partial bytes or invented recovery. Acquisition itself remains bounded; no chunked blob acquisition.
4. Given symlink, submodule or committed LFS pointer objects, when read, then preserve Git object/mode/target identity without following links, traversing submodules or hydrating LFS. Non-regular metadata is not regular-file content.
5. Given representable nested Unicode paths, literal percent/colon/question/hash/space and filename facts, when canonicalized/read, then round-trip identity and decode once; malformed escapes, traversal aliases, encoded separators and provider-unrepresentable paths are refused rather than normalized into another identity or silently omitted.
6. Given denied/inaccessible/deleted forks or unknown immutable objects, when acquisition is attempted, then require explicit repository authority and produce the appropriate bounded typed failure; no other-repository fallback or moving ref substitution.
7. Given configured source/profile/catalog/public-library/real MCP access, when either new Resource is read, then return owned versioned complete bounded facts JSON through existing SourceAdapter/SourceResource and existing five tools; integrate grammar/examples and applicable profile/CLI schema/check paths. Existing human reads retain their meaning.
8. Given successful, failed, retried or cancelled acquisition, when requests are independently observed, then methods/operands/identities match the authorized immutable request with zero forbidden egress and zero remote writes. Charge parent lookups/retries/revalidation to one attempts/deadline ledger. Preserve bounded sanitized error status/access/retry/limit facts and existing retry/cache policy; failed revalidation cannot return stale success.
9. Given valid lower-only acquisition controls, when requested, then intersect them with operator policy separately from display limits; zero and above-hard-ceiling inputs fail. Enforce 8 MiB per response, 16 MiB accepted-body total, 16 MiB serialized JSON including outcome/provenance, 4 MiB decoded content, 1,000 records where applicable. Measure actual implementation deadline/transport/allocation behavior; none of these is a peak-memory/wire-traffic promise.
10. Given applicable multi-page components, when later ordinary acquisition fails, then retain only verified whole admitted pages with honest coverage and valid continuation only when available; first failure without usable facts is an error. Cancellation and authority/requested-identity failures reject the current component. Do not treat internal traversal fragments as complete source facts or fabricate collection continuation.
11. Given library consumption, when facts return, then the full bounded document is parseable; given smaller MCP display limits, then actual Recovery References reconstruct exactly the acquired JSON before parsing. Source continuation is separately upstream acquisition under ADR-0006.
12. Given applicable public read-only live shapes and explicit live gates, when adapter and real stdio live_* smokes run, then check upstream invariants and record exact revision/results. Missing RFS_LIVE=1/GITHUB_TOKEN skips cleanly but is not PASS. Final implementation runs the repository-owned complete gate; live proof is separate and does not close Cyril corporate/native/credential/storage/UI acceptance.
13. Given any successful facts, then preserve configured source/deployment/native repository/object identity and links, acquisition interval/REST version separately from supplied upstream versions/times, absent/null/unknown knowledge and explicit coverage. VersionTag hashes representation bytes, not Git identity. Reuse slice 1's reviewed envelope/errors/controls, permitting additive optional schema fields and rejecting unsupported major versions.

Unknown tests: none.

## Selected route

Empirical — unverified existing provider/parser premises take precedence over the public address/schema and bounded acquisition changes.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit in T4 and approved ticket; spelling/placement remains design work |
| evidence.md, probe.* | prove-it-prototype | required — P1/P2 |
| design.md | falsifiable-design | required — after empirical hand-off |
| plan.md | budgeted-plan | required — after design approval |

Oracle checkpoint in checkpointed-build: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Every empirical premise has PASS evidence; every downstream artifact satisfies its owning completion criterion, ending with no FAIL in checkpointed-build. Implementation, numerical measurement, complete repository gate and applicable adapter/real-stdio live proof remain required. Production design must receive explicit requester approval before implementation. Current status: empirical stage, not implemented.
