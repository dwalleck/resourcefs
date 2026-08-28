use std::{
    env,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, FixedOffset};
use mssql_client::{Client, Config, Error as MssqlError, Ready, ToSql};
use rust_decimal::Decimal;
use uuid::Uuid;

const HOST: &str = "127.0.0.1,14339";
const DATABASE: &str = "tempdb";
const USER: &str = "sa";
const DEFAULT_PASSWORD: &str = "Rfs_w9fe!Prototype42";

#[tokio::main]
async fn main() -> Result<()> {
    let password =
        env::var("RFS_W9FE_SQL_PASSWORD").unwrap_or_else(|_| DEFAULT_PASSWORD.to_owned());

    prove_tls_rejection(&password).await?;
    prove_authentication_error().await?;

    let mut client = Client::connect(config(&password, true)?)
        .await
        .context("connect to disposable SQL Server")?;
    let version = scalar_string(
        &mut client,
        "SELECT CAST(SERVERPROPERTY('ProductVersion') AS NVARCHAR(128))",
        &[],
    )
    .await?;
    println!("SERVER SQL Server {version}");

    create_fixture(&mut client).await?;
    prove_typed_parameters_and_rows(&mut client).await?;
    prove_primary_key_metadata(&mut client).await?;
    prove_bounded_stream_and_reuse(&mut client).await?;
    client = prove_transactions(client).await?;
    client = prove_optimistic_conflicts(client, &password).await?;
    prove_timeout_and_reuse(&mut client).await?;
    client = prove_explicit_cancellation(client, &password).await?;
    prove_server_error_classification(&mut client).await?;

    client
        .execute(
            "DROP TABLE IF EXISTS dbo.rfs_w9fe_rows; DROP TABLE IF EXISTS dbo.rfs_w9fe_plain;",
            &[],
        )
        .await
        .context("drop prototype fixtures")?;
    client.close().await.context("close prototype connection")?;

    println!("VERDICT mssql-client 0.20.2 passed the exercised TDS transport gates");
    Ok(())
}

fn config(password: &str, trust_server_certificate: bool) -> Result<Config> {
    let trust = if trust_server_certificate {
        "true"
    } else {
        "false"
    };
    let connection_string = format!(
        "Server={HOST};Database={DATABASE};User Id={USER};Password={password};Encrypt=true;TrustServerCertificate={trust}"
    );
    Config::from_connection_string(&connection_string).context("parse prototype connection string")
}

async fn prove_tls_rejection(password: &str) -> Result<()> {
    match Client::connect(config(password, false)?).await {
        Err(MssqlError::Tls(_)) => {
            pass("strict TLS rejects the fixture's untrusted certificate");
            Ok(())
        }
        Err(other) => bail!("strict TLS produced the wrong error category: {other:?}"),
        Ok(client) => {
            client
                .close()
                .await
                .context("close unexpected TLS connection")?;
            bail!("strict TLS unexpectedly trusted the disposable server certificate")
        }
    }
}

async fn prove_authentication_error() -> Result<()> {
    match Client::connect(config("Rfs_w9fe!DefinitelyWrong42", true)?).await {
        Err(MssqlError::Authentication(_)) | Err(MssqlError::Server { number: 18456, .. }) => {
            pass("invalid credentials produce a typed authentication failure");
            Ok(())
        }
        Err(other) => bail!("invalid credentials produced the wrong error category: {other:?}"),
        Ok(client) => {
            client
                .close()
                .await
                .context("close unexpected authenticated connection")?;
            bail!("invalid credentials unexpectedly authenticated")
        }
    }
}

async fn create_fixture(client: &mut Client<Ready>) -> Result<()> {
    client
        .execute(
            r#"
            DROP TABLE IF EXISTS dbo.rfs_w9fe_rows;
            DROP TABLE IF EXISTS dbo.rfs_w9fe_plain;
            CREATE TABLE dbo.rfs_w9fe_rows (
                id INT NOT NULL CONSTRAINT PK_rfs_w9fe_rows PRIMARY KEY,
                name NVARCHAR(100) NOT NULL,
                maybe_null NVARCHAR(100) NULL,
                payload VARBINARY(MAX) NOT NULL,
                guid UNIQUEIDENTIFIER NOT NULL,
                amount DECIMAL(18,4) NOT NULL,
                observed_at DATETIMEOFFSET(7) NOT NULL,
                rv ROWVERSION NOT NULL
            );
            CREATE TABLE dbo.rfs_w9fe_plain (
                id INT NOT NULL CONSTRAINT PK_rfs_w9fe_plain PRIMARY KEY,
                name NVARCHAR(100) NULL,
                quantity INT NOT NULL
            );
            "#,
            &[],
        )
        .await
        .context("create prototype tables")?;
    pass("fixture schema created through a fixed SQL batch");
    Ok(())
}

