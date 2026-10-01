//! MCP 서버(stdio) — 줄 단위 JSON-RPC 2.0. 도구 search·pdf_facts·packs. 엔진은 첫 호출 때 로드. stdout 은 프로토콜 전용, 로그는 stderr.
use crate::data::{hit_json, load_engine, ocr_helper, packs_json};
use anyhow::Result;
use nike_core::Engine;
use std::io::{BufRead, Write};

pub(crate) fn mcp() -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut eng: Option<Engine> = None;
    let tools = serde_json::json!([
        {"name": "search", "description": "Find similar court opinions (and, in Korean mode, patent publications and administrative decisions) by describing the facts of a matter. Runs fully on this machine over locally installed NIKH data packs; nothing is generated — every hit is a real opinion with its original text link. Include the legal theory alongside the facts for U.S. queries (e.g. 'retaliation, pretext').",
         "inputSchema": {"type": "object", "properties": {"query": {"type": "string", "description": "Facts of the matter in plain language (Korean or English), optionally with legal issue terms"}, "k": {"type": "integer", "description": "Number of hits (default 10, max 100)"}, "mode": {"type": "string", "description": "case (Korean opinions, default) | patent (Korean patent publications) | us (installed U.S. packs) | packs:key1,key2 (explicit pack keys; see the packs tool)"}}, "required": ["query"]}},
        {"name": "pdf_facts", "description": "Extract only the factual section from a pleading PDF on this machine (Korean 청구원인/공소사실, U.S. statement of facts). Scanned PDFs are OCR'd locally. The file is read, not stored.",
         "inputSchema": {"type": "object", "properties": {"path": {"type": "string", "description": "Absolute path to the PDF"}}, "required": ["path"]}},
        {"name": "packs", "description": "List the NIKH data packs installed on this machine (keys, labels, record counts) and where they live.", "inputSchema": {"type": "object", "properties": {}}}
    ]);
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(serde_json::json!({}));
        if id.is_none() {
            continue;
        } // notifications (notifications/initialized 등)
        let reply = |res: std::result::Result<serde_json::Value, (i64, String)>| -> serde_json::Value {
            match res {
                Ok(r) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": r}),
                Err((c, m)) => serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": c, "message": m}}),
            }
        };
        let res: std::result::Result<serde_json::Value, (i64, String)> = match method.as_str() {
            "initialize" => Ok(
                serde_json::json!({"protocolVersion": params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("2024-11-05"), "capabilities": {"tools": {}}, "serverInfo": {"name": "nikh", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "NIKH runs entirely on this machine. Results are real opinions, never generated text. Do not paraphrase citations; quote the returned title, case number, court, date and url as given."}),
            ),
            "ping" => Ok(serde_json::json!({})),
            "tools/list" => Ok(serde_json::json!({"tools": tools})),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let a = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                let text: std::result::Result<String, String> = (|| match name {
                    "packs" => packs_json().map(|v| v.to_string()).map_err(|e| e.to_string()),
                    "search" => {
                        let q = a.get("query").and_then(|v| v.as_str()).ok_or("query is required")?.to_string();
                        let k = a.get("k").and_then(|v| v.as_u64()).unwrap_or(10).clamp(1, 100) as usize;
                        let mode = a.get("mode").and_then(|v| v.as_str()).map(|s| s.to_string());
                        if eng.is_none() {
                            eng = Some(load_engine().map_err(|e| e.to_string())?);
                        }
                        let e = eng.as_mut().unwrap();
                        let hits = e.search_mode(&q, k, None, mode.as_deref()).map_err(|e| e.to_string())?;
                        Ok(serde_json::json!({"query": q, "hits": hits.iter().map(hit_json).collect::<Vec<_>>()}).to_string())
                    }
                    "pdf_facts" => {
                        let p = a.get("path").and_then(|v| v.as_str()).ok_or("path is required")?;
                        let b = std::fs::read(p).map_err(|e| format!("read {p}: {e}"))?;
                        if eng.is_none() {
                            eng = Some(load_engine().map_err(|e| e.to_string())?);
                        }
                        let e = eng.as_mut().unwrap();
                        let (lab, body, _) = e.pdf_facts_smart_ocr(&b, ocr_helper().as_deref()).map_err(|e| e.to_string())?;
                        Ok(serde_json::json!({"label": lab, "text": body}).to_string())
                    }
                    _ => Err(format!("unknown tool: {name}")),
                })();
                match text {
                    Ok(t) => Ok(serde_json::json!({"content": [{"type": "text", "text": t}]})),
                    Err(m) => Ok(serde_json::json!({"content": [{"type": "text", "text": m}], "isError": true})),
                }
            }
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        let _ = writeln!(out, "{}", reply(res));
        let _ = out.flush();
    }
    Ok(())
}
