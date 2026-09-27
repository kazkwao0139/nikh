// 니케 데스크톱 — Tauri 창 + 인프로세스 엔진 + 도메인별 팩 선택 다운로드. 질의는 네트워크 0·로그 0 (다운로드만 네트워크).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use nike_core::Engine;
use serde::Serialize;
use std::{fs, io::{Read, Write}, path::PathBuf, sync::{Arc, Mutex}};
use tauri::{Manager, State};

const MANIFEST_URL: &str = "https://nike-data.hrmk.studio/manifest.json";   // 팩 배포처 (정적). 미정이면 로컬 packs/manifest.json 사용

struct App { eng: Mutex<Option<Engine>>, data: PathBuf, status: Mutex<String>, progress: Arc<Mutex<(String, u64, u64, String)>> }   // progress: (stage, done, total, label) — UI 게이지

fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("NIKE_DATA") { return PathBuf::from(p); }
    if let Ok(r) = app.path().resource_dir() { if r.join("packs").exists() || r.join("pack").exists() { return r; } }
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("data"))
}
fn model_dir(app: &tauri::AppHandle, data: &PathBuf) -> PathBuf {
    for c in [data.join("onnx").join("bge-m3"), app.path().resource_dir().map(|r| r.join("onnx").join("bge-m3")).unwrap_or_default()] { if c.join("tokenizer.json").exists() { return c; } }
    data.join("onnx").join("bge-m3")
}

#[derive(Serialize)] struct PackInfo { key: String, label: String, bytes: u64, chunks: u64, recs: u64, installed: bool, selected: bool, update: bool }