async fn prove_typed_parameters_and_rows(client: &mut Client<Ready>) -> Result<()> {
    let id = 1i32;
    let name = "Grüße from ResourceFS";
    let maybe_null: Option<String> = None;
    let payload = vec![0_u8, 1, 2, 0xfe, 0xff];
    let guid = Uuid::parse_str("12345678-90ab-cdef-1234-567890abcdef")?;
    let amount: Decimal = "1234567890.1250".parse()?;
    let observed_at: DateTime<FixedOffset> =
        DateTime::parse_from_rfc3339("2026-08-28T12:34:56.1234567+05:30")?;

    let affected = client
        .execute(
            "INSERT INTO dbo.rfs_w9fe_rows \
             (id, name, maybe_null, payload, guid, amount, observed_at) \
             VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)",
            &[
                &id,
                &name,
                &maybe_null,
                &payload,
                &guid,
                &amount,
                &observed_at,
            ],
        )
        .await
        .context("insert typed fixture row")?;
    ensure!(
        affected == 1,
        "typed insert affected {affected} rows, expected one"
    );

    let mut rows = client
        .query(
            "SELECT id, name, maybe_null, payload, guid, amount, observed_at, rv \
             FROM dbo.rfs_w9fe_rows WHERE id = @p1",
            &[&id],
        )
        .await
        .context("read typed fixture row")?;
    let row = rows
        .next()
        .transpose()
        .context("decode typed fixture row")?
        .context("typed fixture row is missing")?;

    let got_id: i32 = row.get(0)?;
    let got_name: String = row.get(1)?;
    let got_null: Option<String> = row.get(2)?;
    let got_payload: Vec<u8> = row.get(3)?;
    let got_guid: Uuid = row.get(4)?;
    let got_amount: Decimal = row.get(5)?;
    let got_observed_at: DateTime<FixedOffset> = row.get(6)?;
    let rowversion: Vec<u8> = row.get(7)?;

    ensure!(got_id == id, "integer round trip changed value");
    ensure!(got_name == name, "Unicode text round trip changed value");
    ensure!(
        got_null.is_none(),
        "SQL NULL did not decode as Option::None"
    );
    ensure!(got_payload == payload, "binary round trip changed bytes");
    ensure!(
        got_guid == guid,
        "uniqueidentifier round trip changed value"
    );
    ensure!(
        got_amount == amount,
        "decimal round trip changed precision or scale"
    );
    ensure!(
        got_observed_at.to_rfc3339() == observed_at.to_rfc3339(),
        "datetimeoffset round trip changed instant or offset"
    );
    ensure!(
        rowversion.len() == 8,
        "rowversion length was {}, expected 8",
        rowversion.len()
    );

    pass("typed parameters and scalar/binary/NULL row values round trip losslessly");
    Ok(())
}

async fn prove_primary_key_metadata(client: &mut Client<Ready>) -> Result<()> {
    let mut rows = client
        .query(
            r#"
            SELECT s.name, t.name, c.name, CAST(ic.key_ordinal AS INT)
            FROM sys.tables AS t
            JOIN sys.schemas AS s ON s.schema_id = t.schema_id
            JOIN sys.key_constraints AS kc
              ON kc.parent_object_id = t.object_id AND kc.type = 'PK'
            JOIN sys.index_columns AS ic
              ON ic.object_id = kc.parent_object_id AND ic.index_id = kc.unique_index_id
            JOIN sys.columns AS c
              ON c.object_id = ic.object_id AND c.column_id = ic.column_id
            WHERE s.name = @p1 AND t.name = @p2
            ORDER BY ic.key_ordinal
            "#,
            &[&"dbo", &"rfs_w9fe_rows"],
        )
        .await
        .context("query primary-key metadata")?;
    let row = rows
        .next()
        .transpose()?
        .context("primary-key metadata row is missing")?;
    let schema: String = row.get(0)?;
    let table: String = row.get(1)?;
    let column: String = row.get(2)?;
    let ordinal: i32 = row.get(3)?;
    ensure!(
        schema == "dbo" && table == "rfs_w9fe_rows",
        "metadata named the wrong table"
    );
    ensure!(
        column == "id" && ordinal == 1,
        "metadata returned the wrong PK identity"
    );
    ensure!(
        rows.next().is_none(),
        "metadata returned an unexpected second PK column"
    );
    pass("catalog views expose deterministic primary-key identity and order");
    Ok(())
}

