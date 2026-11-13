//! (round five) The platform's public file addresses (BD-64) as the fake networks meet them. [`start`] serves the
//! platform's own route (`{PUBLIC_URL}/files/pull/{id}?s=`) on 127.0.0.1, `PUBLIC_URL` being its address: the address is
//! checked by the kernel (`pulled_file`: its signature, not ended, not past its time), the bytes read from the store, a
//! `Range` answered with 206. Before a network's fake answers a request, every address in it is pulled as a network
//! does (a range first, then the whole) and kept in [`PULLED`], so a fake or a test can check what arrived.

use ferrum_kernel::Kernel;
use ferrum_kernel::files::FileStore;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

pub static KERNEL: OnceLock<Arc<Kernel>> = OnceLock::new();
pub static FILES: OnceLock<Arc<dyn FileStore>> = OnceLock::new();
/// Every address pulled: `{url, status, bytes, range}` (`range`: the first four bytes as a 206 gave them).
pub static PULLED: Mutex<Vec<Value>> = Mutex::new(Vec::new());

pub const PULL: &str = "/files/pull/";
/// Where the route is served: the server's `PUBLIC_URL`.
pub static BASE: OnceLock<String> = OnceLock::new();

/// Serves the route, and gives its address (the server's `PUBLIC_URL`).
pub async fn start() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let _ = BASE.set(base.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let mut lines = head.lines();
                let target = lines.next().unwrap_or_default().split_whitespace().nth(1).unwrap_or("/").to_string();
                let headers: HashMap<String, String> =
                    lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
                let (code, extra, body) = serve_pull(&target, &headers).await.unwrap_or((404, vec![], vec![]));
                let mut out = format!("HTTP/1.1 {code} X\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
                for (k, v) in extra {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                let mut bytes = out.into_bytes();
                bytes.extend(body);
                let _ = s.write_all(&bytes).await;
            });
        }
    });
    base
}

/// The platform's route for an address, when `target` is one: (status, headers, body).
pub async fn serve_pull(target: &str, headers: &HashMap<String, String>) -> Option<(u16, Vec<(String, String)>, Vec<u8>)> {
    let rest = target.strip_prefix(PULL)?;
    let (id, q) = rest.split_once("?s=").unwrap_or((rest, ""));
    let gone = Some((404, vec![], b"{\"error\": \"not found\"}".to_vec()));
    let (Ok(id), Some(k)) = (id.parse::<Uuid>(), KERNEL.get()) else { return gone };
    let Ok(p) = k.pulled_file(id, q).await else { return gone };
    let bytes = FILES.get()?.get(&p.key).await.ok()?;
    let ct = vec![("content-type".to_string(), p.content_type.clone())];
    match headers.get("range").and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.split_once('-')) {
        Some((a, b)) => {
            let a: usize = a.parse().unwrap_or(0);
            let b: usize = b.parse::<usize>().map_or(bytes.len().saturating_sub(1), |b| b.min(bytes.len().saturating_sub(1)));
            let mut h = ct;
            h.push(("content-range".into(), format!("bytes {a}-{b}/{}", bytes.len())));
            Some((206, h, bytes.get(a..=b).unwrap_or_default().to_vec()))
        }
        None => Some((200, ct, bytes)),
    }
}

/// Pulls every address in `text` (a request's target and body), as a network does.
pub async fn pull_all(text: &str) {
    let Some(base) = BASE.get() else { return };
    // As written, or percent-encoded in a query or a form.
    let text = &format!("{text} {}", plain(text));
    let find = format!("{base}{PULL}");
    let mut at = 0;
    while let Some(i) = text[at..].find(&find) {
        let start = at + i;
        let tail = &text[start..];
        let end = tail.find(|c: char| !(c.is_ascii_alphanumeric() || "/-?=:.".contains(c))).unwrap_or(tail.len());
        let path = &tail[..end];
        at = start + end;
        let client = reqwest::Client::new();
        let url = path.to_string();
        let head = client.get(&url).header("range", "bytes=0-3").send().await;
        let range = match head {
            Ok(r) if r.status().as_u16() == 206 => r.bytes().await.map(|b| b.to_vec()).unwrap_or_default(),
            _ => vec![],
        };
        let (status, bytes) = match client.get(&url).send().await {
            Ok(r) => (r.status().as_u16(), r.bytes().await.map(|b| b.to_vec()).unwrap_or_default()),
            Err(_) => (0, vec![]),
        };
        PULLED.lock().unwrap().push(json!({"url": url, "status": status, "bytes": bytes, "range": range}));
    }
}

/// What a network pulled from the address `url`: its bytes, once the address answered and its first four bytes came as
/// a range of the same file.
pub fn pulled(url: &str) -> Option<Vec<u8>> {
    let all = PULLED.lock().unwrap();
    let p = all.iter().rev().find(|p| p["url"] == url)?;
    let bytes: Vec<u8> = serde_json::from_value(p["bytes"].clone()).ok()?;
    let range: Vec<u8> = serde_json::from_value(p["range"].clone()).ok()?;
    (p["status"] == 200 && !bytes.is_empty() && range[..] == bytes[..bytes.len().min(4)]).then_some(bytes)
}

/// Whether `url` is an address this server made.
pub fn is_address(url: &str) -> bool {
    url.contains(PULL) && url.contains("?s=")
}

/// `s` percent-decoded.
fn plain(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match s.get(i + 1..i + 3).filter(|_| b[i] == b'%').and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
