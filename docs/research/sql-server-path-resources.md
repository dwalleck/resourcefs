# SQL Server path-shaped Resources: feasibility and adapter boundary

**Question.** Can the `rfs-c85i` SQLite table/row idea become a SQL Server source, and can the source be backed by `sqlcmd`?

## Executive answer

**Yes, as a logical, configured SQL Server Portable Source; no, as a general filesystem-database-path source.** A mount should name an allowed SQL Server connection and one database, while the Path Reference names a schema, table, and primary-key value. The `.mdf` path is not a portable database address: SQL Server attach is an engine operation requiring all database files, server-side file access, and database-level authority ([Attach a database][MS-attach]); LocalDB's `AttachDBFilename` is a narrowly scoped development connection-string feature ([LocalDB][MS-localdb]). A workspace `foo.mdf` can still be exposed as an ordinary file, but that must not imply that ResourceFS can open or explore it as SQL Server.

The ResourceFS **conceptual contract transfers well**: discover a bounded hierarchy, render rows as deterministic JSON, address rows by scalar/composite keys, use content-derived Version Tags, and route fixed read/write verbs through source grants and transactions. The implementation is **not a driver swap**. The repository currently has no database address variant, SQL adapter, SQL profile source, or SQL driver. `db.sqlite:users:42` is currently liable to fall through as a workspace literal/selector, and `CompiledSources` dispatches workspace addresses to the filesystem ([workspace parser][Repo-workspace-parser], [compiled registry][Repo-compiled]). SQL Server therefore needs a typed address, profile/configuration and lifecycle path, catalog/discovery dispatch, source-native type/JSON/key handling, and mutation plumbing.

**Recommendation:** make the production target an in-process TDS adapter behind a configured alias, initially one database per mount and PK-addressable rows. Ship read/list/search first. Add mutation only after deterministic SQL-type mapping, body/key validation, and both `rowversion` and no-`rowversion` compare-before-write contracts have direct tests. Treat `sqlcmd` as an optional, pinned, read-only probe/oracle—not the default adapter. It can execute a carefully generated one-batch transaction, but its substitution-based inputs and human-oriented output are a poor safety and protocol boundary for ResourceFS mutation.

## Baseline: what `rfs-c85i` actually promises

The issue asks for tables and scalar/composite-key rows as versioned JSON Resources: discovery/list/read/search, insert/full replace, hashline edit, delete, grants, transactions, deterministic JSON, content-derived tags, path/body key agreement, schema and stale-state validation, unchanged-on-failure behavior, and explicit rejection of unsupported key models and arbitrary mutating SQL ([`rfs-c85i`][Repo-issue]). The accepted design spells the row form as `db.sqlite:<table>:<percent-encoded-key>`; scalar keys keep the short spelling and composite keys use canonical encoded JSON. `rfs_write` inserts or fully replaces, `rfs_edit` edits the JSON projection, and `REM` deletes; mutations are transactional and arbitrary mutating SQL is excluded ([SQLite design][Repo-sqlite-design]).

Those requirements sit inside these ResourceFS invariants:

- A Path Reference is a source namespace plus Resource identity/hierarchy and optional projection; the operation remains in one of the fixed tools. Internal references are server-relative, not claims over globally owned URI schemes ([Path Reference contract][Repo-design]).
- Source adapters own resolution, projection, enumeration, source-specific policy, mutation, and versioning; core owns common selectors, bounded results/recovery, session identity, mutation coordination, and errors ([architecture][Repo-architecture]).
- Create, update, and delete grants are independent; configuration does not imply authority; replacements carry `ifVersion`; adapters revalidate immediately before commit; concurrent mutations serialize per canonical Resource; Version Tags identify exact authoritative content rather than modification time ([mutation rules][Repo-mutation-rules]).
- The current `VersionTag` is `sha256:` plus 64 lowercase hexadecimal characters and is computed from the authoritative content bytes ([VersionTag][Repo-version]). SQL Server's `rowversion` can be a compare token, but must not replace that content-derived API tag.

### Repository plumbing that is not yet present

This is a design investigation, not an assertion that SQLite is implemented. The current `ResourceAddress` enum has no database variant; `SourceAdapter` only exposes source reads, while discovery and mutation are separate seams ([address enum][Repo-reference], [read seam][Repo-source], [discovery seam][Repo-discovery], [mutation seam][Repo-mutation]). The current registry is exhaustive over catalog/workspace/artifact/local/HTTPS/GitHub and mutation routes only filesystem/local ([compiled registry][Repo-compiled]). A SQL Server adapter must be registered, cataloged, and routed exhaustively.