async fn prove_bounded_stream_and_reuse(client: &mut Client<Ready>) -> Result<()> {
    let mut stream = client
        .query_stream(
            "SELECT TOP (1000000) \
             CAST(ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS INT) AS n \
             FROM sys.all_objects AS a CROSS JOIN sys.all_objects AS b",
            &[],
        )
        .await
        .context("start large incremental row stream")?;

    let mut consumed = 0i32;
    while consumed < 128 {
        let row = stream
            .try_next()
            .await?
            .context("large stream ended before the prototype ceiling")?;
        let value: i32 = row.get(0)?;
        ensure!(
            value == consumed + 1,
            "stream ordering changed at row {value}"
        );
        consumed += 1;
    }
    stream
        .cancel()
        .await
        .context("cancel bounded stream at ceiling")?;

    let value = scalar_i32(client, "SELECT 42", &[]).await?;
    ensure!(
        value == 42,
        "connection returned stale data after stream cancellation"
    );
    pass("incremental streaming stops at a caller ceiling and leaves the connection reusable");
    Ok(())
}

async fn prove_transactions(mut client: Client<Ready>) -> Result<Client<Ready>> {
    let mut rollback_tx = client
        .begin_transaction()
        .await
        .context("begin rollback transaction")?;
    rollback_tx
        .execute(
            "INSERT INTO dbo.rfs_w9fe_plain (id, name, quantity) VALUES (@p1, @p2, @p3)",
            &[&2i32, &"rollback", &2i32],
        )
        .await?;
    client = rollback_tx
        .rollback()
        .await
        .context("rollback transaction")?;
    ensure!(
        scalar_i32(
            &mut client,
            "SELECT COUNT(*) FROM dbo.rfs_w9fe_plain WHERE id = 2",
            &[]
        )
        .await?
            == 0,
        "rolled-back insert remained visible"
    );

    let mut commit_tx = client
        .begin_transaction()
        .await
        .context("begin commit transaction")?;
    commit_tx
        .execute(
            "INSERT INTO dbo.rfs_w9fe_plain (id, name, quantity) VALUES (@p1, @p2, @p3)",
            &[&3i32, &"commit", &3i32],
        )
        .await?;
    client = commit_tx.commit().await.context("commit transaction")?;
    ensure!(
        scalar_i32(
            &mut client,
            "SELECT COUNT(*) FROM dbo.rfs_w9fe_plain WHERE id = 3",
            &[]
        )
        .await?
            == 1,
        "committed insert was not visible"
    );

    pass("type-state transactions prove explicit rollback and commit on one connection");
    Ok(client)
}

async fn prove_optimistic_conflicts(
    mut client: Client<Ready>,
    password: &str,
) -> Result<Client<Ready>> {
    let old_rowversion = scalar_bytes(
        &mut client,
        "SELECT rv FROM dbo.rfs_w9fe_rows WHERE id = 1",
        &[],
    )
    .await?;
    let mut concurrent = Client::connect(config(password, true)?).await?;
    concurrent
        .execute(
            "UPDATE dbo.rfs_w9fe_rows SET name = @p1 WHERE id = @p2",
            &[&"external-writer", &1i32],
        )
        .await?;

    let mut tx = client.begin_transaction().await?;
    let affected = tx
        .execute(
            "UPDATE dbo.rfs_w9fe_rows SET name = @p1 WHERE id = @p2 AND rv = @p3",
            &[&"stale-writer", &1i32, &old_rowversion],
        )
        .await?;
    ensure!(
        affected == 0,
        "stale rowversion update affected {affected} rows"
    );
    client = tx.commit().await?;

    client
        .execute(
            "INSERT INTO dbo.rfs_w9fe_plain (id, name, quantity) VALUES (1, NULL, 1)",
            &[],
        )
        .await?;
    let original_name: Option<String> = None;
    let original_quantity = 1i32;
    concurrent
        .execute(
            "UPDATE dbo.rfs_w9fe_plain SET quantity = 2 WHERE id = 1",
            &[],
        )
        .await?;

    let mut tx = client.begin_transaction().await?;
    let affected = tx
        .execute(
            "UPDATE dbo.rfs_w9fe_plain SET name = @p1 \
             WHERE id = @p2 \
               AND ((name = @p3) OR (name IS NULL AND @p3 IS NULL)) \
               AND quantity = @p4",
            &[&"stale-writer", &1i32, &original_name, &original_quantity],
        )
        .await?;
    ensure!(
        affected == 0,
        "stale all-original-value update affected {affected} rows"
    );
    client = tx.commit().await?;
    concurrent.close().await?;

    pass("rowversion and NULL-safe all-original-value predicates reject stale writers");
    Ok(client)
}

