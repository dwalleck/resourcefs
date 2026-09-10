# Operating ResourceFS

ResourceFS is a local stdio MCP server. Standard output is reserved for MCP frames. Diagnostics go to standard error or the configured log file.

## Launch authority

Choose exactly one launch form:

```sh
resourcefs serve --config ./resourcefs.json
```

or:

```sh
resourcefs serve \
  --root repo=/absolute/path/to/repo \
  --root notes=/absolute/path/to/notes \
  --primary-root repo
```

`--config` cannot be combined with `--root` or `--primary-root`. ResourceFS never grants the process working directory as an implicit Workspace Root. A Server Profile may intentionally contain no Workspace Roots; that creates a scratch-only session until a client supplies MCP Roots.

When a client advertises MCP Roots, its non-empty root set replaces the launch roots for that connection. If the client later reports an empty root set, ResourceFS restores the launch roots. Relative Path References work only when one Primary Workspace Root is selected.

## Server Profile

Version 1 profiles are strict JSON. Unknown fields, explicit `null`, duplicate source claims, unsafe paths, invalid grants, and values above hard ceilings fail before the server starts.

Minimal workspace profile:

```json
{
  "schemaVersion": 1,
  "workspace": {
    "roots": [
      {
        "id": "repo",
        "path": "../repo",
        "grants": {
          "create": false,
          "update": false,
          "delete": false
        }
      }
    ],
    "primaryRoot": "repo",
    "backingPathVisibility": "hidden"
  },
  "limits": {
    "text": {
      "bytes": 49152,
      "lines": 3000,
      "columns": 512
    },
    "discovery": {
      "searchMatches": 1000,
      "globEntries": 1000,
      "listingEntries": 1000
    },
    "storage": {
      "objectBytes": 67108864,
      "sessionBytes": 268435456
    },
    "processConcurrency": 32
  },
  "session": {
    "cacheDirectory": ".resourcefs-cache",
    "retentionTtlSeconds": 86400
  },
  "logging": {
    "level": "info",
    "destination": {
      "kind": "stderr"
    }
  },
  "sources": []
}
```

Relative Workspace Root paths, session cache paths, log paths, manifests, converter programs, and child-server programs resolve from the profile directory. Symlink-aware containment checks reject paths that escape a declared containing directory.

Generate the authoritative schema rather than copying profile fields from this guide:

```sh
resourcefs schema > resourcefs-server-profile-v1.schema.json
```

The schema command emits deterministic JSON with an object root and a trailing newline.

## Check before serving

Static validation performs no network I/O and does not invoke helper commands:

```sh
resourcefs check --config ./resourcefs.json
```

Add `--probe` to perform one bounded, non-mutating availability probe per configured source:

```sh
resourcefs check --config ./resourcefs.json --probe
```

Probe reports are JSON on standard output. Diagnostics remain off standard output.

A missing or invalid profile exits with status 2. A required source that is unavailable exits with status 3. Internal startup or runtime failures exit with status 1. Successful checks and clean server shutdowns exit with status 0.

Optional sources whose compiled adapters become unavailable at runtime become Degraded Sources: the server remains active, diagnostics name the source, and references owned by that source fail without affecting other sources. A profile that requests a source kind not compiled into the installed ResourceFS binary exits with status 3 before stdio starts, regardless of the source's `required` value.

## Credentials and child processes

Profiles reference credentials; they do not embed them. Use an environment reference:

```json
{
  "kind": "environment",
  "name": "RESOURCEFS_GITHUB_TOKEN"
}
```

or a direct-argument helper command:

```json
{
  "kind": "command",
  "command": {
    "argv": ["/usr/local/bin/read-resourcefs-token"],
    "environment": {
      "HOME": { "kind": "inherit", "name": "HOME" }
    }
  }
}
```

ResourceFS does not invoke commands through a shell. Child environments are cleared and rebuilt from the command's explicit `environment` map. Helper execution has bounded time, output, arguments, environment size, and global concurrency. Resolved credentials are redacted before any diagnostic or log write.

## GitHub PR Facts

Add a GitHub source to the profile's `sources` array. For example, this source entry grants read access to one repository on a custom deployment; it grants no mutations:

```json
{
  "kind": "github",
  "id": "enterprise",
  "required": true,
  "apiBaseUrl": "https://git.example.com/api/v3/",
  "webOrigin": "https://git.example.com",
  "allowPrivateNetwork": false,
  "credential": {
    "kind": "environment",
    "name": "RESOURCEFS_GITHUB_TOKEN"
  },
  "repositories": [
    { "name": "owner/repo" }
  ],
  "acquisition": {
    "maxAttempts": 2,
    "timeoutMs": 10000,
    "maxResponseBytes": 1048576,
    "maxAcceptedBodyBytes": 2097152,
    "maxRepresentationBytes": 4194304
  }
}
```