Hashline editing cannot be added only in the SQL source: `ReadEngine` records seen regions only for Workspace and named Local resources ([read recording][Repo-read]), while `PathSession` accepts only canonical Workspace or Local identities as snapshot keys ([snapshot identity][Repo-session]). A row JSON resource therefore requires widening those core identity/recording contracts (or an explicitly designed source-owned snapshot path), not a hidden adapter workaround.

Row JSON must remain source-native. Core deliberately forbids `serde_json` decoding, and the current architecture fence confines source `serde` usage to the GitHub wire module ([architecture fence][Repo-arch-fence]). A SQL adapter needs a deliberate fence/configuration update that admits source-local JSON parsing and typed-cell normalization without moving SQL JSON or profile DTOs into core. The profile's tagged Portable Source union likewise has no SQL Server entry today ([profile model][Repo-profile], [profile conversion][Repo-convert]); the source must be a configured alias with explicit grants, not an implicit file feature.

## Filesystem path, logical SQL path, and LocalDB are different things

| Addressing idea | What it identifies | ResourceFS decision |
|---|---|---|
| `workspace/.../foo.mdf` or `file://.../foo.mdf` | A file under a Workspace Root. It supplies bytes, not a SQL Server session ([Path Reference contract][Repo-design]). | Keep ordinary-file semantics. Do not infer a database, attach it, inspect tables, or mutate rows merely from the extension. |
| `sqlserver://sales/...` | A logical reference into a configured SQL Server mount. The mount owns server, authentication, database, TLS policy, and grants; the path owns schema/table/key identity ([Portable Source][Repo-context], [Path Reference contract][Repo-design]). | Recommended production model. No server hostname, credential, or physical file path appears in the canonical Resource identity. |
| `AttachDBFilename=...` with LocalDB | A provider connection request that adds an MDF to a LocalDB instance. Microsoft documents that omitting `Database` causes the attached database to be removed from the LocalDB instance when the application closes ([LocalDB][MS-localdb]). | Optional, explicitly named LocalDB development mode only. Require a named database and separate policy; do not generalize it to remote SQL Server. |
| SQL Server `CREATE DATABASE ... FOR ATTACH` | An engine/database-administration operation. All data/log files must be available, the Database Engine service account must be able to read them, and the operation requires database-creation/alter authority ([Attach a database][MS-attach]). | Out of the row adapter's normal read/write contract. If ever offered, it is a separately privileged provisioning operation, never a path-shaped row read. |

Thus a configured SQL Server mount is the same *kind* of logical source as other remote Portable Sources, not a Workspace Root. The source must be allowlisted and explicitly granted; the connection and database are not discoverable from an arbitrary user-supplied `.mdf` path.

## SQL Server capability mapping

### Discovery and table/row identity

Use SQL Server's catalog views rather than guessing from names or using `SELECT *`. Microsoft describes catalog views as the general and efficient metadata interface and warns that Microsoft may append columns to a catalog view, so production queries should select named columns ([catalog views][MS-catalog]). A bounded adapter can join `sys.schemas` (schema namespace), `sys.tables`/`sys.objects` (user-table/object identity), and `sys.columns` (column metadata, types, nullability, identity/computed flags), selecting an explicit, adapter-owned column list ([sys.schemas][MS-schemas], [sys.objects][MS-objects], [sys.tables][MS-tables], [sys.columns][MS-columns]). Filter to user tables and omit objects the caller cannot see; metadata visibility means catalog and `INFORMATION_SCHEMA` results can legitimately omit securables for which the caller lacks permission ([metadata visibility][MS-metadata]). Do not leak a hidden table merely because a guessed reference names it.

Resolve a row identity from the primary-key constraint and its enforcing index: `sys.key_constraints` supplies the key constraint and unique-index identity, while `sys.index_columns` supplies ordered `key_ordinal` and distinguishes key columns from included columns ([key constraints][MS-key-constraints], [index columns][MS-index-columns]). SQL Server primary keys may be one column or a tuple; the tuple, not any individual member, identifies the row. A primary key is limited to 32 columns/900 bytes and its columns are non-null ([PK/FK constraints][MS-pk]). Phase 1 should reject tables without a supported PK rather than inventing a row identity. A unique-key fallback is a separate decision: SQL Server UNIQUE constraints permit NULL values, so nullable, filtered, or otherwise ambiguous candidates must not silently become canonical row addresses ([unique constraints][MS-unique]).

