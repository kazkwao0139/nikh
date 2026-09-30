//! nike CLI — 검색·PDF 사실관계 추출·로컬 서버·MCP 서버. 생성 0, 기록 0, 전부 로컬.
//!   nike search "의뢰인 상황" [--k N] [--mode case|patent|us|packs:a,b] [--filter 키=값] [--json]
//!   nike pdf 소장.pdf [--json]          청구원인·공소사실(영문: statement of facts) 구간만 추출(스캔본은 OCR)
//!   nike packs                          설치된 데이터 팩 목록
//!   nike serve [포트]                    http://localhost:8791 (ui/ + /api/search)
//!   nike mcp                            MCP 서버(stdio, JSON-RPC) — Claude Code·Cursor·Claude Desktop 등 에이전트용 도구 search / pdf_facts / packs
//! 데이터 위치: NIKE_DATA 환경변수 → 현재 폴더의 data/ → NIKH 앱이 팩을 내려받은 폴더(자동). NIKE_PACKS=a,b 로 팩을 고를 수 있음.
use anyhow::{anyhow, Result};
use nike_core::{Engine, Hit};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

fn home() -> PathBuf { PathBuf::from(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()) }
fn app_data_dir() -> PathBuf {
    if cfg!(target_os = "macos") { home().join("Library/Application Support/studio.hrmk.nike") }
    else if cfg!(windows) { PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("studio.hrmk.nike") }
    else { std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".local/share")).join("studio.hrmk.nike") }
}
/// (팩 폴더들, 모델 폴더). 앱을 깔고 팩을 받아둔 사람은 아무 설정 없이 그대로 씀.
fn resolve() -> Result<(Vec<PathBuf>, PathBuf, PathBuf)> {
    let base = std::env::var("NIKE_DATA").map(PathBuf::from).ok().or_else(|| { let d = PathBuf::from("data"); if d.join("packs").is_dir() || d.join("pack").is_dir() { Some(d) } else { None } }).unwrap_or_else(app_data_dir);
    let packs_dir = base.join("packs");
    let excluded: Vec<String> = std::fs::read(base.join("excluded.json")).ok().and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()).unwrap_or_default();
    let mut packs: Vec<PathBuf> = if let Ok(keys) = std::env::var("NIKE_PACKS") { keys.split(',').map(|k| packs_dir.join(k.trim())).collect() }
        else if packs_dir.is_dir() { let mut v: Vec<PathBuf> = std::fs::read_dir(&packs_dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("meta.json").exists()).filter(|p| { let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(); !n.ends_with(".tmp") && !n.ends_with(".old") && !excluded.contains(&n) }).collect(); v.sort(); v }
        else if base.join("pack").join("meta.json").exists() { vec![base.join("pack")] } else { vec![] };
    packs.retain(|p| p.join("meta.json").exists());
    if packs.is_empty() { return Err(anyhow!("데이터 팩이 없습니다: {} — NIKH 앱에서 팩을 내려받거나 NIKE_DATA 를 지정하세요.", packs_dir.display())); }
    let mut cands = vec![base.join("onnx/bge-m3"), base.join("../onnx/bge-m3"), PathBuf::from("data/onnx/bge-m3"), PathBuf::from("onnx/bge-m3"),
        PathBuf::from("/Applications/NIKH.app/Contents/Resources/onnx/bge-m3"), home().join("Applications/NIKH.app/Contents/Resources/onnx/bge-m3")];
    if let Ok(la) = std::env::var("LOCALAPPDATA") { cands.push(PathBuf::from(la).join("Programs/NIKH/onnx/bge-m3")); }
    if let Ok(pf) = std::env::var("ProgramFiles") { cands.push(PathBuf::from(pf).join("NIKH/onnx/bge-m3")); }
    let model = cands.into_iter().find(|p| p.join("tokenizer.json").exists()).ok_or_else(|| anyhow!("모델 폴더(tokenizer.json, model_int8.onnx)를 찾지 못했습니다. https://nike-data.hrmk.studio/onnx/bge-m3/ 에서 받아 data/onnx/bge-m3 에 두세요."))?;
    Ok((packs, model, base))
}
fn load() -> Result<Engine> {
    let (packs, model, _) = resolve()?; let t = std::time::Instant::now();
    let eng = Engine::load_packs(&packs, &model)?; let (nrec, nchunk) = eng.stats();
    eprintln!("nike: {} packs, {} recs / {} chunks, {:.1}s", packs.len(), nrec, nchunk, t.elapsed().as_secs_f32()); Ok(eng)
}
fn ocr_helper() -> Option<PathBuf> {
    ["ocr/nike_ocr", "/Applications/NIKH.app/Contents/Resources/ocr/nike_ocr"].iter().map(PathBuf::from).chain(std::iter::once(home().join("Applications/NIKH.app/Contents/Resources/ocr/nike_ocr"))).find(|p| p.exists())
}
fn hit_json(h: &Hit) -> serde_json::Value {
    serde_json::json!({"score": h.score, "title": h.title, "caseno": h.caseno, "court": h.court, "date": h.date, "level": h.level, "kind": h.kind, "result": h.result, "laws": h.laws,
        "matched_section": h.sec, "snippet": h.snippet.chars().take(600).collect::<String>(), "issue": h.issue.chars().take(800).collect::<String>(), "url": h.url, "kipris": h.kipris})
}
fn arg_val(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()) }
fn packs_json() -> Result<serde_json::Value> {
    let (packs, model, base) = resolve()?;
    let list: Vec<serde_json::Value> = packs.iter().filter_map(|p| { let m: serde_json::Value = serde_json::from_slice(&std::fs::read(p.join("meta.json")).ok()?).ok()?; Some(serde_json::json!({"key": m["key"], "label": m["label"], "recs": m["recs"], "chunks": m["n"]})) }).collect();
    Ok(serde_json::json!({"data_dir": base.display().to_string(), "model_dir": model.display().to_string(), "packs": list}))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "--pdftext" => { let b = std::fs::read(&args[1])?; println!("{}", nike_core::pdf_text(&b)?); return Ok(()); }
        "packs" => { println!("{}", serde_json::to_string_pretty(&packs_json()?)?); return Ok(()); }
        "mcp" => return mcp(),
        "serve" | "--serve" => return serve(args.get(1).and_then(|p| p.parse().ok()).unwrap_or(8791)),
        "pdf" | "--pdf" => {
            let json = args.iter().any(|a| a == "--json"); let path = args.iter().skip(1).find(|a| !a.starts_with("--")).ok_or_else(|| anyhow!("pdf 파일 경로"))?;
            let mut eng = load()?; let b = std::fs::read(path)?; let (lab, body, detail) = eng.pdf_facts_smart_ocr(&b, ocr_helper().as_deref())?;
            if json { println!("{}", serde_json::json!({"label": lab, "text": body})); } else { for (l, p, c) in &detail { eprintln!("  {l:6} {c:.3} {p}"); } println!("[{lab}] {}자\n{body}", body.chars().count()); }
            return Ok(());
        }
        "search" | _ => {
            let (q, k, mode, filter, json) = if cmd == "search" {
                (args.iter().skip(1).find(|a| !a.starts_with("--") && !["--k", "--mode", "--filter"].iter().any(|f| args.iter().position(|x| x == f).map(|i| &args[i + 1] == *a).unwrap_or(false))).cloned().ok_or_else(|| anyhow!("검색어"))?,
                 arg_val(&args, "--k").and_then(|x| x.parse().ok()).unwrap_or(10usize), arg_val(&args, "--mode"), arg_val(&args, "--filter"), args.iter().any(|a| a == "--json"))
            } else { (args.first().cloned().unwrap_or_else(|| "임차인이 보증금을 돌려받기 전에 집을 비웠는데 임대인이 원상복구 비용을 공제했다".into()), args.get(1).and_then(|x| x.parse().ok()).unwrap_or(8), None, None, false) };
            let mut eng = load()?; let f = filter.as_deref().and_then(|s| s.split_once('=')).map(|(a, b)| (a.to_string(), b.to_string()));
            let t = std::time::Instant::now(); let hits = eng.search_mode(&q, k, f.as_ref().map(|(a, b)| (a.as_str(), b.as_str())), mode.as_deref())?; let ms = t.elapsed().as_millis();
            if json { println!("{}", serde_json::to_string(&serde_json::json!({"query": q, "ms": ms, "hits": hits.iter().map(hit_json).collect::<Vec<_>>()}))?); }
            else { for (i, h) in hits.iter().enumerate() { println!("#{} [{:.3}] {} — {} {} {} [{}]", i + 1, h.score, h.title.chars().take(46).collect::<String>(), h.court.clone().unwrap_or_default(), h.caseno, h.result.join("·"), h.sec); } println!("⏱ {ms} ms"); }
            Ok(())
        }
    }
}

