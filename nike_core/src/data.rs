//! 데이터 위치·엔진 로드 — NIKE_DATA → ./data → NIKH 앱 데이터 폴더(excluded.json 존중), 모델 폴더 후보, OCR 헬퍼 위치, 공통 JSON 변환.
use anyhow::{anyhow, Result};
use nike_core::{Engine, Hit};
use std::path::PathBuf;

pub(crate) fn home() -> PathBuf { PathBuf::from(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()) }
pub(crate) fn app_data_dir() -> PathBuf {
    if cfg!(target_os = "macos") { home().join("Library/Application Support/studio.hrmk.nike") }
    else if cfg!(windows) { PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("studio.hrmk.nike") }
    else { std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".local/share")).join("studio.hrmk.nike") }
}
/// (팩 폴더들, 모델 폴더). 앱을 깔고 팩을 받아둔 사람은 아무 설정 없이 그대로 씀.
pub(crate) fn resolve() -> Result<(Vec<PathBuf>, PathBuf, PathBuf)> {
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
pub(crate) fn load() -> Result<Engine> {
    let (packs, model, _) = resolve()?; let t = std::time::Instant::now();
    let eng = Engine::load_packs(&packs, &model)?; let (nrec, nchunk) = eng.stats();
    eprintln!("nike: {} packs, {} recs / {} chunks, {:.1}s", packs.len(), nrec, nchunk, t.elapsed().as_secs_f32()); Ok(eng)
}
pub(crate) fn ocr_helper() -> Option<PathBuf> {
    ["ocr/nike_ocr", "/Applications/NIKH.app/Contents/Resources/ocr/nike_ocr"].iter().map(PathBuf::from).chain(std::iter::once(home().join("Applications/NIKH.app/Contents/Resources/ocr/nike_ocr"))).find(|p| p.exists())
}
pub(crate) fn hit_json(h: &Hit) -> serde_json::Value {
    serde_json::json!({"score": h.score, "title": h.title, "caseno": h.caseno, "court": h.court, "date": h.date, "level": h.level, "kind": h.kind, "result": h.result, "laws": h.laws,
        "matched_section": h.sec, "snippet": h.snippet.chars().take(600).collect::<String>(), "issue": h.issue.chars().take(800).collect::<String>(), "url": h.url, "kipris": h.kipris})
}
pub(crate) fn packs_json() -> Result<serde_json::Value> {
    let (packs, model, base) = resolve()?;
    let list: Vec<serde_json::Value> = packs.iter().filter_map(|p| { let m: serde_json::Value = serde_json::from_slice(&std::fs::read(p.join("meta.json")).ok()?).ok()?; Some(serde_json::json!({"key": m["key"], "label": m["label"], "recs": m["recs"], "chunks": m["n"]})) }).collect();
    Ok(serde_json::json!({"data_dir": base.display().to_string(), "model_dir": model.display().to_string(), "packs": list}))
}