SQL Server identifiers are not values. Regular/delimited identifiers can contain reserved or special characters, are bounded by the identifier rules, and obey identifier collation ([database identifiers][MS-identifiers]). Generated SQL should validate each configured/metadata identifier and quote it component-by-component with `QUOTENAME`; Microsoft documents that it brackets and escapes `]`, accepts a `sysname`-sized input, and returns `NULL` for overlong/invalid quote inputs ([QUOTENAME][MS-quotename]). Every key, filter, JSON body, offset, and version value must instead be a typed parameter. Microsoft documents that parameter input is treated as a literal rather than executable code and that SQL Server parameters use named `@` markers ([ADO.NET parameters][MS-parameters]); secure dynamic-SQL guidance says never concatenate parameter values and to validate/quote system names ([secure dynamic SQL][MS-secure-dynamic]).

### Candidate reference grammar

The following grammar keeps the connection/database private in a configured mount and makes a row key one unambiguous path segment:

```text
sqlserver://<mount>/                         # mounted-source catalog/root
sqlserver://<mount>/<schema>/                # schema/table listing scope
sqlserver://<mount>/<schema>/<table>         # table aggregate/list Resource
sqlserver://<mount>/<schema>/<table>/<key>  # one PK-addressable row
```

`<mount>`, `<schema>`, and `<table>` are percent-encoded path segments. `<key>` is either the scalar PK's canonical scalar spelling (the SQLite short-spelling convention) or one percent-encoded canonical JSON object whose properties appear in PK `key_ordinal` order. Examples:

```text
sqlserver://sales/sales/Orders/42
sqlserver://sales/sales/OrderLines/%7B%22OrderId%22%3A42%2C%22LineNo%22%3A3%7D
```

The object form is deliberately not a delimiter-joined tuple: it preserves names, types, nullability rules, and key order without collisions. The rendered row JSON should use one documented column order (for example, `column_id` order after explicit metadata validation), explicit scalar encodings, and explicit null behavior. A typed `SqlServerAddress { mount, schema, table, key }` should be added before the current workspace fall-through, with canonicalization and exhaustive read/search/mutation/catalog dispatch. A database segment alternative such as `sqlserver://<mount>/<database>/<schema>/<table>/<key>` could support multiple databases behind one server alias, but it expands cross-database metadata and permission boundaries, leaks or duplicates database identity in references, and complicates Azure SQL/contained-database deployment. Prefer one configured mount = one database until those boundaries are designed.

### Read and deterministic JSON

For each row, generate a fixed `SELECT` with explicit, metadata-derived columns and parameterized PK predicates. `FOR JSON PATH` gives explicit control over output shape; SQL Server omits `NULL` properties by default, `INCLUDE_NULL_VALUES` includes them, and `WITHOUT_ARRAY_WRAPPER` produces a single object for a single-row result ([FOR JSON][MS-for-json]). The adapter must choose one policy (recommended: include nulls and use one object), define conversion for every admitted SQL type, and hash exactly the resulting canonical UTF-8 bytes. `FOR JSON` is a useful projection primitive, not by itself a ResourceFS determinism guarantee: SQL Server says large JSON may be split over multiple result rows that clients must concatenate ([FOR JSON][MS-for-json]).

If the adapter accepts replacement JSON, parse and validate it in the sources crate against the discovered schema before DML. SQL Server's `OPENJSON ... WITH` can project declared paths and SQL types, but missing paths become `NULL`; that is insufficient by itself to distinguish omitted from explicitly-null properties ([OPENJSON][MS-openjson]). ResourceFS must enforce required/key fields and path/body key agreement before issuing SQL, and must reject unsupported or lossy SQL types rather than silently changing their meaning.

### List, search, and pagination

Table/schema listing is catalog enumeration, not filesystem globbing. Row list/search needs bounded SQL and deterministic order. SQL Server documents that row order is not guaranteed without `ORDER BY`; `OFFSET`/`FETCH` require ordered paging, and their values can be parameters ([ORDER BY/OFFSET][MS-order-by]). Use an order containing the complete unique PK tuple as the tie-breaker. A continuation over changing data can skip or duplicate rows; a stable multi-page snapshot requires an explicitly chosen isolation/transaction policy, not merely an offset number ([isolation levels][MS-isolation]).

The ResourceFS search contract and SQL Server predicates are not automatically equivalent. `LIKE` has wildcard, `ESCAPE`, collation, and character/trailing-space behavior ([LIKE][MS-like]). A portable default can read bounded deterministic row JSON and apply the existing source-local search engine; a pushed-down literal mode must document its `LIKE` semantics and escape user literals. Full-text search is optional and index/configuration dependent; Microsoft describes it as a separate language-aware facility, and its full-text index requires a unique non-null key ([full-text search][MS-fulltext]). Do not advertise full-text semantics for every mount.