/// MCP 서버(stdio): 줄 단위 JSON-RPC 2.0. 도구 search·pdf_facts·packs. 엔진은 첫 호출 때 로드. stdout 은 프로토콜 전용, 로그는 stderr.
fn mcp() -> Result<()> {
    let stdin = std::io::stdin(); let mut out = std::io::stdout(); let mut eng: Option<Engine> = None;
    let tools = serde_json::json!([
        {"name": "search", "description": "Find similar court opinions (and, in Korean mode, patent publications and administrative decisions) by describing the facts of a matter. Runs fully on this machine over locally installed NIKH data packs; nothing is generated — every hit is a real opinion with its original text link. Include the legal theory alongside the facts for U.S. queries (e.g. 'retaliation, pretext').",
         "inputSchema": {"type": "object", "properties": {"query": {"type": "string", "description": "Facts of the matter in plain language (Korean or English), optionally with legal issue terms"}, "k": {"type": "integer", "description": "Number of hits (default 10, max 100)"}, "mode": {"type": "string", "description": "case (Korean opinions, default) | patent (Korean patent publications) | us (installed U.S. packs) | packs:key1,key2 (explicit pack keys; see the packs tool)"}}, "required": ["query"]}},
        {"name": "pdf_facts", "description": "Extract only the factual section from a pleading PDF on this machine (Korean 청구원인/공소사실, U.S. statement of facts). Scanned PDFs are OCR'd locally. The file is read, not stored.",
         "inputSchema": {"type": "object", "properties": {"path": {"type": "string", "description": "Absolute path to the PDF"}}, "required": ["path"]}},
        {"name": "packs", "description": "List the NIKH data packs installed on this machine (keys, labels, record counts) and where they live.", "inputSchema": {"type": "object", "properties": {}}}
    ]);
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break }; if line.trim().is_empty() { continue; }
        let msg: serde_json::Value = match serde_json::from_str(&line) { Ok(v) => v, Err(_) => continue };
        let id = msg.get("id").cloned(); let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string(); let params = msg.get("params").cloned().unwrap_or(serde_json::json!({}));
        if id.is_none() { continue; }   // notifications (notifications/initialized 등)
        let reply = |res: std::result::Result<serde_json::Value, (i64, String)>| -> serde_json::Value { match res { Ok(r) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": r}), Err((c, m)) => serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": c, "message": m}}) } };
        let res: std::result::Result<serde_json::Value, (i64, String)> = match method.as_str() {
            "initialize" => Ok(serde_json::json!({"protocolVersion": params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("2024-11-05"), "capabilities": {"tools": {}}, "serverInfo": {"name": "nikh", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "NIKH runs entirely on this machine. Results are real opinions, never generated text. Do not paraphrase citations; quote the returned title, case number, court, date and url as given."})),
            "ping" => Ok(serde_json::json!({})),
            "tools/list" => Ok(serde_json::json!({"tools": tools})),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or(""); let a = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                let text: std::result::Result<String, String> = (|| {
                    match name {
                        "packs" => packs_json().map(|v| v.to_string()).map_err(|e| e.to_string()),
                        "search" => { let q = a.get("query").and_then(|v| v.as_str()).ok_or("query is required")?.to_string(); let k = a.get("k").and_then(|v| v.as_u64()).unwrap_or(10).clamp(1, 100) as usize; let mode = a.get("mode").and_then(|v| v.as_str()).map(|s| s.to_string());
                            if eng.is_none() { eng = Some(load().map_err(|e| e.to_string())?); } let e = eng.as_mut().unwrap();
                            let hits = e.search_mode(&q, k, None, mode.as_deref()).map_err(|e| e.to_string())?; Ok(serde_json::json!({"query": q, "hits": hits.iter().map(hit_json).collect::<Vec<_>>()}).to_string()) }
                        "pdf_facts" => { let p = a.get("path").and_then(|v| v.as_str()).ok_or("path is required")?; let b = std::fs::read(p).map_err(|e| format!("read {p}: {e}"))?;
                            if eng.is_none() { eng = Some(load().map_err(|e| e.to_string())?); } let e = eng.as_mut().unwrap();
                            let (lab, body, _) = e.pdf_facts_smart_ocr(&b, ocr_helper().as_deref()).map_err(|e| e.to_string())?; Ok(serde_json::json!({"label": lab, "text": body}).to_string()) }
                        _ => Err(format!("unknown tool: {name}")) }
                })();
                match text { Ok(t) => Ok(serde_json::json!({"content": [{"type": "text", "text": t}]})), Err(m) => Ok(serde_json::json!({"content": [{"type": "text", "text": m}], "isError": true})) }
            }
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        let _ = writeln!(out, "{}", reply(res)); let _ = out.flush();
    }
    Ok(())
}

