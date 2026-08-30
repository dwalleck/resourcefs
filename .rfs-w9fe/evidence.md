# SQL Server TDS transport prototype evidence

## Question

Does `mssql-client` 0.20.2 behave well enough on a real SQL Server surface to remain the production transport hypothesis for provider-neutral Database Mounts?

This is a throwaway transport probe, not a ResourceFS adapter. It does not define the final Database Mount interface, canonical row JSON, grants, profile schema, or error mapping.

## Verdict

**Yes, conditionally.** On the exercised Linux client and pinned SQL Server 2022 fixture, `mssql-client` 0.20.2 provided the deep transport module the later Source Adapter needs: typed parameters/rows, catalog queries, incremental streaming with explicit cancellation, type-state transactions, optimistic compare predicates, typed TLS/auth/server/timeout/cancellation failures, server-side Attention cancellation, and clean connection reuse afterward.

Keep `mssql-client` 0.20.2 as the candidate for the Database Mount specification. Do not yet declare it the permanent dependency: the unproven matrix below remains part of implementation acceptance/live-smoke work. `odbc-api` remains the fallback, Microsoft `mssql-tds` remains a watch item, and `sqlcmd` remains a pinned read-only oracle.

## Reproduction

Run from the repository root on branch `prototype/sql-server-tds-boundary`:

```bash
.rfs-w9fe/tds-prototype/run.sh
```

The runner starts and removes its own disposable container. It pins:

- SQL Server image: `mcr.microsoft.com/mssql/server@sha256:bf438d7104f861f5e1e1ba14b063d60a5bd883964b3c9b3b3c0064536177baa5`
- SQL Server product version observed through both transports: `16.0.4230.2`
- `mssql-client`: `0.20.2`
- ODBC `sqlcmd` in the pinned image: `18.4.0001.1 Linux`
- Rust compiler used for the captured run: `rustc 1.95.0 (59807616e 2026-04-14)`

The first runner attempt incorrectly pinned Docker's image-configuration digest rather than its runnable repository manifest digest. Docker rejected it before startup. The runner was corrected to the repository digest above; the complete rerun exited zero and removed the container. This distinguishes a reproducibility failure from a transport failure.

## Captured successful run

```text
PASS strict TLS rejects the fixture's untrusted certificate
PASS invalid credentials produce a typed authentication failure
SERVER SQL Server 16.0.4230.2
PASS fixture schema created through a fixed SQL batch
PASS typed parameters and scalar/binary/NULL row values round trip losslessly
PASS catalog views expose deterministic primary-key identity and order
PASS incremental streaming stops at a caller ceiling and leaves the connection reusable
PASS type-state transactions prove explicit rollback and commit on one connection
PASS rowversion and NULL-safe all-original-value predicates reject stale writers
PASS command timeout sends Attention, returns a typed error, and preserves connection reuse
PASS explicit cancellation terminates server work and drains the connection for reuse
PASS missing objects produce typed SQL Server error 208 without text matching
VERDICT mssql-client 0.20.2 passed the exercised TDS transport gates
16.0.4230.2
```

The final `16.0.4230.2` is the fixed read-only `sqlcmd` oracle result; it matches the in-process TDS result.

## What the probe established

### Typed values and parameter binding

A parameterized insert and read round-tripped:

- `INT`
- Unicode `NVARCHAR`
- typed `NULL` as `Option<String>`
- `VARBINARY(MAX)` bytes including zero and non-UTF-8 values
- `UNIQUEIDENTIFIER`
- `DECIMAL(18,4)`
- `DATETIMEOFFSET(7)` including the original offset
- an eight-byte `ROWVERSION`

All values crossed the driver as parameters or typed row values; no caller value was interpolated into SQL.

### Catalog identity

A named-column query over `sys.schemas`, `sys.tables`, `sys.key_constraints`, `sys.index_columns`, and `sys.columns` recovered the fixture's `dbo.rfs_w9fe_rows(id)` primary key and `key_ordinal = 1`. This proves the driver can carry the metadata required for adapter-owned row identity; it does not decide the final generic key model.

### Bounded streaming

`query_stream` began a one-million-row query, consumed exactly 128 ordered rows, sent stream cancellation at the caller ceiling, and immediately reused the same connection for `SELECT 42`. This proves incremental consumption, early stop, Attention/drain, and connection reuse. It does not measure peak RSS or prove every MAX/LOB column layout.

### Transactions

The type-state transaction interface:

