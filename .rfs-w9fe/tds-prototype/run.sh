#!/usr/bin/env bash
set -euo pipefail

readonly container_name="rfs-w9fe-sqlserver"
readonly password="Rfs_w9fe!Prototype42"
readonly image="mcr.microsoft.com/mssql/server@sha256:bf438d7104f861f5e1e1ba14b063d60a5bd883964b3c9b3b3c0064536177baa5"
readonly sqlcmd="/opt/mssql-tools18/bin/sqlcmd"

cleanup() {
  if ! docker rm --force "${container_name}" >/dev/null 2>&1; then
    printf 'warning: failed to remove prototype container %s\n' "${container_name}" >&2
  fi
}
trap cleanup EXIT

docker run --detach --rm \
  --name "${container_name}" \
  --env ACCEPT_EULA=Y \
  --env "MSSQL_SA_PASSWORD=${password}" \
  --publish 14339:1433 \
  "${image}" >/dev/null

ready=false
for _ in $(seq 1 60); do
  if docker exec \
    --env "SQLCMDPASSWORD=${password}" \
    "${container_name}" \
    "${sqlcmd}" -S localhost -U sa -C -d tempdb -b -Q "SELECT 1" \
    >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "${ready}" != true ]]; then
  printf 'SQL Server prototype fixture did not become ready\n' >&2
  exit 1
fi

RFS_W9FE_SQL_PASSWORD="${password}" \
  cargo run --manifest-path "$(dirname "$0")/Cargo.toml"

docker exec \
  --env "SQLCMDPASSWORD=${password}" \
  "${container_name}" \
  "${sqlcmd}" -S localhost -U sa -C -d tempdb -b -V 11 -h -1 -W \
  -Q "SET NOCOUNT ON; SELECT CAST(SERVERPROPERTY('ProductVersion') AS NVARCHAR(128));"