### Transactions, mutations, and failure atomicity

SQL Server transactions are all-or-nothing units: explicit `BEGIN TRANSACTION` ends in `COMMIT` or `ROLLBACK`, while autocommit otherwise treats each statement independently ([transactions][MS-transactions]). A mutation should keep validation, compare, DML, generated-value retrieval, and commit on one connection and one short explicit transaction. Long transactions can retain locks and delay cleanup; isolation is a source policy, not an incidental driver default ([BEGIN TRANSACTION][MS-begin], [isolation levels][MS-isolation]).

Generate only ResourceFS operations: insert, full replace, edit-projection replace, and delete. Bind all values and quote only validated metadata identifiers. Reject identity/computed columns as client-controlled writes unless a separately designed server-default contract handles them; `sys.columns` exposes those flags ([sys.columns][MS-columns]). On any constraint/type/permission failure, roll back and return a stable ResourceFS error without publishing a new tag. No arbitrary mutating SQL belongs in the API.

### Optimistic concurrency and Version Tags

If a table has a `rowversion` column, include it in the read projection as a stable binary encoding, but keep it separate from the ResourceFS Version Tag. SQL Server documents `rowversion` as an 8-byte database counter: it increments on insert/update of a rowversion-bearing table, is not a date/time, and increments even when an update writes the same value; it is also a poor key ([rowversion][MS-rowversion]). A conditional update/delete should include PK plus the exact old rowversion, return/obtain the new rowversion, and treat zero affected rows as a version conflict.

For a table without rowversion, use PK plus null-safe comparisons for every original value covered by the Resource projection. Microsoft's optimistic-concurrency guidance shows all-original-column predicates, requires a unique key to avoid affecting multiple rows, and treats `RecordsAffected == 0` as a conflict; it also calls out explicit NULL handling ([optimistic concurrency][MS-optimistic]). Checking only a ResourceFS SHA-256 digest is not a substitute for a conditional predicate: the digest is the API's content identity, while the adapter must prove that the authoritative row still matches. If the projection omits columns that may change, either include them in the authoritative snapshot or reject no-rowversion mutation for that table.

The content-derived Version Tag remains `sha256:` over ResourceFS's canonical JSON bytes. `rowversion` is an upstream compare token, not a content hash; an unchanged-content SQL UPDATE can still advance it. This separation lets reads and receipts retain the common ResourceFS tag while the adapter chooses the strongest source compare contract.

### Grants, connection, and security

A SQL Server source needs two independent authorization layers:

1. ResourceFS source grants: `read/list/search`, create, update, and delete remain explicit and independent; merely configuring a mount never grants mutation ([Mutation Policy][Repo-context], [grants implementation][Repo-grants]).
2. SQL Server permissions: use a least-privilege login/user/role. `SELECT` is needed for reads/search; catalog visibility depends on ownership/permission and may require carefully scoped `VIEW DEFINITION`; absence of metadata visibility must result in bounded omission or an explicit unavailable/unsupported result, not guessed metadata ([metadata visibility][MS-metadata], [permissions][MS-permissions]).

The profile should accept a validated connection configuration or secret reference, never an untrusted concatenated connection string. Microsoft warns that manually constructed connection strings can be vulnerable to connection-string injection and recommends a strongly typed builder ([connection strings][MS-connection-strings]). Support the deployment's explicit authentication mode (Windows/integrated, SQL login, or Microsoft Entra where applicable), do not log credentials, and avoid `sa`/broad administrator authority ([SQL authentication][MS-auth]). Require encrypted transport with verifiable certificate validation; `TrustServerCertificate` bypasses normal certificate validation and must never be silently enabled for production ([encryption and certificate validation][MS-encryption]).

## `sqlcmd` as a backing implementation

### What Microsoft documents

Microsoft provides two cross-platform `sqlcmd` variants: Go (`go-mssqldb`-based, standalone) and ODBC (the traditional utility distributed with SQL Server/Command Line Utilities). Both are documented for Windows, macOS, Linux, and containers, but their options and behavior differ ([sqlcmd utility][MS-sqlcmd], [installation][MS-sqlcmd-install]). The install documentation also notes that the ODBC download/build may differ from the copy shipped with a SQL Server cumulative update ([installation][MS-sqlcmd-install]); a ResourceFS adapter cannot assume one stable executable protocol without pinning and probing the exact variant/version.