fn manifest_local(data: &PathBuf) -> Option<serde_json::Value> { fs::read(data.join("packs").join("manifest.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()) }
fn manifest_remote() -> Option<serde_json::Value> { ureq::get(MANIFEST_URL).timeout(std::time::Duration::from_secs(10)).call().ok()?.into_json().ok() }
/// 내려받기 완료(.complete)된 <key>.tmp 폴더를 설치본으로 교체. 미완성 tmp 는 삭제. 엔진이 내려간 상태에서만 호출.
fn swap_pending(packs_dir: &PathBuf) {
    let Ok(it) = fs::read_dir(packs_dir) else { return };
    for e in it.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string(); if !name.ends_with(".tmp") { continue; }
        let key = name.trim_end_matches(".tmp").to_string(); let tmp = e.path(); let dst = packs_dir.join(&key);
        if !tmp.join(".complete").exists() { let _ = fs::remove_dir_all(&tmp); continue; }
        let _ = fs::remove_file(tmp.join(".complete")); let old = packs_dir.join(format!("{key}.old")); let _ = fs::remove_dir_all(&old);
        if dst.exists() { let _ = fs::rename(&dst, &old); }
        if fs::rename(&tmp, &dst).is_ok() { let _ = fs::remove_dir_all(&old); } else if old.exists() { let _ = fs::rename(&old, &dst); }
    }
}
/// selected.json(옛 형식: 켤 팩 목록) → excluded.json 1회 이전
fn migrate_selected(data: &PathBuf) {
    let sel = data.join("selected.json"); let ex = data.join("excluded.json");
    if !sel.exists() || ex.exists() { return; }
    let Some(keys) = fs::read_to_string(&sel).ok().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()) else { return };
    let installed: Vec<String> = fs::read_dir(data.join("packs")).map(|it| it.filter_map(|e| e.ok()).filter(|e| e.path().join("meta.json").exists()).map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    let excluded: Vec<String> = installed.into_iter().filter(|k| !keys.contains(k)).collect();
    let _ = fs::write(&ex, serde_json::to_string(&excluded).unwrap_or_default()); let _ = fs::rename(&sel, data.join("selected.json.migrated"));
}
fn excluded_keys(data: &PathBuf) -> Vec<String> { fs::read_to_string(data.join("excluded.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }   // 사용자가 끈 팩만 기억 → 새 팩은 기본 켜짐

#[tauri::command]
async fn packs(st: State<'_, App>) -> Result<Vec<PackInfo>, String> {
    let m = manifest_remote().or_else(|| manifest_local(&st.data)).ok_or("manifest 없음")?;
    let ex = excluded_keys(&st.data); let mut out = Vec::new();
    for p in m["packs"].as_array().cloned().unwrap_or_default() {
        let key = p["key"].as_str().unwrap_or("").to_string();
        let dir = st.data.join("packs").join(&key); let installed = dir.join("meta.json").exists();
        // 갱신 판정: manifest 의 파일 크기와 설치본 크기가 하나라도 다르면(팩 v2 는 append-only 라 크기가 곧 버전)
        let update = installed && p["files"].as_object().map(|fs| fs.iter().any(|(fname, info)| info["bytes"].as_u64().map(|want| std::fs::metadata(dir.join(fname)).map(|m| m.len() != want).unwrap_or(true)).unwrap_or(false))).unwrap_or(false);
        out.push(PackInfo { key: key.clone(), label: p["label"].as_str().unwrap_or(&key).to_string(), bytes: p["bytes"].as_u64().unwrap_or(0),
            chunks: p["chunks"].as_u64().unwrap_or(0), recs: p["recs"].as_u64().unwrap_or(0), installed, selected: installed && !ex.contains(&key), update });
    }
    Ok(out)
}

#[tauri::command]
async fn download_model(app: tauri::AppHandle, st: State<'_, App>) -> Result<String, String> {   // 첫 실행: bge-m3(int8)+tokenizer 를 R2 에서 data/onnx/bge-m3 로
    let dir = model_dir(&app, &st.data); if dir.join("tokenizer.json").exists() && dir.join("model_int8.onnx").exists() { return Ok("present".into()); }
    let m = manifest_remote().ok_or("서버 manifest 없음")?; let base = MANIFEST_URL.trim_end_matches("manifest.json");
    let files = m["model_files"]["files"].as_object().ok_or("manifest 에 model_files 없음")?.clone(); let sub = m["model_files"]["dir"].as_str().unwrap_or("onnx/bge-m3").to_string();
    let dst_dir = st.data.join("onnx").join("bge-m3"); fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let total: u64 = files.values().filter_map(|v| v["bytes"].as_u64()).sum(); let mut got = 0u64;
    for (fname, info) in &files {
        let dst = dst_dir.join(fname); if dst.exists() && fs::metadata(&dst).map(|m| Some(m.len()) == info["bytes"].as_u64()).unwrap_or(false) { got += info["bytes"].as_u64().unwrap_or(0); continue; }
        let url = format!("{base}{sub}/{fname}"); let tmp = dst_dir.join(format!("{fname}.part"));
        let mut resp = ureq::get(&url).call().map_err(|e| format!("{fname}: {e}"))?.into_reader();
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?; let mut hasher = sha2::Sha256::default(); let mut buf = vec![0u8; 1 << 20];
        loop { let n = resp.read(&mut buf).map_err(|e| e.to_string())?; if n == 0 { break; } f.write_all(&buf[..n]).map_err(|e| e.to_string())?; use sha2::Digest; hasher.update(&buf[..n]); got += n as u64; if let Ok(mut pr) = st.progress.lock() { *pr = ("dl".into(), got, total.max(1), "model".into()); } }
        use sha2::Digest; let hex = format!("{:x}", hasher.finalize()); if let Some(want) = info["sha256"].as_str() { if want != hex { let _ = fs::remove_file(&tmp); return Err(format!("{fname}: 체크섬 불일치")); } }
        fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    }
    if let Ok(mut pr) = st.progress.lock() { *pr = (String::new(), 0, 0, String::new()); }
    Ok("downloaded".into())
}
#[tauri::command]
async fn download_pack(key: String, st: State<'_, App>) -> Result<String, String> {
    let m = manifest_remote().ok_or("서버 manifest 없음")?; let base = MANIFEST_URL.trim_end_matches("manifest.json");
    let p = m["packs"].as_array().and_then(|a| a.iter().find(|p| p["key"] == key)).ok_or("팩 없음")?.clone();
    let dir = st.data.join("packs").join(format!("{key}.tmp")); let _ = fs::remove_dir_all(&dir); fs::create_dir_all(&dir).map_err(|e| e.to_string())?;   // 임시 폴더에 받고(설치본·mmap 무손상), 적용 시 교체
    let total: u64 = p["files"].as_object().map(|fs| fs.values().filter_map(|v| v["bytes"].as_u64()).sum()).unwrap_or(0); let mut got: u64 = 0;
    if let Ok(mut pr) = st.progress.lock() { *pr = ("dl".into(), 0, total.max(1), key.clone()); }
    for (fname, info) in p["files"].as_object().ok_or("files 없음")? {
        let url = format!("{base}{key}/{fname}"); let dst = dir.join(fname);
        let mut resp = ureq::get(&url).call().map_err(|e| format!("{fname}: {e}"))?.into_reader();
        let mut f = fs::File::create(&dst).map_err(|e| e.to_string())?; let mut hasher = sha2::Sha256::default(); let mut buf = vec![0u8; 1 << 20];
        loop { let n = resp.read(&mut buf).map_err(|e| e.to_string())?; if n == 0 { break; } f.write_all(&buf[..n]).map_err(|e| e.to_string())?; use sha2::Digest; hasher.update(&buf[..n]); got += n as u64; if let Ok(mut pr) = st.progress.lock() { *pr = ("dl".into(), got, total.max(1), key.clone()); } }
        use sha2::Digest; let got = format!("{:x}", hasher.finalize());
        if let Some(want) = info["sha256"].as_str() { if want != got { let _ = fs::remove_file(&dst); return Err(format!("{fname}: 체크섬 불일치")); } }
    }
    fs::write(dir.join(".complete"), b"1").map_err(|e| e.to_string())?;
    if let Ok(mut pr) = st.progress.lock() { *pr = (String::new(), 0, 0, String::new()); }
    Ok(key)
}

#[tauri::command]
async fn delete_packs(app: tauri::AppHandle, keys: Vec<String>, st: State<'_, App>) -> Result<u64, String> {   // 설치된 팩 삭제(디스크에서 제거) → 엔진 재로드. 반환: 지운 바이트
    let packs_dir = st.data.join("packs"); let mut freed = 0u64;
    *st.eng.lock().map_err(|e| e.to_string())? = None;   // mmap 해제 후 삭제
    for k in &keys { if k.contains('/') || k.contains("..") { continue; } let d = packs_dir.join(k); if !d.join("meta.json").exists() { continue; }
        if let Ok(it) = fs::read_dir(&d) { for e in it.filter_map(|e| e.ok()) { freed += e.metadata().map(|m| m.len()).unwrap_or(0); } }
        fs::remove_dir_all(&d).map_err(|e| format!("{k}: {e}"))?; }
    let ex: Vec<String> = excluded_keys(&st.data).into_iter().filter(|k| !keys.contains(k)).collect(); let _ = fs::write(st.data.join("excluded.json"), serde_json::to_string(&ex).unwrap_or_default());
    if let Ok(mut s) = st.status.lock() { *s = "loading".into(); }
    let h = app.clone(); std::thread::spawn(move || { let st: State<App> = h.state(); if let Err(e) = ensure_engine(&h, &st) { if let Ok(mut s) = st.status.lock() { *s = format!("error: {e}"); } } });
    Ok(freed)
}
#[tauri::command]
async fn select_packs(app: tauri::AppHandle, keys: Vec<String>, st: State<'_, App>) -> Result<(), String> {
    let packs_dir = st.data.join("packs");   // 설치된 팩 중 체크 해제된 것만 기록
    let installed: Vec<String> = fs::read_dir(&packs_dir).map(|it| it.filter_map(|e| e.ok()).filter(|e| e.path().join("meta.json").exists()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.ends_with(".tmp") && !n.ends_with(".old")).collect()).unwrap_or_default();
    let excluded: Vec<String> = installed.into_iter().filter(|k| !keys.contains(k)).collect();
    fs::write(st.data.join("excluded.json"), serde_json::to_string(&excluded).unwrap()).map_err(|e| e.to_string())?;
    *st.eng.lock().map_err(|e| e.to_string())? = None;
    swap_pending(&packs_dir);   // 엔진을 내린 뒤에야 내려받은 <key>.tmp 를 설치본과 교체(mmap 중 덮어쓰기 금지)
    if let Ok(mut s) = st.status.lock() { *s = "loading".into(); }
    let h = app.clone(); std::thread::spawn(move || { let st: State<App> = h.state(); if let Err(e) = ensure_engine(&h, &st) { if let Ok(mut s) = st.status.lock() { *s = format!("error: {e}"); } } });   // 적용 즉시 백그라운드 재로드 → 푸터 게이지·준비 완료
    Ok(())
}

fn logline(data: &PathBuf, msg: String) {   // 진단 로그: 데이터 폴더의 nike.log (질의 내용은 기록하지 않음)
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(data.join("nike.log")) { let _ = writeln!(f, "{} {}", chrono_like(), msg); }
}
fn chrono_like() -> String { let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0); format!("[t={}]", s) }

fn ensure_engine(app: &tauri::AppHandle, st: &State<App>) -> Result<(), String> {
    let mut g = st.eng.lock().map_err(|e| e.to_string())?;
    if g.is_some() { return Ok(()); }
    let t0 = std::time::Instant::now();
    let model = model_dir(app, &st.data);
    let mut dirs: Vec<PathBuf> = Vec::new();
    let packs_dir = st.data.join("packs");
    if packs_dir.exists() {
        let ex = excluded_keys(&st.data);
        let mut keys: Vec<String> = fs::read_dir(&packs_dir).map_err(|e| e.to_string())?.filter_map(|e| e.ok()).filter(|e| e.path().join("meta.json").exists()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        keys.sort(); keys.retain(|k| !ex.contains(k));
        dirs = keys.iter().map(|k| packs_dir.join(k)).collect();
    } else if st.data.join("pack").join("meta.json").exists() { dirs.push(st.data.join("pack")); }
    if dirs.is_empty() { return Err("설치된 데이터 팩이 없습니다. 설정에서 팩을 선택해 내려받으세요.".into()); }
    logline(&st.data, format!("load start packs={:?} model={:?}", dirs.iter().map(|d| d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).collect::<Vec<_>>(), model));
    let prog = |done: usize, total: usize, key: &str| { if let Ok(mut p) = st.progress.lock() { *p = ("load".into(), done as u64, total as u64, key.to_string()); } };
    let eng = Engine::load_packs_with(&dirs, &model, &prog).map_err(|e| { logline(&st.data, format!("load FAILED {e}")); format!("엔진 로드 실패: {e}") })?;
    let (r, c) = eng.stats(); logline(&st.data, format!("load done recs={} chunks={} in {:.1}s", r, c, t0.elapsed().as_secs_f32()));
    let mut eng = eng; let t2 = std::time::Instant::now(); let _ = eng.warm(); logline(&st.data, format!("onnx warm in {:.2}s", t2.elapsed().as_secs_f32()));
    *g = Some(eng); drop(g);   // 여기서부터 검색 가능
    // 임베딩 파일 페이지-인은 락 없이 뒤에서: 게이지 "캐시 n%" (검색을 막지 않음)
    let data = st.data.clone(); let pr = st.progress.clone(); if let Ok(mut p) = pr.lock() { *p = ("warm".into(), 0, 1, String::new()); }
    let t3 = std::time::Instant::now(); let dirs2 = dirs.clone();
    std::thread::spawn(move || { Engine::warm_files(&dirs2, &|d, t| { if let Ok(mut p) = pr.lock() { *p = ("warm".into(), d, t.max(1), String::new()); } }); if let Ok(mut p) = pr.lock() { *p = ("ready".into(), 1, 1, String::new()); } logline(&data, format!("file warm done in {:.1}s", t3.elapsed().as_secs_f32())); });
    Ok(())
}

#[tauri::command]
async fn search(app: tauri::AppHandle, q: String, k: usize, filter: Option<String>, mode: Option<String>, st: State<'_, App>) -> Result<serde_json::Value, String> {
    ensure_engine(&app, &st)?;
    let mut g = st.eng.lock().map_err(|e| e.to_string())?; let eng = g.as_mut().unwrap();
    let f = filter.as_deref().and_then(|s| s.split_once('=')).map(|(a, b)| (a.to_string(), b.to_string()));
    let t1 = std::time::Instant::now();
    let hits = eng.search_mode(&q, k.max(1).min(200), f.as_ref().map(|(a, b)| (a.as_str(), b.as_str())), mode.as_deref()).map_err(|e| e.to_string())?;
    logline(&st.data, format!("search k={} hits={} in {}ms (질의 길이 {}자)", k, hits.len(), t1.elapsed().as_millis(), q.chars().count()));
    Ok(serde_json::json!({ "rows": hits, "loaded": eng.loaded }))
}

#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {   // 웹뷰는 target=_blank 를 열지 않음 → OS 기본 브라우저로
    if !(url.starts_with("https://") || url.starts_with("http://")) { return Err("http(s)만".into()); }
    open_url_impl(&url).map_err(|e| e.to_string())
}

#[tauri::command]
async fn prewarm(mode: String, st: State<'_, App>) -> Result<(), String> {   // 모드 탭 전환 시 그 모드 팩 임베딩을 미리 페이지-인 (첫 검색 6초 → 0.2초)
    let packs_dir = st.data.join("packs"); let ex = excluded_keys(&st.data);
    let dirs: Vec<PathBuf> = fs::read_dir(&packs_dir).map_err(|e| e.to_string())?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("meta.json").exists())
        .filter(|p| { let k = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(); !ex.contains(&k) && (match mode.as_str() { "patent" => k.starts_with("pat_"), "us" => k.starts_with("us_"), _ => !k.starts_with("pat_") && !k.starts_with("us_") }) }).collect();
    let data = st.data.clone(); std::thread::spawn(move || { let t = std::time::Instant::now(); Engine::warm_files(&dirs, &|_, _| {}); logline(&data, format!("prewarm {} done in {:.1}s", mode, t.elapsed().as_secs_f32())); });
    Ok(())
}

#[tauri::command]
async fn pdf_facts(app: tauri::AppHandle, request: tauri::ipc::Request<'_>, st: State<'_, App>) -> Result<serde_json::Value, String> {
    let bytes: Vec<u8> = match request.body() { tauri::ipc::InvokeBody::Raw(b) => b.clone(), tauri::ipc::InvokeBody::Json(v) => serde_json::from_value(v.get("bytes").cloned().unwrap_or(v.clone())).map_err(|e| e.to_string())? };   // 웹뷰에서 ArrayBuffer 그대로(40MB 도 JSON 배열 없이)   // 소장·공소장 PDF → 사실 문단 (표제 규칙 → 없으면 문단 임베딩 분류; 메모리에서만, 로그 0)
    ensure_engine(&app, &st)?;
    let mut g = st.eng.lock().map_err(|e| e.to_string())?; let eng = g.as_mut().unwrap();
    let helper = app.path().resource_dir().map(|r| r.join("ocr").join("nike_ocr")).unwrap_or_else(|_| PathBuf::from("ocr/nike_ocr"));
    let (label, body, _) = eng.pdf_facts_smart_ocr(&bytes, Some(&helper)).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"label": label, "text": body}))
}