async fn prove_timeout_and_reuse(client: &mut Client<Ready>) -> Result<()> {
    let started = Instant::now();
    let result = client
        .query_with_timeout(
            "WAITFOR DELAY '00:00:10'; SELECT 1",
            &[],
            Duration::from_millis(500),
        )
        .await;
    match result {
        Err(MssqlError::CommandTimeout) => {}
        Err(other) => bail!("expected CommandTimeout, got {other:?}"),
        Ok(_) => bail!("timed query unexpectedly succeeded"),
    }
    ensure!(
        started.elapsed() < Duration::from_secs(5),
        "command timeout did not interrupt promptly"
    );
    ensure!(
        scalar_i32(client, "SELECT 42", &[]).await? == 42,
        "connection unusable after timeout"
    );
    pass("command timeout sends Attention, returns a typed error, and preserves connection reuse");
    Ok(())
}

async fn prove_explicit_cancellation(
    mut client: Client<Ready>,
    password: &str,
) -> Result<Client<Ready>> {
    let spid = scalar_i32(&mut client, "SELECT CAST(@@SPID AS INT)", &[]).await?;
    let mut observer = Client::connect(config(password, true)?).await?;
    let canceller = client.cancel_handle();

    let watcher = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        let running = scalar_i32(
            &mut observer,
            "SELECT COUNT(*) FROM sys.dm_exec_requests \
             WHERE session_id = @p1 AND command = 'WAITFOR'",
            &[&spid],
        )
        .await?;
        canceller
            .cancel()
            .await
            .context("send Attention cancellation")?;
        Ok::<_, anyhow::Error>((observer, running))
    });

    let started = Instant::now();
    let result = client.query("WAITFOR DELAY '00:00:30'", &[]).await;
    match result {
        Err(MssqlError::Cancelled) => {}
        Err(other) => bail!("expected Cancelled, got {other:?}"),
        Ok(_) => bail!("cancelled query unexpectedly succeeded"),
    }
    ensure!(
        started.elapsed() < Duration::from_secs(10),
        "explicit cancellation did not interrupt promptly"
    );

    let (mut observer, running_before) = watcher.await.context("join cancellation observer")??;
    ensure!(
        running_before == 1,
        "observer did not see the server-side WAITFOR request"
    );
    let running_after = scalar_i32(
        &mut observer,
        "SELECT COUNT(*) FROM sys.dm_exec_requests WHERE session_id = @p1 AND command = 'WAITFOR'",
        &[&spid],
    )
    .await?;
    ensure!(
        running_after == 0,
        "cancelled WAITFOR remained active server-side"
    );
    ensure!(
        scalar_i32(&mut client, "SELECT 42", &[]).await? == 42,
        "connection unusable after explicit cancel"
    );
    observer.close().await?;

    pass("explicit cancellation terminates server work and drains the connection for reuse");
    Ok(client)
}

async fn prove_server_error_classification(client: &mut Client<Ready>) -> Result<()> {
    match client
        .query("SELECT * FROM dbo.rfs_w9fe_missing_table", &[])
        .await
    {
        Err(MssqlError::Server { number: 208, .. }) => {
            pass("missing objects produce typed SQL Server error 208 without text matching");
            Ok(())
        }
        Err(other) => bail!("missing object produced the wrong error category: {other:?}"),
        Ok(_) => bail!("missing object query unexpectedly succeeded"),
    }
}

async fn scalar_i32(
    client: &mut Client<Ready>,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<i32> {
    let mut rows = client.query(sql, params).await?;
    let row = rows
        .next()
        .transpose()?
        .context("scalar integer query returned no row")?;
    row.get(0).map_err(Into::into)
}

async fn scalar_string(
    client: &mut Client<Ready>,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<String> {
    let mut rows = client.query(sql, params).await?;
    let row = rows
        .next()
        .transpose()?
        .context("scalar string query returned no row")?;
    row.get(0).map_err(Into::into)
}

async fn scalar_bytes(
    client: &mut Client<Ready>,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<u8>> {
    let mut rows = client.query(sql, params).await?;
    let row = rows
        .next()
        .transpose()?
        .context("scalar byte query returned no row")?;
    row.get(0).map_err(Into::into)
}

fn pass(message: &str) {
    println!("PASS {message}");
}