`sqlcmd` accepts command-line queries (`-q`/`-Q`), input scripts (`-i`), and output redirection (`-o`/`:Out`). Its normal row-oriented output includes column headers, separators, padding, and row-count text ([sqlcmd utility][MS-sqlcmd], [sqlcmd commands][MS-sqlcmd-commands]). SQL Server's `FOR JSON` can produce a single-column JSON result, but long JSON may be split into rows ([FOR JSON][MS-for-json]); the `sqlcmd` documentation says `:XML ON` is required to suppress the column name and obtain valid JSON output, while the commands page marks `:XML` unsupported on Linux and macOS ([sqlcmd commands][MS-sqlcmd-commands]). Therefore stdout is not a portable, typed, structured ResourceFS wire protocol. `-h`, `-W`, separators, and output files can reduce formatting noise but do not establish framing or SQL-type fidelity.

`sqlcmd` scripting variables are textual `$(VARNAME)` substitutions set by `-v`, `:Setvar`, or the environment; `-x` disables substitution ([sqlcmd commands][MS-sqlcmd-commands], [sqlcmd utility][MS-sqlcmd]). This is not the typed parameter-object API documented for an in-process SQL client ([ADO.NET parameters][MS-parameters]). Identifiers cannot be bound as values, so a generated script still needs validated/quoted identifiers. A fixed script could call `sp_executesql`, but values must still cross the script boundary and the adapter would own all escaping/framing; that is needlessly risky for arbitrary row JSON. `-X[1]` disables dangerous commands/startup script and prevents environment variables from being passed to `sqlcmd`; consequently, do not claim that `SQLCMDPASSWORD` works together with `-X[1]` without testing the exact variant and credential path ([sqlcmd utility][MS-sqlcmd]).

Authentication and transport are exposed as flags/environment rather than a ResourceFS connection object: `-E`, `-G`, `-U`, `-P`, `SQLCMDPASSWORD`, `-N`, and `-C` are documented options. Microsoft explicitly calls `-P` insecure and recommends `SQLCMDPASSWORD` or interactive input; `-C` trusts the server certificate, while `-N` controls encryption ([sqlcmd utility][MS-sqlcmd]). For an automated prototype, pass a fixed script through stdin or a tightly controlled input file, use `-x`, avoid `-P`, and choose a credential path that has been tested with the selected variant. Do not combine `-X[1]` with an environment-secret recipe unless the resulting authentication behavior is verified.

`-t` supplies a query timeout; it is not a complete ResourceFS cancellation contract ([sqlcmd utility][MS-sqlcmd]). `-b` returns a nonzero error level when a qualifying SQL error occurs, `-V` controls the severity threshold, and callers are expected to inspect the exit code ([sqlcmd utility][MS-sqlcmd]). The adapter would still have to distinguish login/transport failure, SQL error, timeout, cancellation, zero-row optimistic conflict, and malformed/truncated stdout. A killed subprocess is not a typed server-side cancellation API; any prototype must prove rollback and cleanup behavior for that failure mode.

A single `sqlcmd` invocation can submit a fixed multi-statement batch, so compare + DML + `COMMIT` can be atomic if they remain on the same connection/session. Separate invocations cannot retain that transaction/session. `GO` separates batches, and `:Connect` closes the current connection ([sqlcmd commands][MS-sqlcmd-commands]); SQL Server's transaction semantics still require explicit commit/rollback ([transactions][MS-transactions]). Thus one-batch mutation is possible in principle, but robustly reporting `RecordsAffected`, generated rowversion, canonical JSON, and conflict/error categories through formatted stdout is not a good production boundary.

### Decision

| Concern | In-process TDS client | `sqlcmd` subprocess |
|---|---|---|
| Parameter values | Typed parameters can carry keys, nulls, binary, JSON, offsets, and old compare tokens as literals rather than code ([ADO.NET parameters][MS-parameters]). | `-v`/`:Setvar` substitution is text; the adapter must escape and frame every value ([sqlcmd commands][MS-sqlcmd-commands]). |
| Identifiers | Generated SQL can quote validated metadata components with `QUOTENAME` ([MS-quotename]). | Same quoting is possible only in generated script text; no identifier/value separation at the process boundary. |
| JSON/type fidelity | A reader can receive typed columns, normalize in sources, and hash canonical bytes. | Normal stdout is padded/tabular; `FOR JSON` plus `:XML ON` has a documented Linux/macOS limitation and large-result row splitting ([MS-sqlcmd-commands], [MS-for-json]). |
| Transactions/session | One connection/transaction object can hold compare, DML, `OUTPUT`, and commit. | Must put the entire operation in one invocation; process death, reconnect, and output parsing are additional failure states. |
| Cancellation/timeout | Candidate driver API can be integrated with `OperationGuard` and bounded reads; required behavior must be proven for the chosen implementation. | `-t` is query timeout; cancellation is at best subprocess lifecycle unless the exact utility proves more ([MS-sqlcmd]). |
| Error/conflict result | Rows affected and typed output are directly inspectable. | Requires `-b`/`-V`, stderr/exit parsing, and a collision-free stdout protocol ([MS-sqlcmd]). |
| Deployment | One library/runtime dependency under the ResourceFS binary, subject to driver support testing. | Separate Go/ODBC executable, installation, PATH/container packaging, version drift, and flag/default differences ([MS-sqlcmd], [MS-sqlcmd-install]). |
| Security | Secret references and certificate policy stay inside the configured source/client. | Environment/command-line/process inspection and `-X[1]` interactions need an explicit threat model ([MS-sqlcmd]). |

