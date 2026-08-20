#!/usr/bin/env python3
"""Compile and run the smallest rmcp 3.1.3 in-flight disconnect probe."""

from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

CARGO_TOML = """\
[package]
name = "resourcefs-disconnect-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
anyhow = "1"
rmcp = { version = "=3.1.3", features = ["client", "server"] }
serde_json = "1"
tokio = { version = "1", features = ["io-util", "macros", "rt-multi-thread", "sync", "time"] }
"""

MAIN_RS = r'''
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result};
use rmcp::{
    ClientHandler, ErrorData as McpError, ServerHandler, ServiceExt,
    model::{ClientInfo, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerInfo},
    service::{RequestContext, RoleServer},
};
use serde_json::json;
use tokio::{
    sync::{Notify, oneshot},
    time::{Duration, timeout},
};

#[derive(Debug, Clone)]
struct EmptyClient;

impl ClientHandler for EmptyClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info
    }
}

#[derive(Debug, Clone)]
struct GatedServer {
    started: Arc<Notify>,
    release: Arc<Notify>,
    finished: Arc<AtomicBool>,
    request_cancelled: Arc<AtomicBool>,
    finished_tx: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl ServerHandler for GatedServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::default().with_protocol_version(ProtocolVersion::V_2026_07_28)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let started = self.started.clone();
        let release = self.release.clone();
        let finished = self.finished.clone();
        let request_cancelled = self.request_cancelled.clone();
        let finished_tx = self.finished_tx.clone();
        async move {
            started.notify_one();
            release.notified().await;
            request_cancelled.store(context.ct.is_cancelled(), Ordering::SeqCst);
            finished.store(true, Ordering::SeqCst);
            if let Some(tx) = finished_tx.lock().expect("probe mutex").take() {
                let _ = tx.send(());
            }
            Ok(ListToolsResult::default())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let finished = Arc::new(AtomicBool::new(false));
    let request_cancelled = Arc::new(AtomicBool::new(false));
    let (finished_tx, finished_rx) = oneshot::channel();
    let server_handler = GatedServer {
        started: started.clone(),
        release: release.clone(),
        finished: finished.clone(),
        request_cancelled: request_cancelled.clone(),
        finished_tx: Arc::new(std::sync::Mutex::new(Some(finished_tx))),
    };

    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_start = tokio::spawn(async move {
        server_handler
            .serve(server_transport)
            .await
            .context("start probe server")
    });
    let client = EmptyClient
        .serve(client_transport)
        .await
        .context("start probe client")?;
    let server = server_start.await.context("join server startup")??;

    let peer = client.peer().clone();
    let request = tokio::spawn(async move { peer.list_tools(None).await });
    timeout(Duration::from_secs(2), started.notified())
        .await
        .context("handler did not start")?;

    let wait_started = Instant::now();
    let server_wait = tokio::spawn(async move { server.waiting().await });
    let client_close = tokio::spawn(async move { client.cancel().await });
    let quit_reason = timeout(Duration::from_secs(10), server_wait)
        .await
        .context("server waiting did not return")?
        .context("join server waiting")??;
    let waiting_elapsed = wait_started.elapsed();
    let finished_before_waiting = finished.load(Ordering::SeqCst);

    release.notify_one();
    timeout(Duration::from_secs(2), finished_rx)
        .await
        .context("handler did not finish after release")?
        .context("handler finish signal dropped")?;
    let handler_completed_after_waiting = finished.load(Ordering::SeqCst);
    let request_cancelled_after_waiting = request_cancelled.load(Ordering::SeqCst);

    let _ = timeout(Duration::from_secs(3), client_close).await;
    let request_result = timeout(Duration::from_secs(3), request)
        .await
        .ok()
        .and_then(|join| join.ok())
        .map(|result| format!("{result:?}"))
        .unwrap_or_else(|| "request task did not return cleanly".to_owned());

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "rmcp": "3.1.3",
            "quitReason": format!("{quit_reason:?}"),
            "waitingElapsedMs": waiting_elapsed.as_millis(),
            "handlerFinishedBeforeWaitingReturned": finished_before_waiting,
            "handlerCompletedAfterWaitingReturned": handler_completed_after_waiting,
            "requestContextCancelledAfterWaiting": request_cancelled_after_waiting,
            "requestResult": request_result,
        }))?
    );
    Ok(())
}
'''


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-disconnect-probe-") as temp_dir:
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
