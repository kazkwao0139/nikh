//! 로컬 웹 서버 — ui/(index.html·app.css·js/) + /api/search·/api/loaded. 127.0.0.1 전용.
use crate::data::{load, resolve};
use anyhow::Result;
use std::io::{Read, Write};
use std::path::Path;

pub(crate) fn serve(port: u16) -> Result<()> {
    let mut eng = load()?;
    let ui_dir = std::env::var("NIKE_UI").unwrap_or_else(|_| "ui".into());
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("NIKH → http://localhost:{port}/");
    for stream in listener.incoming() {
        let mut s = stream?;
        let mut buf = [0u8; 8192];
        let n = s.read(&mut buf)?;
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
        let (ctype, body): (&str, Vec<u8>) = if path.starts_with("/api/loaded") {
            let keys: Vec<String> =
                resolve().map(|(p, _, _)| p.iter().filter_map(|d| d.file_name().map(|n| n.to_string_lossy().to_string())).collect()).unwrap_or_default();
            ("application/json; charset=utf-8", serde_json::to_vec(&serde_json::json!({"loaded": keys}))?)
        } else if path.starts_with("/api/search") {
            let qs: std::collections::HashMap<String, String> = path
                .splitn(2, '?')
                .nth(1)
                .unwrap_or("")
                .split('&')
                .filter_map(|kv| {
                    let mut it = kv.splitn(2, '=');
                    Some((it.next()?.to_string(), urldecode(it.next().unwrap_or(""))))
                })
                .collect();
            let q = qs.get("q").cloned().unwrap_or_default();
            let k = qs.get("k").and_then(|x| x.parse().ok()).unwrap_or(20);
            let flt = qs.get("filter").and_then(|f| f.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())));
            let mode = qs.get("mode").cloned();
            let hits = if q.trim().is_empty() { vec![] } else { eng.search_mode(&q, k, flt.as_ref().map(|(a, b)| (a.as_str(), b.as_str())), mode.as_deref())? };
            ("application/json; charset=utf-8", serde_json::to_vec(&serde_json::json!({"rows": hits}))?)
        } else {
            let f = if path == "/" { "index.html".to_string() } else { path.trim_start_matches('/').to_string() };
            let ct = if f.ends_with(".js") {
                "application/javascript; charset=utf-8"
            } else if f.ends_with(".css") {
                "text/css; charset=utf-8"
            } else {
                "text/html; charset=utf-8"
            }; // ui 가 index.html + app.css + js/ 로 나뉨
            match std::fs::read(Path::new(&ui_dir).join(&f)) {
                Ok(b) => (ct, b),
                Err(_) => ("text/plain", b"not found".to_vec()),
            }
        };
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", ctype, body.len());
        let _ = s.write_all(&body);
    }
    Ok(())
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}
