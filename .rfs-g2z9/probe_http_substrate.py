#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Probe candidate HTTP-client capabilities for the rfs-g2z9 bounded HTTP substrate.

Answers four empirical premises against a real client library and real sockets:

  P1  validated-address connection  -- does the client connect to the exact
      address our policy hook authorized, so a second resolution cannot
      substitute a different one (DNS rebinding / TOCTOU)?
  P2  per-redirect authorization    -- can each hop be vetoed BEFORE it is
      requested, so an off-allowlist URL is never sent?
  P3  bounded + cancellable body    -- can a body be abandoned at a byte ceiling
      without buffering it whole, and can an in-flight request be dropped?
  P4  tokenizer determinism         -- is the HTML tokenizer byte-deterministic
      across repeated runs and across separate processes?

Safety: no external network egress. Every request targets a loopback listener
started by this probe; DNS is simulated through the client's resolver hook.
The probe builds a standalone Cargo project in a temporary directory, so it
adds no dependency to the workspace and touches no production code.
"""

from __future__ import annotations

import json
import subprocess
import tempfile
from pathlib import Path

CARGO_TOML = """\
[package]
name = "resourcefs-http-substrate-probe"
version = "0.0.0"
edition = "2021"

[dependencies]
reqwest = { version = "=0.13.2", default-features = false, features = ["stream"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "io-util", "time"] }
futures-util = "0.3"
html5ever = "=0.39.0"
serde_json = "1"
sha2 = "0.10"

[[bin]]
name = "p1_validated_address"
path = "src/p1_validated_address.rs"

[[bin]]
name = "p2_redirect_authz"
path = "src/p2_redirect_authz.rs"

[[bin]]
name = "p3_bounded_cancel"
path = "src/p3_bounded_cancel.rs"