**Production choice:** use an in-process TDS adapter. **Bounded prototype choice:** an optional read-only `sqlcmd` oracle may validate catalog queries and a small `FOR JSON` row projection on pinned Go and ODBC versions. It must report the exact variant/version, enforce output ceilings, parse only a deliberately framed fixed query, treat nonzero exit/stderr as failure, and never accept arbitrary caller SQL. Do not use that prototype as evidence that `sqlcmd` is suitable for production mutation.

## Recommended implementation scope

1. **Mount and identity:** add a strict `sqlserver` Portable Source profile entry with alias, server/instance, one database, authentication/secret reference, TLS policy, timeouts, and independent source grants. Canonical references use the candidate grammar above; raw `.mdf` paths are never accepted as SQL mounts.
2. **Typed plumbing:** add `SqlServerAddress` and parser precedence, canonical segment/key encoding, catalog metadata, `CompiledSources` registration/dispatch, and source-local schema/key/type logic. Update catalog examples and profile checks. Widen `SnapshotResourceKey`/`ReadEngine` seen recording for editable SQL rows before enabling hashline edit.
3. **Read-only phase:** discover visible schemas/tables/columns/PKs; reject unsupported/no-PK models; provide bounded list/read/search with explicit PK ordering, continuation, collation/search semantics, deterministic JSON, and content-derived tags. Start with a documented SQL type subset and fail closed for lossy/unsupported types.
4. **Mutation phase:** implement insert/full replace/edit/delete only through generated fixed statements with typed parameters, path/body key agreement, grants, short transactions, and immediate compare-before-commit. Use PK + rowversion where available; otherwise PK + null-safe original-value predicates and exactly-one-row checks. Return new canonical JSON/tag and preserve unchanged state on all failures.
5. **Proof:** direct adapter fixtures must cover catalog visibility, scalar/composite keys, nulls and generated columns, deterministic JSON, pagination under the chosen isolation policy, stale rowversion and no-rowversion conflicts, rollback on constraint failure, permissions/TLS/auth failures, cancellation/timeout, and malicious identifiers/values. Add a separate pinned `sqlcmd` read-only smoke oracle if desired; it is not the mutation implementation.

## Unresolved design decisions

- Which Rust in-process TDS implementation and supported SQL Server/Azure SQL versions provide typed values, cancellation, TLS validation, transactions, and metadata behavior required by the seam?
- Is one mount permanently one database, or may a mount enumerate multiple explicitly allowlisted databases? If the latter, how are cross-database metadata visibility and Azure/contained-user rules represented?
- What exact SQL type subset and canonical JSON encoding is supported for `decimal`, date/time/offset, `uniqueidentifier`, binary, XML, spatial, `sql_variant`, CLR/UDT, encrypted, and large-value columns? Which values are strings versus JSON numbers, and how are missing versus explicit `null` represented?
- Does row JSON include generated identity/computed/rowversion columns, and are they editable, read-only, or regenerated on replace? How is a returned rowversion encoded and refreshed after insert/update?
- For no-rowversion tables, does the authoritative projection include every changeable column, or are mutations rejected when a hidden/unprojected column could change? Which isolation level is required for multi-page list/search snapshots?
- Does hashline editing operate on pretty-printed canonical JSON, and how does it preserve key types, generated fields, and path/body key agreement while avoiding edits to unseen content?
- Are unique non-PK keys ever addressable? If yes, how are nullable, filtered, collation-sensitive, and mutable unique constraints represented without identity surprises?
- Is `sqlcmd` kept only as an opt-in probe, and which exact variant/version/output framing is pinned? If a future stored-procedure protocol is considered, can it provide typed parameters, bounded structured output, cancellation, and unambiguous transaction/conflict receipts without exposing arbitrary SQL?

## Primary sources

