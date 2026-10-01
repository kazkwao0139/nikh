//! nike CLI — 검색·PDF 사실관계 추출·로컬 서버·MCP 서버. 생성 0, 기록 0, 전부 로컬.
//!   nike search "의뢰인 상황" [--k N] [--mode case|patent|us|packs:a,b] [--filter 키=값] [--json]
//!   nike pdf 소장.pdf [--json]          청구원인·공소사실(영문: statement of facts) 구간만 추출(스캔본은 OCR)
//!   nike packs                          설치된 데이터 팩 목록
//!   nike serve [포트]                    http://localhost:8791 (ui/ + /api/search)
//!   nike mcp                            MCP 서버(stdio, JSON-RPC) — Claude Code·Cursor·Claude Desktop 등 에이전트용 도구 search / pdf_facts / packs
//! 데이터 위치: NIKE_DATA 환경변수 → 현재 폴더의 data/ → NIKH 앱이 팩을 내려받은 폴더(자동). NIKE_PACKS=a,b 로 팩을 고를 수 있음.
mod data;
mod mcp;
mod serve;
use crate::data::{hit_json, load, ocr_helper, packs_json};
use crate::mcp::mcp;
use crate::serve::serve;
use anyhow::{anyhow, Result};

fn arg_val(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "--pdftext" => {
            let b = std::fs::read(&args[1])?;
            println!("{}", nike_core::pdf_text(&b)?);
            return Ok(());
        }
        "packs" => {
            println!("{}", serde_json::to_string_pretty(&packs_json()?)?);
            return Ok(());
        }
        "mcp" => return mcp(),
        "serve" | "--serve" => return serve(args.get(1).and_then(|p| p.parse().ok()).unwrap_or(8791)),
        "pdf" | "--pdf" => {
            let json = args.iter().any(|a| a == "--json");
            let path = args.iter().skip(1).find(|a| !a.starts_with("--")).ok_or_else(|| anyhow!("pdf 파일 경로"))?;
            let mut eng = load()?;
            let b = std::fs::read(path)?;
            let (lab, body, detail) = eng.pdf_facts_smart_ocr(&b, ocr_helper().as_deref())?;
            if json {
                println!("{}", serde_json::json!({"label": lab, "text": body}));
            } else {
                for (l, p, c) in &detail {
                    eprintln!("  {l:6} {c:.3} {p}");
                }
                println!("[{lab}] {}자\n{body}", body.chars().count());
            }
            return Ok(());
        }
        "search" | _ => {
            let (q, k, mode, filter, json) = if cmd == "search" {
                (
                    args.iter()
                        .skip(1)
                        .find(|a| {
                            !a.starts_with("--")
                                && !["--k", "--mode", "--filter"].iter().any(|f| args.iter().position(|x| x == f).map(|i| &args[i + 1] == *a).unwrap_or(false))
                        })
                        .cloned()
                        .ok_or_else(|| anyhow!("검색어"))?,
                    arg_val(&args, "--k").and_then(|x| x.parse().ok()).unwrap_or(10usize),
                    arg_val(&args, "--mode"),
                    arg_val(&args, "--filter"),
                    args.iter().any(|a| a == "--json"),
                )
            } else {
                (
                    args.first().cloned().unwrap_or_else(|| "임차인이 보증금을 돌려받기 전에 집을 비웠는데 임대인이 원상복구 비용을 공제했다".into()),
                    args.get(1).and_then(|x| x.parse().ok()).unwrap_or(8),
                    None,
                    None,
                    false,
                )
            };
            let mut eng = load()?;
            let f = filter.as_deref().and_then(|s| s.split_once('=')).map(|(a, b)| (a.to_string(), b.to_string()));
            let t = std::time::Instant::now();
            let hits = eng.search_mode(&q, k, f.as_ref().map(|(a, b)| (a.as_str(), b.as_str())), mode.as_deref())?;
            let ms = t.elapsed().as_millis();
            if json {
                println!("{}", serde_json::to_string(&serde_json::json!({"query": q, "ms": ms, "hits": hits.iter().map(hit_json).collect::<Vec<_>>()}))?);
            } else {
                for (i, h) in hits.iter().enumerate() {
                    println!(
                        "#{} [{:.3}] {} — {} {} {} [{}]",
                        i + 1,
                        h.score,
                        h.title.chars().take(46).collect::<String>(),
                        h.court.clone().unwrap_or_default(),
                        h.caseno,
                        h.result.join("·"),
                        h.sec
                    );
                }
                println!("⏱ {ms} ms");
            }
            Ok(())
        }
    }
}