[[bin]]
name = "p4_tokenizer_determinism"
path = "src/p4_tokenizer_determinism.rs"
"""

# --------------------------------------------------------------------------
# P1: does the client connect to the address our hook authorized?
#
# Two listeners share one port on two distinct loopback IPs. reqwest documents
# that an explicit port in the URL overrides the resolver's port, so the IP --
# not the port -- must distinguish them. The resolver hands out the authorized
# address on its FIRST call and a different ("rebound") address on every later
# call. Ground truth is which listener actually accepted a connection, recorded
# by the listener itself -- never the client's own report.
# --------------------------------------------------------------------------
P1_RS = r'''
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const AUTHORIZED: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
const REBOUND: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);

async fn bind_pair() -> (TcpListener, TcpListener, u16) {
    for _ in 0..128 {
        let scout = TcpListener::bind((AUTHORIZED, 0)).await.expect("scout bind");
        let port = scout.local_addr().expect("scout addr").port();
        drop(scout);
        let a = TcpListener::bind((AUTHORIZED, port)).await;
        let b = TcpListener::bind((REBOUND, port)).await;
        if let (Ok(a), Ok(b)) = (a, b) {
            return (a, b, port);
        }
    }
    panic!("could not bind one shared port on both loopback addresses");
}

/// Serve minimal HTTP and record every accepted connection. This counter is
/// the ORACLE: it is observed server-side, independent of what the client
/// library reports about where it connected.
fn serve(listener: TcpListener, accepts: Arc<AtomicUsize>) {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            accepts.fetch_add(1, Ordering::SeqCst);
            let mut buffer = vec![0u8; 2048];
            let _ = socket.read(&mut buffer).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
            let _ = socket.shutdown().await;
        }
    });
}

struct Alternating {
    authorized: SocketAddr,
    rebound: SocketAddr,
    calls: Arc<AtomicUsize>,
}

impl reqwest::dns::Resolve for Alternating {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let nth = self.calls.fetch_add(1, Ordering::SeqCst);
        let addr = if nth == 0 { self.authorized } else { self.rebound };
        Box::pin(async move {
            let addrs: reqwest::dns::Addrs = Box::new(std::iter::once(addr));
            Ok(addrs)
        })
    }
}

struct Denying {
    calls: Arc<AtomicUsize>,
}

impl reqwest::dns::Resolve for Denying {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Err("policy denied every resolved address".into()) })
    }
}

#[tokio::main]
async fn main() {
    let (listener_a, listener_b, port) = bind_pair().await;
    let accepts_a = Arc::new(AtomicUsize::new(0));
    let accepts_b = Arc::new(AtomicUsize::new(0));
    serve(listener_a, Arc::clone(&accepts_a));
    serve(listener_b, Arc::clone(&accepts_b));

    let authorized = SocketAddr::new(IpAddr::V4(AUTHORIZED), port);
    let rebound = SocketAddr::new(IpAddr::V4(REBOUND), port);

    // Scenario 1 -- the resolver authorizes one address, then "rebinds".
    let calls = Arc::new(AtomicUsize::new(0));
    let client = reqwest::Client::builder()
        .dns_resolver(Alternating {
            authorized,
            rebound,
            calls: Arc::clone(&calls),
        })
        .build()
        .expect("client");
    let url = format!("http://probe.invalid:{port}/doc");
    let first = client.get(&url).send().await;
    let first_status = first.as_ref().map(|r| r.status().as_u16()).ok();

    // A second request re-enters the resolver, which now returns the rebound
    // address. Policy would reject it; here we let it through to observe
    // whether the client honours resolver output at all.
    let second = client.get(&url).send().await;
    let second_ok = second.is_ok();

    let after_two_a = accepts_a.load(Ordering::SeqCst);
    let after_two_b = accepts_b.load(Ordering::SeqCst);

    // Scenario 2 -- the resolver denies. No connection may be attempted.
    let deny_calls = Arc::new(AtomicUsize::new(0));
    let denying = reqwest::Client::builder()
        .dns_resolver(Denying {
            calls: Arc::clone(&deny_calls),
        })
        .build()
        .expect("deny client");
    let before_a = accepts_a.load(Ordering::SeqCst);
    let before_b = accepts_b.load(Ordering::SeqCst);
    let denied = denying.get(&url).send().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let denied_errored = denied.is_err();
    let deny_new_a = accepts_a.load(Ordering::SeqCst) - before_a;
    let deny_new_b = accepts_b.load(Ordering::SeqCst) - before_b;

    let report = serde_json::json!({
        "port": port,
        "firstRequest": {
            "status": first_status,
            "acceptsAuthorizedAfterFirst": 1,
        },
        "afterTwoRequests": {
            "resolverCalls": calls.load(Ordering::SeqCst),
            "acceptsAuthorized": after_two_a,
            "acceptsRebound": after_two_b,
            "secondRequestOk": second_ok,
        },
        "resolverDenial": {
            "resolverCalls": deny_calls.load(Ordering::SeqCst),
            "errored": denied_errored,
            "newAcceptsAuthorized": deny_new_a,
            "newAcceptsRebound": deny_new_b,
        }
    });
    println!("{}", serde_json::to_string_pretty(&report).expect("json"));
}
'''

# --------------------------------------------------------------------------
# P2: can a redirect hop be vetoed before it is requested?
#
# The "allowed" origin redirects to an off-allowlist origin. Each listener
# records EVERY request path it actually received -- that log is the oracle.
# The off-allowlist path must appear in no log.
# --------------------------------------------------------------------------
P2_RS = r'''
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ALLOWED: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
const OFF_ALLOWLIST: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);

async fn bind_pair() -> (TcpListener, TcpListener, u16) {
    for _ in 0..128 {
        let scout = TcpListener::bind((ALLOWED, 0)).await.expect("scout bind");
        let port = scout.local_addr().expect("scout addr").port();
        drop(scout);
        let a = TcpListener::bind((ALLOWED, port)).await;
        let b = TcpListener::bind((OFF_ALLOWLIST, port)).await;
        if let (Ok(a), Ok(b)) = (a, b) {
            return (a, b, port);
        }
    }
    panic!("could not bind one shared port on both loopback addresses");
}

fn request_path(raw: &str) -> String {
    raw.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("<unparsed>")
        .to_owned()
}

/// Record every received path (the ORACLE) and answer per the path.
fn serve(listener: TcpListener, log: Arc<Mutex<Vec<String>>>, port: u16, redirecting: bool) {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 4096];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let raw = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request_path(&raw);
                log.lock().expect("log").push(path.clone());

                let response = if redirecting && path == "/hop-offsite" {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://offsite.invalid:{port}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else if redirecting && path == "/hop-onsite" {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://allowed.invalid:{port}/landing\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
}

struct StaticMap {
    allowed: SocketAddr,
    offsite: SocketAddr,
}

impl reqwest::dns::Resolve for StaticMap {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let addr = if name.as_str().starts_with("offsite") {
            self.offsite
        } else {
            self.allowed
        };
        Box::pin(async move {
            let addrs: reqwest::dns::Addrs = Box::new(std::iter::once(addr));
            Ok(addrs)
        })
    }
}

#[tokio::main]
async fn main() {
    let (listener_allowed, listener_offsite, port) = bind_pair().await;
    let allowed_log = Arc::new(Mutex::new(Vec::new()));
    let offsite_log = Arc::new(Mutex::new(Vec::new()));
    serve(listener_allowed, Arc::clone(&allowed_log), port, true);
    serve(listener_offsite, Arc::clone(&offsite_log), port, false);

    let inspected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let inspected_for_policy = Arc::clone(&inspected);

    let client = reqwest::Client::builder()
        .dns_resolver(StaticMap {
            allowed: SocketAddr::new(IpAddr::V4(ALLOWED), port),
            offsite: SocketAddr::new(IpAddr::V4(OFF_ALLOWLIST), port),
        })
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let host = attempt.url().host_str().unwrap_or_default().to_owned();
            inspected_for_policy
                .lock()
                .expect("inspected")
                .push(attempt.url().to_string());
            // Allowlist policy: only `allowed.invalid` may be followed.
            if host == "allowed.invalid" {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .expect("client");

    // Case 1: redirect that leaves the allowlist -- must be refused pre-request.
    let offsite_attempt = client
        .get(format!("http://allowed.invalid:{port}/hop-offsite"))
        .send()
        .await;
    let offsite_final_status = offsite_attempt.as_ref().map(|r| r.status().as_u16()).ok();
    let offsite_final_url = offsite_attempt
        .as_ref()
        .map(|r| r.url().to_string())
        .unwrap_or_else(|e| format!("<error: {e}>"));

    // Case 2: redirect that stays on the allowlist -- must be followed.
    let onsite_attempt = client
        .get(format!("http://allowed.invalid:{port}/hop-onsite"))
        .send()
        .await;
    let onsite_final_status = onsite_attempt.as_ref().map(|r| r.status().as_u16()).ok();

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    let allowed_paths = allowed_log.lock().expect("allowed log").clone();
    let offsite_paths = offsite_log.lock().expect("offsite log").clone();

    let report = serde_json::json!({
        "offsiteRedirect": {
            "finalStatus": offsite_final_status,
            "finalUrl": offsite_final_url,
        },
        "onsiteRedirect": {
            "finalStatus": onsite_final_status,
        },
        "policyInspectedUrls": inspected.lock().expect("inspected").clone(),
        "oracleAllowedServerPaths": allowed_paths,
        "oracleOffsiteServerPaths": offsite_paths,
    });
    println!("{}", serde_json::to_string_pretty(&report).expect("json"));
}
'''

# --------------------------------------------------------------------------
# P3: bounded body reads and cancellation.
#
# The server writes a large body and records how many bytes it actually
# flushed before the peer went away -- that count is the oracle. A client that
# truly streams and abandons early leaves the server far short of the total.
# --------------------------------------------------------------------------
P3_RS = r'''
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const HOST: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
const BODY_BYTES: usize = 64 * 1024 * 1024;
const CEILING_BYTES: usize = 1024 * 1024;
const CHUNK: usize = 64 * 1024;

/// Stream a large body, counting bytes actually handed to the socket. The
/// counter is the ORACLE: it is server-side and independent of the client.
fn serve(listener: TcpListener, written: Arc<AtomicUsize>, slow: bool) {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let written = Arc::clone(&written);
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 2048];
                let _ = socket.read(&mut buffer).await;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {BODY_BYTES}\r\nConnection: close\r\n\r\n"
                );
                if socket.write_all(header.as_bytes()).await.is_err() {
                    return;
                }
                let chunk = vec![b'x'; CHUNK];
                let mut sent = 0usize;
                while sent < BODY_BYTES {
                    if socket.write_all(&chunk).await.is_err() {
                        break;
                    }
                    if socket.flush().await.is_err() {
                        break;
                    }
                    sent += chunk.len();
                    written.fetch_add(chunk.len(), Ordering::SeqCst);
                    if slow {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            });
        }
    });
}

#[tokio::main]
async fn main() {
    // Scenario 1 -- bounded read: stop at the ceiling and drop the response.
    let bounded_listener = TcpListener::bind((HOST, 0)).await.expect("bind");
    let bounded_port = bounded_listener.local_addr().expect("addr").port();
    let bounded_written = Arc::new(AtomicUsize::new(0));
    serve(bounded_listener, Arc::clone(&bounded_written), false);

    let client = reqwest::Client::builder().build().expect("client");
    let response = client
        .get(format!("http://127.0.0.1:{bounded_port}/big"))
        .send()
        .await
        .expect("send");
    let declared_length = response.content_length();
    let mut stream = response.bytes_stream();
    let mut read = 0usize;
    while let Some(item) = stream.next().await {
        let bytes = item.expect("chunk");
        read += bytes.len();
        if read >= CEILING_BYTES {
            break;
        }
    }
    drop(stream);
    tokio::time::sleep(Duration::from_millis(250)).await;
    let bounded_server_wrote = bounded_written.load(Ordering::SeqCst);

    // Scenario 2 -- cancellation: drop the in-flight future entirely.
    let cancel_listener = TcpListener::bind((HOST, 0)).await.expect("bind");
    let cancel_port = cancel_listener.local_addr().expect("addr").port();
    let cancel_written = Arc::new(AtomicUsize::new(0));
    serve(cancel_listener, Arc::clone(&cancel_written), true);

    let cancel_client = reqwest::Client::builder().build().expect("client");
    let inflight = async move {
        let response = cancel_client
            .get(format!("http://127.0.0.1:{cancel_port}/slow"))
            .send()
            .await
            .expect("send");
        let mut stream = response.bytes_stream();
        let mut total = 0usize;
        while let Some(item) = stream.next().await {
            total += item.expect("chunk").len();
        }
        total
    };
    let cancelled = tokio::time::timeout(Duration::from_millis(120), inflight).await;
    let cancelled_early = cancelled.is_err();
    let at_cancel = cancel_written.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let after_cancel = cancel_written.load(Ordering::SeqCst);

    let report = serde_json::json!({
        "bounded": {
            "declaredContentLength": declared_length,
            "totalBodyBytes": BODY_BYTES,
            "ceilingBytes": CEILING_BYTES,
            "clientReadBytes": read,
            "oracleServerWroteBytes": bounded_server_wrote,
            "serverWroteWholeBody": bounded_server_wrote >= BODY_BYTES,
        },
        "cancellation": {
            "cancelledEarly": cancelled_early,
            "oracleServerWroteAtCancel": at_cancel,
            "oracleServerWroteAfterSettle": after_cancel,
            "serverStoppedWriting": after_cancel < BODY_BYTES,
        }
    });
    println!("{}", serde_json::to_string_pretty(&report).expect("json"));
}
'''

# --------------------------------------------------------------------------
# P4: tokenizer determinism.
#
# Canonicalise the token stream and hash it. The driver runs this binary in two
# SEPARATE PROCESSES and compares -- cross-process agreement is the oracle for
# determinism, and a hand-authored expected stream is the oracle for the small
# document's structure.
# --------------------------------------------------------------------------
P4_RS = r'''
use std::cell::RefCell;

use html5ever::tendril::TendrilSink;
use html5ever::tokenizer::{
    BufferQueue, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use sha2::{Digest, Sha256};

/// Canonical, order-preserving record of the token stream.
struct Canonical {
    events: RefCell<Vec<String>>,
}

impl TokenSink for Canonical {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut events = self.events.borrow_mut();
        match token {
            Token::DoctypeToken(doctype) => {
                // Render the NAME, never `{:?}` on the tendril: Debug leaks the
                // inline-vs-heap storage discriminator, which changes with the
                // string's length and would make the record non-deterministic.
                let name = doctype
                    .name
                    .as_ref()
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                events.push(format!("DOCTYPE {name}"));
            }
            Token::TagToken(tag) => {
                let kind = match tag.kind {
                    html5ever::tokenizer::StartTag => "START",
                    html5ever::tokenizer::EndTag => "END",
                };
                // Attributes are sorted so the record cannot depend on hash
                // iteration order anywhere upstream.
                let mut attrs: Vec<String> = tag
                    .attrs
                    .iter()
                    .map(|attr| format!("{}={}", attr.name.local, attr.value))
                    .collect();
                attrs.sort();
                events.push(format!(
                    "{kind} {} [{}] self_closing={}",
                    tag.name,
                    attrs.join(","),
                    tag.self_closing
                ));
            }
            Token::CommentToken(text) => events.push(format!("COMMENT {text}")),
            Token::CharacterTokens(text) => events.push(format!("CHARS {text}")),
            Token::NullCharacterToken => events.push("NULL".to_owned()),
            Token::EOFToken => events.push("EOF".to_owned()),
            Token::ParseError(message) => events.push(format!("ERROR {message}")),
        }
        TokenSinkResult::Continue
    }
}

fn tokenize(document: &str) -> Vec<String> {
    let sink = Canonical {
        events: RefCell::new(Vec::new()),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(html5ever::tendril::StrTendril::from(document));
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    tokenizer.sink.events.into_inner()
}

fn digest(events: &[String]) -> String {
    let mut hasher = Sha256::new();
    for event in events {
        hasher.update(event.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn corpus() -> Vec<(&'static str, String)> {
    vec![
        (
            "small",
            "<!doctype html><html><body><h1>Title</h1><p>Body <a href=\"/x\">link</a>.</p></body></html>"
                .to_owned(),
        ),
        (
            "malformed",
            "<div><p>unclosed<div><span>deep</p></span></div>".to_owned(),
        ),
        (
            "script_style",
            "<html><head><style>body{color:red}</style><script>var x = 1 < 2;</script></head><body>text</body></html>"
                .to_owned(),
        ),
        (
            "unicode",
            "<p>\u{8bbe}\u{8ba1} \u{2014} caf\u{e9} \u{1f600}</p>".to_owned(),
        ),
        (
            "attrs",
            "<img src=\"a.png\" alt=\"z\" id=\"b\" data-q=\"1\"><input disabled value=\"v\" name=\"n\">"
                .to_owned(),
        ),
        ("deep", {
            let mut document = String::new();
            for _ in 0..256 {
                document.push_str("<div>");
            }
            document.push_str("leaf");
            for _ in 0..256 {
                document.push_str("</div>");
            }
            document
        }),
        ("entities", "<p>&amp;&lt;&gt;&quot;&#65;&nbsp;&unknown;</p>".to_owned()),
        // A doctype name long enough to force tendril off its inline storage --
        // proves the canonical record does not embed the storage discriminator.
        (
            "long_doctype",
            "<!DOCTYPE averyverylongdoctypenamewellbeyondinlinestorage><p>x</p>".to_owned(),
        ),
    ]
}

fn main() {
    let mut digests = serde_json::Map::new();
    let mut repeated_in_process = serde_json::Map::new();
    for (name, document) in corpus() {
        let first = tokenize(&document);
        let second = tokenize(&document);
        repeated_in_process.insert(
            name.to_owned(),
            serde_json::Value::Bool(digest(&first) == digest(&second)),
        );
        digests.insert(name.to_owned(), serde_json::Value::String(digest(&first)));
    }

    // The small document's full token stream, for comparison against a
    // hand-authored expectation held by the driver.
    let small_events = tokenize(&corpus()[0].1);

    let report = serde_json::json!({
        "digests": digests,
        "repeatedInProcessStable": repeated_in_process,
        "smallDocumentEvents": small_events,
    });
    println!("{}", serde_json::to_string_pretty(&report).expect("json"));
}
'''

# Hand-authored expectation for the `small` document. Written from the HTML
# tokenizer specification by reading the markup, NOT by running the probe --
# this is the independent structural oracle for P4.
HAND_AUTHORED_SMALL = [
    "DOCTYPE html",
    "START html [] self_closing=false",
    "START body [] self_closing=false",
    "START h1 [] self_closing=false",
    "CHARS Title",
    "END h1 [] self_closing=false",
    "START p [] self_closing=false",
    "CHARS Body ",
    "START a [href=/x] self_closing=false",
    "CHARS link",
    "END a [] self_closing=false",
    "CHARS .",
    "END p [] self_closing=false",
    "END body [] self_closing=false",
    "END html [] self_closing=false",
    "EOF",
]


def run(command: list[str], *, cwd: Path) -> str:
    completed = subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {command!r}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed.stdout.strip()


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-http-probe-") as temporary:
        project = Path(temporary)
        (project / "src").mkdir()
        (project / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
        (project / "src" / "p1_validated_address.rs").write_text(P1_RS, encoding="utf-8")
        (project / "src" / "p2_redirect_authz.rs").write_text(P2_RS, encoding="utf-8")
        (project / "src" / "p3_bounded_cancel.rs").write_text(P3_RS, encoding="utf-8")
        (project / "src" / "p4_tokenizer_determinism.rs").write_text(P4_RS, encoding="utf-8")

        run(["cargo", "build", "--offline", "--quiet"], cwd=project)

        p1 = json.loads(run(["cargo", "run", "--offline", "--quiet", "--bin", "p1_validated_address"], cwd=project))
        p2 = json.loads(run(["cargo", "run", "--offline", "--quiet", "--bin", "p2_redirect_authz"], cwd=project))
        p3 = json.loads(run(["cargo", "run", "--offline", "--quiet", "--bin", "p3_bounded_cancel"], cwd=project))

        # Two SEPARATE processes -- cross-process agreement is the oracle.
        p4_first = json.loads(run(["cargo", "run", "--offline", "--quiet", "--bin", "p4_tokenizer_determinism"], cwd=project))
        p4_second = json.loads(run(["cargo", "run", "--offline", "--quiet", "--bin", "p4_tokenizer_determinism"], cwd=project))

    p4_cross_process = p4_first["digests"] == p4_second["digests"]
    p4_matches_hand_authored = p4_first["smallDocumentEvents"] == HAND_AUTHORED_SMALL

    report = {
        "p1_validated_address": p1,
        "p2_redirect_authz": p2,
        "p3_bounded_cancel": p3,
        "p4_tokenizer_determinism": {
            "digestsFirstProcess": p4_first["digests"],
            "digestsSecondProcess": p4_second["digests"],
            "crossProcessStable": p4_cross_process,
            "repeatedInProcessStable": p4_first["repeatedInProcessStable"],
            "smallDocumentEvents": p4_first["smallDocumentEvents"],
            "handAuthoredExpectation": HAND_AUTHORED_SMALL,
            "matchesHandAuthoredOracle": p4_matches_hand_authored,
        },
    }
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