### ResourceFS repository sources

- [`rfs-c85i` issue][Repo-issue]
- [`DESIGN.md` Path Reference and SQLite contract][Repo-sqlite-design]
- [`DESIGN.md` mutation/version rules][Repo-mutation-rules]
- [`DESIGN.md` architecture][Repo-architecture]
- [`CONTEXT.md` domain terms and Mutation Policy][Repo-context]
- [`reference.rs` address model][Repo-reference] and [workspace fallback parser][Repo-workspace-parser]
- [`source.rs` SourceAdapter][Repo-source]
- [`discovery.rs` DiscoveryAdapter][Repo-discovery]
- [`mutation.rs` MutationAdapter/Engine][Repo-mutation]
- [`read.rs` seen recording][Repo-read]
- [`session.rs` snapshot identity][Repo-session]
- [`version.rs` VersionTag][Repo-version]
- [`compiled.rs` registry/dispatch][Repo-compiled]
- [`architecture_contract.rs` JSON/dependency fence][Repo-arch-fence]
- [`profile/model.rs` strict profile][Repo-profile] and [`profile/convert.rs`][Repo-convert]
- [`configuration/grants.rs` source grants][Repo-grants]

### Microsoft / first-party SQL Server and sqlcmd documentation

- [System catalog views][MS-catalog]; [sys.schemas][MS-schemas]; [sys.objects][MS-objects]; [sys.tables][MS-tables]; [sys.columns][MS-columns]
- [sys.key_constraints][MS-key-constraints]; [sys.index_columns][MS-index-columns]; [Primary and foreign key constraints][MS-pk]; [UNIQUE and CHECK constraints][MS-unique]
- [Database identifiers][MS-identifiers]; [QUOTENAME][MS-quotename]; [ADO.NET parameters][MS-parameters]; [Writing secure dynamic SQL][MS-secure-dynamic]
- [Transactions][MS-transactions]; [BEGIN TRANSACTION][MS-begin]; [SET TRANSACTION ISOLATION LEVEL][MS-isolation]
- [rowversion][MS-rowversion]; [Optimistic concurrency][MS-optimistic]
- [Format query results as JSON with FOR JSON][MS-for-json]; [OPENJSON][MS-openjson]
- [ORDER BY/OFFSET/FETCH][MS-order-by]; [LIKE][MS-like]; [Full-text search][MS-fulltext]
- [Metadata visibility][MS-metadata]; [Database Engine permissions][MS-permissions]; [SQL Server authentication][MS-auth]; [Connection strings][MS-connection-strings]; [Encryption and certificate validation][MS-encryption]
- [Attach a database][MS-attach]; [SqlClient support for LocalDB][MS-localdb]
- [sqlcmd utility][MS-sqlcmd]; [sqlcmd commands][MS-sqlcmd-commands]; [sqlcmd installation][MS-sqlcmd-install]

[Repo-issue]: ../../.rivets/issues.jsonl#L17
[Repo-design]: ../../DESIGN.md#path-reference-contract
[Repo-sqlite-design]: ../../DESIGN.md#sqlite
[Repo-mutation-rules]: ../../DESIGN.md#mutation-versioning-and-concurrency
[Repo-architecture]: ../../DESIGN.md#internal-architecture
[Repo-context]: ../../CONTEXT.md#mutation-policy
[Repo-reference]: ../../crates/resourcefs-core/src/reference.rs#L590-L600
[Repo-workspace-parser]: ../../crates/resourcefs-core/src/reference.rs#L1341-L1405
[Repo-source]: ../../crates/resourcefs-core/src/source.rs#L5-L20
[Repo-discovery]: ../../crates/resourcefs-core/src/discovery.rs#L451-L467
[Repo-mutation]: ../../crates/resourcefs-core/src/mutation.rs#L1157-L1177
[Repo-read]: ../../crates/resourcefs-core/src/read.rs#L226-L245
[Repo-session]: ../../crates/resourcefs-core/src/session.rs#L401-L425
[Repo-version]: ../../crates/resourcefs-core/src/version.rs#L5-L50
[Repo-compiled]: ../../crates/resourcefs-sources/src/compiled.rs#L16-L279
[Repo-arch-fence]: ../../crates/resourcefs-mcp/tests/architecture_contract.rs#L36-L88
[Repo-profile]: ../../crates/resourcefs-mcp/src/profile/model.rs#L850-L874
[Repo-convert]: ../../crates/resourcefs-mcp/src/profile/convert.rs#L36-L201
[Repo-grants]: ../../crates/resourcefs-sources/src/configuration/grants.rs#L3-L80