Replace the example hosts/repository and provide the referenced credential. A private-address deployment additionally needs `allowPrivateNetwork: true`; `webOrigin` is identity configuration, not a network grant. It must be an HTTPS origin without a path, query, fragment, or credentials; an optional trailing slash is accepted and normalized away. Custom API bases may retain their API path and port.

For public GitHub, omit `apiBaseUrl` and `webOrigin`: they default to `https://api.github.com/` and `https://github.com`. For Enterprise Cloud, `apiBaseUrl: "https://api.<tenant>.ghe.com/"` derives `https://<tenant>.ghe.com`. Those documented API bases accept no custom path or explicit port, and any explicit `webOrigin` must match the derived deployment. Custom API bases without `webOrigin` still support existing GitHub reads, but PR Facts fail with `unsupported_projection` and reason `deployment_identity_unavailable`; ResourceFS never guesses the web origin.

Call `rfs_read` with a PR Facts path and optional per-call lower acquisition limits:

```json
{
  "path": "pr://owner/repo/123/facts",
  "acquisition": {
    "maxAttempts": 1,
    "timeoutMs": 5000
  },
  "limits": {
    "bytes": 16384
  }
}
```

The example is a tool input, not a profile fragment. Output `limits` control the rendered result, not upstream acquisition. Source and per-call acquisition policies intersect dimension by dimension; a call cannot raise the source policy. Omitted dimensions use the hard defaults before intersection: 10 attempts, 30000 ms, 8388608 response bytes, 16777216 accepted body bytes, and 16777216 representation bytes. Applicable HTTP substrate ceilings may lower them further. Every supplied value must be a positive integer at or below its hard ceiling. Unknown/duplicate keys, nulls and non-object controls are rejected. An empty `acquisition: {}` still explicitly requests support. Singular PR Facts, conversation-comment collection Facts and singular conversation-comment Facts support these controls; non-facts Resources reject them rather than ignoring them.