- inserted and rolled back a row, which remained absent;
- inserted and committed another row, which became visible;
- returned the same client to the ready state after each outcome.

This is sufficient transport support for a later `MutationAdapter` to keep compare, DML, generated-value retrieval, and commit on one connection. The prototype did not exercise connection loss during commit acknowledgement.

### Optimistic conflicts

Two independent connections proved both planned SQL Server comparison shapes:

- PK plus stale `rowversion` affected zero rows after an external writer changed the row;
- PK plus NULL-safe original values affected zero rows after an external writer changed a table without `rowversion`.

The probe used exactly-one-row effects as the conflict signal. It did not substitute the ResourceFS SHA-256 Version Tag for an upstream SQL predicate.

### TLS, authentication, timeout, cancellation, and errors

- Strict TLS rejected the container's untrusted certificate with `Error::Tls`; the successful fixture connection used an explicit development-only trust override.
- Valid SQL authentication connected; an invalid password produced a typed authentication/server-login failure.
- A 500 ms command timeout interrupted a ten-second `WAITFOR`, returned `Error::CommandTimeout`, and left the same connection reusable.
- Explicit `CancelHandle` cancellation observed the target `WAITFOR` in `sys.dm_exec_requests`, sent Attention, received `Error::Cancelled`, observed the server request disappear, and reused the same connection.
- A missing table produced typed SQL Server error number 208 without matching message text.

### Source-audit constraints on the wrapper seam

The prototype result is conditional on a small ResourceFS-owned transport wrapper rather than exposing the driver directly:

- `query_stream` does not apply the client's command deadline internally; streamed reads need an `OperationGuard` watcher that calls `CancelHandle`, as the successful explicit-cancellation scenario did ([client source](https://docs.rs/crate/mssql-client/0.20.2/source/src/client.rs)).
- The legacy `Config::connect_timeout` field/parser and the nested `TimeoutConfig` used by connection code do not currently agree. Production configuration must set and test the nested timeout path rather than trusting the legacy builder ([configuration source](https://docs.rs/crate/mssql-client/0.20.2/source/src/config.rs), [connect source](https://docs.rs/crate/mssql-client/0.20.2/source/src/client/connect.rs)).
- Dropping `Client<InTransaction>` is not documented or implemented as an acknowledged rollback. Every adapter path must explicitly commit or roll back; an uncertain/dropped connection must be retired rather than returned to a pool ([transaction methods](https://docs.rs/crate/mssql-client/0.20.2/source/src/client.rs)).
- The adapter should use typed `Row::get` extraction, which the probe exercised. The driver's raw zero-copy accessors have a source-level buffer/slice inconsistency and are not part of this verdict ([row source](https://docs.rs/crate/mssql-client/0.20.2/source/src/row.rs), [column parser](https://docs.rs/crate/mssql-client/0.20.2/source/src/column_parser.rs)).

## Still unproven

The prototype deliberately does not settle:

- strict TLS success with a configured trusted private/public CA, certificate hostname variants, or TLS policy across Windows and macOS;
- Windows integrated/SSPI, Linux Kerberos/GSSAPI, or deployment-specific authentication beyond SQL login;
- SQL Server versions other than the pinned 2022 build;
- every SQL type and canonical JSON encoding, especially decimal precision above the Rust decimal ceiling, XML, spatial/CLR, `sql_variant`, encrypted columns, and generated fields;
- multi-gigabyte and interleaved MAX/LOB streaming or measured memory ceilings;
- pool checkout/reset behavior after timeout, cancellation, abandoned streams, routing, or transaction failure;
- network loss immediately before or after `COMMIT`, where the mutation outcome can be ambiguous;
- server failover/routing, deadlock, throttling, and permission-error classification;
- production dependency/SBOM policy for a pre-1.0 client;
- ODBC fallback deployment and cancellation behavior.

These are implementation/live-smoke gates or later Database Mount decisions. They do not falsify the transport seam demonstrated here, but production mutation must not ship until the relevant gates pass.

## Artifact contents

- `.rfs-w9fe/tds-prototype/Cargo.toml` — isolated throwaway crate pinned to `mssql-client` 0.20.2.
- `.rfs-w9fe/tds-prototype/Cargo.lock` — exact transitive dependency resolution used by the captured run.
- `.rfs-w9fe/tds-prototype/src/main.rs` — executable transport checks.
- `.rfs-w9fe/tds-prototype/run.sh` — one-command pinned SQL Server fixture, probe, `sqlcmd` oracle, and cleanup.
