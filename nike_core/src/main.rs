//! CLI: nike "의뢰인 상황" [k]   — 로컬 서버 없이 엔진만 검증. 로컬 HTTP 모드: nike --serve 8791 (ui/ 정적 + /api/search)
use anyhow::Result;
use nike_core::{default_paths, Engine};
use std::io::{Read, Write};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (pack, model) = default_paths();
    let pdf_mode = args.first().map(|s| s.as_str()) == Some("--pdf");
    let t = std::time::Instant::now();
    let mut eng = if let Ok(keys) = std::env::var("NIKE_PACKS") {
        let base = pack.parent().unwrap().join("packs"); let dirs: Vec<_> = keys.split(',').map(|k| base.join(k.trim())).collect();
        Engine::load_packs(&dirs, &model)?
    } else { Engine::load(&pack, &model)? };
    let (nrec, nchunk) = eng.stats(); eprintln!("loaded {} recs / {} chunks in {:.1}s", nrec, nchunk, t.elapsed().as_secs_f32());
    if pdf_mode { let b = std::fs::read(&args[1])?; let (lab, body, detail) = eng.pdf_facts_smart(&b)?; for (l, p, c) in &detail { eprintln!("  {l:6} {c:.3} {p}"); } println!("[{lab}] {}자\n{body}", body.chars().count()); return Ok(()); }
    if args.first().map(|s| s.as_str()) == Some("--serve") {
        let port: u16 = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(8791);
        let ui_dir = std::env::var("NIKE_UI").unwrap_or_else(|_| "ui".into());
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))?; eprintln!("니케 → http://localhost:{port}/");
        for stream in listener.incoming() { let mut s = stream?; let mut buf = [0u8; 8192]; let n = s.read(&mut buf)?; let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (ctype, body): (&str, Vec<u8>) = if path.starts_with("/api/loaded") {   // 웹 모드: 실린 팩 키(NIKE_PACKS) → UI 가 미설치 주를 회색 처리
                let keys: Vec<String> = std::env::var("NIKE_PACKS").map(|k| k.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default();
                ("application/json; charset=utf-8", serde_json::to_vec(&serde_json::json!({"loaded": keys}))?)
            } else if path.starts_with("/api/search") {
                let qs: std::collections::HashMap<String, String> = path.splitn(2, '?').nth(1).unwrap_or("").split('&').filter_map(|kv| { let mut it = kv.splitn(2, '='); Some((it.next()?.to_string(), urldecode(it.next().unwrap_or("")))) }).collect();
                let q = qs.get("q").cloned().unwrap_or_default(); let k = qs.get("k").and_then(|x| x.parse().ok()).unwrap_or(20);
                let flt = qs.get("filter").and_then(|f| f.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())));
                let mode = qs.get("mode").cloned();
                let hits = if q.trim().is_empty() { vec![] } else { eng.search_mode(&q, k, flt.as_ref().map(|(a, b)| (a.as_str(), b.as_str())), mode.as_deref())? };
                ("application/json; charset=utf-8", serde_json::to_vec(&serde_json::json!({"rows": hits}))?)
            } else {
                let f = if path == "/" { "index.html".to_string() } else { path.trim_start_matches('/').to_string() };
                match std::fs::read(std::path::Path::new(&ui_dir).join(&f)) { Ok(b) => ("text/html; charset=utf-8", b), Err(_) => ("text/plain", b"not found".to_vec()) }
            };
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", ctype, body.len()); let _ = s.write_all(&body);
        }
        return Ok(());
    }
    let q = args.first().cloned().unwrap_or_else(|| "임차인이 보증금을 돌려받기 전에 집을 비웠는데 임대인이 원상복구 비용을 공제했다".into());
    let k = args.get(1).and_then(|x| x.parse().ok()).unwrap_or(8);
    let t = std::time::Instant::now(); let hits = eng.search(&q, k, None)?; let ms = t.elapsed().as_millis();
    for (i, h) in hits.iter().enumerate() { println!("#{} [{:.3}] {} — {} {} {} [{}]", i + 1, h.score, h.title.chars().take(46).collect::<String>(), h.court.clone().unwrap_or_default(), h.caseno, h.result.join("·"), h.sec); }
    println!("⏱ {ms} ms");
    Ok(())
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes(); let mut out = Vec::with_capacity(b.len()); let mut i = 0;
    while i < b.len() { match b[i] { b'%' if i + 2 < b.len() => { if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i+1..i+3]).unwrap_or("zz"), 16) { out.push(v); i += 3; continue; } out.push(b[i]); i += 1; } b'+' => { out.push(b' '); i += 1; } c => { out.push(c); i += 1; } } }
    String::from_utf8_lossy(&out).to_string()
}
