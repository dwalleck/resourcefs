#!/usr/bin/env python3
"""Static-source and independent-path oracle for the rfs-cgbq probes."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path, PureWindowsPath


def one(paths: list[Path], description: str) -> Path:
    if len(paths) != 1:
        raise RuntimeError(f"expected one {description}, found {len(paths)}: {paths}")
    return paths[0]


def rmcp_oracle() -> dict[str, object]:
    registry = Path.home() / ".cargo" / "registry" / "src"
    source = one(list(registry.glob("*/rmcp-3.1.3/src")), "rmcp 3.1.3 source")
    client_handler = (source / "handler" / "client.rs").read_text(encoding="utf-8")
    server_handler = (source / "handler" / "server.rs").read_text(encoding="utf-8")
    service_server = (source / "service" / "server.rs").read_text(encoding="utf-8")
    service = (source / "service.rs").read_text(encoding="utf-8")
    capabilities = (source / "model" / "capabilities.rs").read_text(encoding="utf-8")
    model = (source / "model.rs").read_text(encoding="utf-8")

    return {
        "clientDispatchesListRoots": (
            "ServerRequest::ListRootsRequest(_)" in client_handler
            and ".list_roots(context)" in client_handler
        ),
        "serverPeerExposesListRoots": "peer_req list_roots ListRootsRequest()" in service_server,
        "serverDispatchesRootsChanged": (
            "ClientNotification::RootsListChangedNotification" in server_handler
            and "self.on_roots_list_changed(context).await" in server_handler
        ),
        "notificationContextCarriesPeer": (
            "pub struct NotificationContext" in service and "pub peer: Peer<R>" in service
        ),
        "rootsCapabilityExists": "pub roots: Option<RootsCapabilities>" in capabilities,
        "rootsAreDeprecated": (
            "Roots is deprecated by SEP-2577" in capabilities
            and "Roots is deprecated by SEP-2577" in model
        ),
        "listRootsRestrictedBySep2260": (
            "ServerRequest::ListRootsRequest(_)" in service_server
            and "SEP-2260: server-to-client requests must be associated" in service_server
        ),
        "requestHandlersCarryOriginatingScope": (
            "ORIGINATING_REQUEST" in service
            and ".scope(handler_id, service.handle_request(request, context))" in service
        ),
        "notificationHandlersLackOriginatingScope": (
            "service.handle_notification(notification, context).await" in service
        ),
        "expectedProtocolVersion": "2026-07-28",
        "expectedLifecycle": [
            ["initialized-notification", True, []],
            ["request-1", False, ["file:///workspace/alpha"]],
            ["changed-notification", True, []],
            [
                "request-2",
                False,
                ["file:///workspace/beta", "file:///workspace/gamma"],
            ],
        ],
        "expectedListRootsCalls": 2,
    }


def windows_oracle() -> dict[str, object]:
    sysroot = Path(
        subprocess.run(
            ["rustc", "--print", "sysroot"],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
        ).stdout.strip()
    )
    path_source = (
        sysroot / "lib" / "rustlib" / "src" / "rust" / "library" / "std" / "src" / "sys" / "path" / "windows.rs"
    ).read_text(encoding="utf-8")
    fs_source = (
        sysroot / "lib" / "rustlib" / "src" / "rust" / "library" / "std" / "src" / "sys" / "fs" / "windows.rs"
    ).read_text(encoding="utf-8")
    canonicalize = fs_source.split("pub fn canonicalize", 1)[1].split("pub fn copy", 1)[0]

    forms = {
        "driveAbsolute": r"C:\workspace\file.txt",
        "uncAbsolute": r"\\server\share\file.txt",
        "driveRelative": r"C:workspace\file.txt",
        "rootRelative": r"\workspace\file.txt",
        "slashRooted": r"/workspace/file.txt",
    }
    classifications = {
        name: PureWindowsPath(value).is_absolute() for name, value in forms.items()
    }

    with tempfile.TemporaryDirectory(prefix="resourcefs-windows-oracle-") as temp_dir:
        fixture = Path(temp_dir)
        root = fixture / "root"
        outside = fixture / "outside"
        root.mkdir()
        outside.mkdir()
        (outside / "secret.txt").write_text("outside\n", encoding="utf-8")
        (root / "escape").symlink_to(outside, target_is_directory=True)
        canonical_root = Path(os.path.realpath(root))
        canonical_escape = Path(os.path.realpath(root / "escape" / "secret.txt"))
        escaping_link_is_outside = not canonical_escape.is_relative_to(canonical_root)

    return {
        "lexicalClassifications": classifications,
        "rustRequiresRootAndPrefix": (
            "path.has_root() && path.prefix().is_some()" in path_source
        ),
        "rustCanonicalizeUsesFinalHandlePath": (
            "GetFinalPathNameByHandleW" in fs_source
            and "get_path(f.handle)" in canonicalize
        ),
        "rustCanonicalizeFollowsReparsePoint": (
            "FILE_FLAG_BACKUP_SEMANTICS" in canonicalize
            and "FILE_FLAG_OPEN_REPARSE_POINT" not in canonicalize
        ),
        "escapingLinkResolvesOutside": escaping_link_is_outside,
    }


def main() -> None:
    print(
        json.dumps(
            {
                "rmcp": rmcp_oracle(),
                "windows": windows_oracle(),
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