fn serve(port: u16) -> Result<()> {
    let mut eng = load()?;
    let ui_dir = std::env::var("NIKE_UI").unwrap_or_else(|_| "ui".into());
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?; eprintln!("NIKH → http://localhost:{port}/");
    for stream in listener.incoming() { let mut s = stream?; let mut buf = [0u8; 8192]; let n = s.read(&mut buf)?; let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
        let (ctype, body): (&str, Vec<u8>) = if path.starts_with("/api/loaded") {
            let keys: Vec<String> = resolve().map(|(p, _, _)| p.iter().filter_map(|d| d.file_name().map(|n| n.to_string_lossy().to_string())).collect()).unwrap_or_default();
            ("application/json; charset=utf-8", serde_json::to_vec(&serde_json::json!({"loaded": keys}))?)
        } else if path.starts_with("/api/search") {
            let qs: std::collections::HashMap<String, String> = path.splitn(2, '?').nth(1).unwrap_or("").split('&').filter_map(|kv| { let mut it = kv.splitn(2, '='); Some((it.next()?.to_string(), urldecode(it.next().unwrap_or("")))) }).collect();
            let q = qs.get("q").cloned().unwrap_or_default(); let k = qs.get("k").and_then(|x| x.parse().ok()).unwrap_or(20);
            let flt = qs.get("filter").and_then(|f| f.split_once('=').map(|(a, b)| (a.to_string(), b.to_string()))); let mode = qs.get("mode").cloned();
            let hits = if q.trim().is_empty() { vec![] } else { eng.search_mode(&q, k, flt.as_ref().map(|(a, b)| (a.as_str(), b.as_str())), mode.as_deref())? };
            ("application/json; charset=utf-8", serde_json::to_vec(&serde_json::json!({"rows": hits}))?)
        } else {
            let f = if path == "/" { "index.html".to_string() } else { path.trim_start_matches('/').to_string() };
            match std::fs::read(Path::new(&ui_dir).join(&f)) { Ok(b) => ("text/html; charset=utf-8", b), Err(_) => ("text/plain", b"not found".to_vec()) }
        };
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", ctype, body.len()); let _ = s.write_all(&body);
    }
    Ok(())
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes(); let mut out = Vec::with_capacity(b.len()); let mut i = 0;
    while i < b.len() { match b[i] { b'%' if i + 2 < b.len() => { if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i+1..i+3]).unwrap_or("zz"), 16) { out.push(v); i += 3; continue; } out.push(b[i]); i += 1; } b'+' => { out.push(b' '); i += 1; } c => { out.push(c); i += 1; } } }
    String::from_utf8_lossy(&out).to_string()
}