Facts return `application/json; charset=utf-8` in the normal read-result envelope. The inner JSON schema is `{"major": 1, "minor": 0}` under `schemaVersion`, with `kind: "github.pull_request"`. Native numeric IDs and PR numbers are decimal strings, not JSON numbers. Optional nulls, omitted fields, empty strings and false values remain distinct. Both branches have validated commit SHAs; missing or null repository metadata is reported via `repositoryAvailability`, not filled with a guessed repository. See the [complete Facts contract](../DESIGN.md#pr-facts-version-1) for fields and provenance.

Read singular PR Facts without a selector (`:raw` is also unsupported). This route does not acquire issues, a repository PR listing, comments, reviews or diffs; conversation-comment Facts have their own routes below. It performs no mutation, automatic link follow, head-repository fetch, ref resolution, or commit comparison. Returned URLs and fork metadata do not grant authority to fetch those destinations.

Facts acquisitions refuse HTTP redirects, unlike the sibling human-readable `pr://` and `issue://` reads, which follow them: redirect hops cannot escape the shared attempt budget. For singular PR Facts the identity check also compares the object's own `url` against the requested endpoint. A renamed or moved repository therefore fails the Facts read with the redirect's `httpStatus` in `details` rather than silently resolving; re-read the canonical reference.

One logical deadline covers the acquisition, retry waits, and final acceptance, with at most one retry within the attempt budget. Cache revalidation retains original body provenance separately from the 304 observation, and reused body bytes still count toward admission limits. Acquisition overflow or cancellation never returns partial Facts JSON. A complete representation that exceeds the separate text output limit uses normal lossless artifact recovery; follow the returned artifact selectors rather than appending a selector to `/facts`.

Failures retain the ordinary error category and may include bounded structured `details`. In particular, a 404 has reason `upstream_not_found_or_hidden` and `accessAmbiguity: "missing_or_access_hidden"`: it is not proof that a private PR does not exist. Rate-limit errors may carry numeric retry guidance/reset time; limit errors may name the effective bound and observed value. Provider error prose, response bodies and arbitrary headers are not exposed in these details. Inspect the category/reason rather than matching human-readable messages.

## GitHub immutable commit Facts

The same GitHub source profile and readable repository allowlist also serve exact commit metadata; no additional source kind or MCP tool is needed:

```json
{
  "path": "github://owner/repo/commits/0123456789abcdef0123456789abcdef01234567/facts",
  "acquisition": { "maxAttempts": 2 }
}
```

Replace the sample ID with a real full lowercase commit SHA from the allowed repository. Branches, tags, abbreviated IDs and selectors such as `:raw` are rejected. Two attempts cover repository and commit acquisition without retries; use the default attempt budget if retries must remain possible. Enterprise API prefixes such as `/api/v3/` are preserved.

The inner schema is version 1.0, `kind: "github.commit"`. Requested and observed commit IDs are separate; `data` preserves the provider's message, native Git author/committer, ordered parents and optional/null GitHub accounts. Repository and commit responses have separate `upstream` observations. This is neither a commit diff nor raw Git object bytes. The ResourceFS Version Tag includes acquisition provenance and is not the native commit SHA.

Use the normal artifact recovery references when the complete JSON exceeds display limits; recovery does not fetch GitHub again. The `github://` catalog entry advertises both commit and exact source Facts.

## GitHub immutable source Facts

Use the same readable repository allowlist to acquire one exact path at a full lowercase commit SHA:

```json
{
  "path": "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/docs/guide.txt/facts",
  "acquisition": { "maxDecodedBytes": 1048576 }
}
```

Encode each path component independently: `docs/λ space%2F:raw.bin` becomes `docs/%CE%BB%20space%252F%3Araw.bin`. Literal encoded-looking text is not decoded twice. Empty components, dot segments, absolute paths and encoded separators are rejected; selectors are not supported. A terminal directory, symlink or submodule returns verified metadata with `content.state: "unsupported"`; ResourceFS never follows it.

The inner schema is version 1.0, `kind: "github.source"`. `data.content.state: "available"` provides `encoding: "base64"`, `decodedSizeBytes` and `bytesBase64`. Decoding that string yields exact blob bytes, including binary content; it is not a text projection. Requested and observed commit/path identities are separate. Every traversed non-recursive tree and accepted blob is checked against its native Git SHA. The Version Tag instead names the complete Facts JSON, including acquisition provenance.

`maxDecodedBytes` lowers the decoded-content ceiling (4 MiB by default), not the HTTP-body or final-JSON budgets. Repository, commit, every traversed tree and any blob request share one acquisition budget; deeper paths may require a larger lower-only attempt allowance within the hard ceiling. A decoded-size limit or ordinary blob acquisition failure can return verified metadata with `content.state: "unavailable"` and a bounded reason, without stale bytes. Identity/integrity failures, cancellation, deadline expiration and cache-generation changes refuse the whole read. Intermediate traversal failure cannot return a verified terminal object.

Display overflow uses the normal artifact recovery references for the complete JSON. Reading or paging those references never reacquires GitHub. Do not confuse the absence of displayable bytes with `content.state`; inspect the recovered JSON when necessary.

## GitHub conversation-comment facts

`pr://owner/repo/<number>/comments/facts` returns the conversation-comment collection and `pr://owner/repo/<number>/comments/<id>/facts` returns one comment, as owned read-only JSON with the shared envelope and `acquisition` controls. The collection schema is version 1.1; singular comment Facts remain version 1.0. The parent pull request is read and verified first. A singular read also verifies the returned comment ID against the addressed ID, and recognizable record links must agree with the deployment and record identity. A wrong parent, wrong requested identity or invalid link authority rejects the whole read, not merely the offending record.

The `collection` object reports `scope` (`initial` or `continuation`), `state` (`complete`, `incomplete`, `unknown`), `acceptedCount`, and only the facts that are true: `localLimit`, sanitized `failure`, `continuation`, `inconsistency`. Coverage describes this traversal segment: a complete continuation segment is still a tail, not the full conversation. An empty successful initial read is `complete` with `acceptedCount: 0`; a denied or missing collection is an error, never an empty success. An ordinary later transport or malformed-data failure preserves verified earlier pages with `incomplete` coverage. Cancellation, authority/requested-identity failure, cache invalidation or actual logical-deadline expiry rejects publication. An early acquisition stop can retain a prefix only if final acceptance still meets the original deadline. `providerCap` and `reportedTotal` are absent because the provider supplies neither.

Pages are admitted whole and only while the 1,000-record ceiling, the accepted-body ceiling and the actual final serialized document all fit, including retained metadata and continuation overhead. The record ceiling is local policy: it is not a caller `acquisition` dimension or a provider cap. Its machine-readable dimension is `collection_records`; failure facts preserve typed limit details and distinguish relative retry delays from absolute retry times. With the default ten-attempt budget a read can acquire at most nine data pages plus the parent lookup, fewer if retries or revalidations consume attempts. Both comment routes, including empty collections and resumes, require at least two attempts. `maxAttempts: 1` remains a valid upper bound but permits only the parent request; the child is refused rather than exempted from accounting.

Final failure metadata can require removing an already verified last page to keep the document within its byte ceiling. Such a result names `representation_bytes` and resumes at the first removed page, not at a later failed target. Its records, provenance and availability describe only the retained prefix. If no nonempty safe prefix fits, the read fails rather than publishing an empty substitute.

A resumable truncated collection names `collection.continuation`, an opaque `pr://.../comments/facts:cursor:<...>` reference. Read it unchanged: it contains no credential and is authenticated by an independent private key shared by clones of the session-bound source. Altered targets, other source/session contexts and old unsigned handles fail before any acquisition. A locally refused but verified page resumes at that page, not its successor; fetched malformed data and inconsistent traversal do not produce a cursor. A cursor is not a promise that an unchanged per-response ceiling can accommodate an oversized upstream body. Repeated targets or duplicate record identities are never silently completed. Continuation is best-effort traversal, not a snapshot.

Artifact recovery and source traversal are different operations. When the current document exceeds the display limit, reconstruct that one document through `recoveryReference` and its artifact continuations, with no upstream reacquisition. A source continuation is surfaced as `sourceContinuationReference` on spilled output; on inline output it can occupy `continuationReference`. Do not concatenate a source-cursor result onto the previous document: it acquires a new segment with its own JSON envelope. Parse each recovered document separately.

## GitHub review-submission and inline review-comment facts

`pr://owner/repo/<number>/reviews/facts` returns the review-submission collection, `pr://owner/repo/<number>/reviews/<id>/facts` one review, `pr://owner/repo/<number>/review-comments/facts` the inline review-comment collection and `pr://owner/repo/<number>/review-comments/<id>/facts` one inline comment. All four use the shared envelope and `acquisition` controls; both collections are schema version 1.1 and both singular reads version 1.0. Record kinds are `github.review_submission` and `github.review_comment`; the collection kinds are `github.review_submission_collection` and `github.review_comment_collection`.

A review preserves the native `id`, `nodeId`, parent identity and links, nullable `body`/`author`/`submittedAt`, supplied `state`, the review's own `commitSha` and original links. That commit names the revision the submission was made against, so an old review keeps its original commit after a later push; it is never retargeted to the current head.

An inline comment preserves the supplied review/reply relationship (`reviewId`, `replyToId`), `path`, `diffHunk`, current and original commits, `side`/`line`/`startSide`/`startLine`, `originalLine`/`originalStartLine`, native `position`/`originalPosition`, `subjectType` and links. A supplied property stays present — including an explicit null such as the current `line` of an outdated comment — an unsupplied property stays absent, and unrecognized native values are emitted verbatim. Nothing is reconstructed from the diff hunk, substituted from output coordinates or auto-retargeted, and a file-level or null-line anchor keeps its null instead of a fabricated line.

The parent pull request is read and verified first, and a supplied parent or object link that names another repository or pull request rejects the whole read, including a later page after verified earlier pages. Collections share the conversation-comment collection semantics above: whole-page admission against the record, accepted-body and serialized-document ceilings, honest `complete`/`incomplete`/`unknown` coverage, ordinary later-page prefix retention, cancellation/authority rejection, and the same opaque, credential-free, session- and resource-bound `:cursor:` continuation. The singular inline route addresses the provider's repository-wide comment endpoint, so its parent-link verification is what proves the comment belongs to the addressed pull request.

## Logging

The default sink is `info` level on standard error. Configure bounded rotating files when stderr belongs to an MCP supervisor:

```json
{
  "logging": {
    "level": "warn",
    "destination": {
      "kind": "file",
      "path": "logs/resourcefs.log",
      "rotationBytes": 1048576,
      "retainFiles": 3
    }
  }
}
```

File paths resolve from the profile directory. Rotation occurs before an append that would exceed `rotationBytes`; ResourceFS retains exactly the configured number of rotated files. Logging failures fail closed rather than writing diagnostics into MCP standard output.

## Session retention

One MCP connection owns one Path Session. Disconnect invalidates its live `local://` and `artifact://` references immediately. Retained session storage remains under the configured cache directory only for cleanup and diagnosis, then expires after `retentionTtlSeconds`. A value of `0` removes disconnected sessions immediately. Cleanup never traverses outside ResourceFS's `resourcefs` cache namespace.

## Kiro CLI

Configure Kiro to launch one local stdio command using an argument array, for example:

```json
{
  "command": "resourcefs",
  "args": ["serve", "--config", "/absolute/path/resourcefs.json"]
}
```

Do not wrap the command in a shell and do not merge standard error into standard output. ResourceFS negotiates the supported MCP revision during `initialize`; tools remain the canonical model-controlled interface even when the client does not expose MCP Resources.