#[tauri::command]
async fn stats(app: tauri::AppHandle, st: State<'_, App>) -> Result<serde_json::Value, String> {
    let model_ok = { let d = model_dir(&app, &st.data); d.join("tokenizer.json").exists() && d.join("model_int8.onnx").exists() };
    // 로드 중엔 엔진 락이 잡혀 있음 → 기다리지 않고(try_lock) 진행률만 돌려줘야 게이지가 움직임
    let g = match st.eng.try_lock() { Ok(g) => g, Err(_) => { let (stage, done, total, label) = st.progress.lock().map(|p| p.clone()).unwrap_or_default();
        return Ok(serde_json::json!({"recs": 0, "chunks": 0, "loaded": [], "status": "loading", "stage": stage, "done": done, "total": total, "label": label, "model": model_ok})); } };
    let status = st.status.lock().map(|s| s.clone()).unwrap_or_default();
    let (stage, done, total, label) = st.progress.lock().map(|p| p.clone()).unwrap_or_default();
    Ok(match g.as_ref() { Some(e) => { let (r, c) = e.stats(); serde_json::json!({"recs": r, "chunks": c, "loaded": e.loaded, "status": "ready", "model": model_ok, "stage": stage, "done": done, "total": total, "label": label}) },
        None => serde_json::json!({"recs": 0, "chunks": 0, "loaded": [], "status": status, "stage": stage, "done": done, "total": total, "label": label, "model": model_ok}) })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| { let d = data_dir(app.handle()); let _ = fs::create_dir_all(&d); migrate_selected(&d); swap_pending(&d.join("packs")); app.manage(App { eng: Mutex::new(None), data: d, status: Mutex::new("loading".into()), progress: Arc::new(Mutex::new(("load".into(), 0, 0, String::new()))) });
            // 시작 즉시 백그라운드 로드 + 워밍업 (첫 검색 대기 제거). 실패해도 첫 검색 때 다시 시도.
            let h = app.handle().clone(); std::thread::spawn(move || { let st: State<App> = h.state(); if let Err(e) = ensure_engine(&h, &st) { if let Ok(mut s) = st.status.lock() { *s = format!("error: {e}"); } } });
            if std::env::var("NIKE_DEVTOOLS").is_ok() { if let Some(w) = app.get_webview_window("main") { w.open_devtools(); } }   // 진단용: NIKE_DEVTOOLS=1
            Ok(()) })
        .invoke_handler(tauri::generate_handler![search, stats, packs, download_model, download_pack, delete_packs, select_packs, open_url, prewarm, pdf_facts])
        .run(tauri::generate_context!())
        .expect("니케 실행 실패");
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn open_url_impl(url: &str) -> std::io::Result<()> { open::that(url) }
#[cfg(any(target_os = "ios", target_os = "android"))]
fn open_url_impl(_url: &str) -> std::io::Result<()> { Ok(()) }   // 모바일: 웹뷰 쪽 window.open 으로 처리(TODO opener 플러그인)
