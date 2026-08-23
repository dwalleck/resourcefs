# Raw request: rfs-60g1

> start with rfs-60g1

Issue (rfs-60g1) — Use Session Scratch as independently writable state:

> Make local Session Scratch a complete independently authorized Resource family: agents can create, read, search, replace, hashline-edit, delete, and move local Resources without Workspace Mutation grants while retaining quotas, Version Tags, isolation, and lifecycle guarantees.

Acceptance criteria (verbatim):
- local Resources support the complete read/search/write/edit/delete/same-source-move workflow through the five public tools.
- Scratch mutation succeeds without Workspace Mutation permission but still enforces object/session quotas and Version Tags.
- Scratch names and backing paths cannot escape the Path Session sandbox, including through links or traversal.
- Scratch Resources are isolated across simultaneous Path Sessions and invalidated at disconnect.
- Quota failure is atomic and never evicts a live Resource.
- The Local Source Adapter has direct contract tests for every operation and lifecycle transition.