[MS-catalog]: https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/catalog-views-transact-sql?view=sql-server-ver17
[MS-schemas]: https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/schemas-catalog-views-sys-schemas?view=sql-server-ver17
[MS-objects]: https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-objects-transact-sql?view=sql-server-ver17
[MS-tables]: https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-tables-transact-sql?view=sql-server-ver17
[MS-columns]: https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-columns-transact-sql?view=sql-server-ver17
[MS-key-constraints]: https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-key-constraints-transact-sql?view=sql-server-ver17
[MS-index-columns]: https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-index-columns-transact-sql?view=sql-server-ver17
[MS-pk]: https://learn.microsoft.com/en-us/sql/relational-databases/tables/primary-and-foreign-key-constraints?view=sql-server-ver17
[MS-unique]: https://learn.microsoft.com/en-us/sql/relational-databases/tables/unique-constraints-and-check-constraints?view=sql-server-ver17
[MS-identifiers]: https://learn.microsoft.com/en-us/sql/relational-databases/databases/database-identifiers?view=sql-server-ver17
[MS-quotename]: https://learn.microsoft.com/en-us/sql/t-sql/functions/quotename-transact-sql?view=sql-server-ver17
[MS-parameters]: https://learn.microsoft.com/en-us/dotnet/framework/data/adonet/configuring-parameters-and-parameter-data-types
[MS-secure-dynamic]: https://learn.microsoft.com/en-us/sql/connect/ado-net/sql/writing-secure-dynamic-sql?view=sql-server-ver17
[MS-transactions]: https://learn.microsoft.com/en-us/sql/t-sql/language-elements/transactions-transact-sql?view=sql-server-ver17
[MS-begin]: https://learn.microsoft.com/en-us/sql/t-sql/language-elements/begin-transaction-transact-sql?view=sql-server-ver17
[MS-isolation]: https://learn.microsoft.com/en-us/sql/t-sql/statements/set-transaction-isolation-level-transact-sql?view=sql-server-ver17
[MS-rowversion]: https://learn.microsoft.com/en-us/sql/t-sql/data-types/rowversion-transact-sql?view=sql-server-ver17
[MS-optimistic]: https://learn.microsoft.com/en-us/sql/connect/ado-net/optimistic-concurrency?view=sql-server-ver17
[MS-for-json]: https://learn.microsoft.com/en-us/sql/relational-databases/json/format-query-results-as-json-with-for-json-sql-server?view=sql-server-ver17
[MS-openjson]: https://learn.microsoft.com/en-us/sql/relational-databases/json/convert-json-data-to-rows-and-columns-with-openjson-sql-server?view=sql-server-ver17
[MS-order-by]: https://learn.microsoft.com/en-us/sql/t-sql/queries/select-order-by-clause-transact-sql?view=sql-server-ver17
[MS-like]: https://learn.microsoft.com/en-us/sql/t-sql/language-elements/like-transact-sql?view=sql-server-ver17
[MS-fulltext]: https://learn.microsoft.com/en-us/sql/relational-databases/search/full-text-search?view=sql-server-ver17
[MS-metadata]: https://learn.microsoft.com/en-us/sql/relational-databases/security/metadata-visibility-configuration?view=sql-server-ver17
[MS-permissions]: https://learn.microsoft.com/en-us/sql/relational-databases/security/authentication-access/getting-started-with-database-engine-permissions?view=sql-server-ver17
[MS-auth]: https://learn.microsoft.com/en-us/sql/connect/ado-net/sql/authentication-sql-server?view=sql-server-ver17
[MS-connection-strings]: https://learn.microsoft.com/en-us/sql/connect/ado-net/connection-strings?view=sql-server-ver17
[MS-encryption]: https://learn.microsoft.com/en-us/sql/connect/ado-net/encryption-and-certificate-validation?view=sql-server-ver17
[MS-attach]: https://learn.microsoft.com/en-us/sql/relational-databases/databases/attach-a-database?view=sql-server-ver17
[MS-localdb]: https://learn.microsoft.com/en-us/sql/connect/ado-net/sql/sqlclient-support-localdb?view=sql-server-ver17
[MS-sqlcmd]: https://learn.microsoft.com/en-us/sql/tools/sqlcmd/sqlcmd-utility?view=sql-server-ver17
[MS-sqlcmd-commands]: https://learn.microsoft.com/en-us/sql/tools/sqlcmd/sqlcmd-commands?view=sql-server-ver17
[MS-sqlcmd-install]: https://learn.microsoft.com/en-us/sql/tools/sqlcmd/sqlcmd-download-install?view=sql-server-ver17
