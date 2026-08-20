#!/usr/bin/env python3
"""Compile and run the smallest rmcp 3.1.3 client-roots lifecycle probe."""

from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

CARGO_TOML = """\
[package]
name = "resourcefs-roots-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
anyhow = "1"
rmcp = { version = "=3.1.3", features = ["client", "server"] }
serde_json = "1"
tokio = { version = "1", features = ["io-util", "macros", "rt-multi-thread", "sync", "time"] }
"""

MAIN_RS = r'''
#![allow(deprecated)]

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use anyhow::{Context, Result};
use rmcp::{
    ClientHandler, ErrorData as McpError, Peer, ServerHandler, ServiceExt,
    model::{
        ClientCapabilities, ClientInfo, ListRootsResult, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, Root, ServerInfo,
    },
    service::{NotificationContext, RequestContext, RoleClient, RoleServer},
};
use serde_json::json;
use tokio::{sync::{RwLock, mpsc}, time::{Duration, timeout}};

type Observation = (String, bool, Vec<String>, bool);

#[derive(Debug, Clone)]
struct RootsClient {
    roots: Arc<RwLock<Vec<Root>>>,
    list_calls: Arc<AtomicUsize>,
}

impl ClientHandler for RootsClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info.capabilities = ClientCapabilities::builder()
            .enable_roots()
            .enable_roots_list_changed()
            .build();
        info
    }

    async fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, McpError> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        Ok(ListRootsResult::new(self.roots.read().await.clone()))
    }
}

#[derive(Debug, Clone)]
struct RootsServer {
    observations: mpsc::UnboundedSender<Observation>,
    request_events: Arc<AtomicUsize>,
}

impl RootsServer {
    async fn observe(
        observations: mpsc::UnboundedSender<Observation>,
        event: String,
        peer: Peer<RoleServer>,
    ) {
        let advertised = peer
            .peer_info()
            .is_some_and(|info| info.capabilities.roots.is_some());
        let (roots, association_rejected) = match peer.list_roots().await {
            Ok(result) => (
                result.roots.into_iter().map(|root| root.uri).collect(),
                false,
            ),
            Err(error) => (
                Vec::new(),
                error.to_string().contains("SEP-2260"),
            ),
        };
        observations
            .send((event, advertised, roots, association_rejected))
            .expect("probe receiver remains live");
    }
}

impl ServerHandler for RootsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::default().with_protocol_version(ProtocolVersion::V_2026_07_28)
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let observations = self.observations.clone();
        async move {
            Self::observe(
                observations,
                "initialized-notification".to_owned(),
                context.peer,
            )
            .await
        }
    }

    fn on_roots_list_changed(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let observations = self.observations.clone();
        async move {
            Self::observe(
                observations,
                "changed-notification".to_owned(),
                context.peer,
            )
            .await
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let observations = self.observations.clone();
        let request_index = self.request_events.fetch_add(1, Ordering::SeqCst) + 1;
        async move {
            Self::observe(
                observations,
                format!("request-{request_index}"),
                context.peer,
            )
            .await;
            Ok(ListToolsResult::default())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let roots = Arc::new(RwLock::new(vec![
        Root::new("file:///workspace/alpha").with_name("alpha"),
    ]));
    let list_calls = Arc::new(AtomicUsize::new(0));
    let client_handler = RootsClient {
        roots: roots.clone(),
        list_calls: list_calls.clone(),
    };
    let (observations_tx, mut observations_rx) = mpsc::unbounded_channel();
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        RootsServer {
            observations: observations_tx,
            request_events: Arc::new(AtomicUsize::new(0)),
        }
        .serve(server_transport)
        .await
        .context("start roots probe server")
    });
    let client = client_handler
        .serve(client_transport)
        .await
        .context("start roots probe client")?;
    let server = server_task.await.context("join server startup")??;

    let initial_notification = timeout(Duration::from_secs(2), observations_rx.recv())
        .await
        .context("wait for initialized notification observation")?
        .context("initialized notification observation channel")?;

    client
        .list_tools(None)
        .await
        .context("request tools/list after initialization")?;
    let initial_request = timeout(Duration::from_secs(2), observations_rx.recv())
        .await
        .context("wait for initial request observation")?
        .context("initial request observation channel")?;

    *roots.write().await = vec![
        Root::new("file:///workspace/beta").with_name("beta"),
        Root::new("file:///workspace/gamma"),
    ];
    client
        .notify_roots_list_changed()
        .await
        .context("send roots/list_changed")?;
    let changed_notification = timeout(Duration::from_secs(2), observations_rx.recv())
        .await
        .context("wait for changed notification observation")?
        .context("changed notification observation channel")?;

    client
        .list_tools(None)
        .await
        .context("request tools/list after roots change")?;
    let changed_request = timeout(Duration::from_secs(2), observations_rx.recv())
        .await
        .context("wait for changed request observation")?
        .context("changed request observation channel")?;

    let observation = |value: Observation| {
        json!({
            "event": value.0,
            "clientAdvertisedRoots": value.1,
            "roots": value.2,
            "associationRejected": value.3,
        })
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "rmcp": "3.1.3",
            "protocolVersion": "2026-07-28",
            "initializedNotification": observation(initial_notification),
            "initialRequest": observation(initial_request),
            "changedNotification": observation(changed_notification),
            "changedRequest": observation(changed_request),
            "listRootsCalls": list_calls.load(Ordering::SeqCst),
        }))?
    );

    client.cancel().await.context("cancel client")?;
    server.cancel().await.context("cancel server")?;
    Ok(())
}
'''


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-roots-probe-") as temp_dir:
        root = Path(temp_dir)
        (root / "src").mkdir()
        (root / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
        (root / "src" / "main.rs").write_text(MAIN_RS, encoding="utf-8")
        completed = subprocess.run(
            ["cargo", "run", "--quiet", "--manifest-path", str(root / "Cargo.toml")],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
        )
        print(completed.stdout, end="")


if __name__ == "__main__":
    main()
