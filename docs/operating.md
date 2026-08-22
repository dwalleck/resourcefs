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
