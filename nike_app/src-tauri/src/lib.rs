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
    let rd = app.path().resource_dir().unwrap_or_default();
    for c in [data.join("onnx").join("bge-m3"), rd.join("onnx").join("bge-m3"), rd.join("assets").join("onnx").join("bge-m3")] { if c.join("tokenizer.json").exists() { return c; } }   // iOS 번들은 assets/ 아래
    data.join("onnx").join("bge-m3")
}

#[derive(Serialize)] struct PackInfo { key: String, label: String, bytes: u64, chunks: u64, recs: u64, installed: bool, selected: bool, update: bool, offline: bool, dl: u64 }

fn manifest_local(data: &PathBuf) -> Option<serde_json::Value> { fs::read(data.join("packs").join("manifest.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()) }
fn manifest_remote() -> Option<serde_json::Value> {   // 연결 4초·전체 10초(캡티브 와이파이에서 오래 멈추지 않게)
    let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(4)).timeout(std::time::Duration::from_secs(10)).build();
    agent.get(MANIFEST_URL).call().ok()?.into_json().ok()
}
/// 오프라인 대비 manifest: 서버 → (성공 시 manifest_cache.json 에 저장) / 실패 → 저장본 → 개발용 packs/manifest.json → 설치된 팩 폴더(meta.json)로 합성.
/// 반환 (manifest, 온라인 여부). 어떤 경우에도 None 이 아님 → 팩 화면이 에러 대신 설치된 팩을 보여줌.
fn manifest_any(data: &PathBuf) -> (serde_json::Value, bool) {
    let cache = data.join("manifest_cache.json");
    if let Some(m) = manifest_remote() {
        if let Ok(b) = serde_json::to_vec(&m) { let tmp = data.join("manifest_cache.json.tmp"); if fs::write(&tmp, b).is_ok() { let _ = fs::rename(&tmp, &cache); } }
        return (m, true);
    }
    (manifest_offline(data), false)
}
fn manifest_offline(data: &PathBuf) -> serde_json::Value {
    let cache = data.join("manifest_cache.json");
    if let Some(m) = fs::read(&cache).ok().and_then(|b| serde_json::from_slice(&b).ok()).or_else(|| manifest_local(data)) { return m; }
    let mut packs = Vec::new();
    for e in fs::read_dir(data.join("packs")).into_iter().flatten().filter_map(|e| e.ok()) {
        let dir = e.path(); let key = e.file_name().to_string_lossy().to_string();
        if key.ends_with(".tmp") || key.ends_with(".old") { continue; }
        let Some(meta) = fs::read(dir.join("meta.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
        let bytes: u64 = fs::read_dir(&dir).into_iter().flatten().filter_map(|f| f.ok()).filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum();
        packs.push(serde_json::json!({"key": key, "label": meta["label"].as_str().unwrap_or(&key), "bytes": bytes, "chunks": meta["n"], "recs": meta["recs"], "files": {}}));
    }
    serde_json::json!({"packs": packs})
}
#[cfg(test)]
mod offline_tests {
    use super::*;
    #[test]
    fn offline_manifest_fallbacks() {
        let root = std::env::temp_dir().join(format!("nikh_off_{}", std::process::id())); let _ = fs::remove_dir_all(&root);
        let pk = root.join("packs").join("treaty"); fs::create_dir_all(&pk).unwrap();
        fs::write(pk.join("meta.json"), r#"{"key":"treaty","label":"조약","n":7155,"recs":3610}"#).unwrap(); fs::write(pk.join("emb_i8.bin"), vec![0u8; 1000]).unwrap();
        fs::create_dir_all(root.join("packs").join("civil.tmp")).unwrap();
        let m = manifest_offline(&root); let ps = m["packs"].as_array().unwrap();   // 저장본 없음 → 폴더 합성, .tmp 제외
        assert_eq!(ps.len(), 1); assert_eq!(ps[0]["key"], "treaty"); assert_eq!(ps[0]["recs"], 3610); assert!(ps[0]["bytes"].as_u64().unwrap() >= 1000);
        fs::write(root.join("manifest_cache.json"), r#"{"packs":[{"key":"civil"},{"key":"treaty"}]}"#).unwrap();
        assert_eq!(manifest_offline(&root)["packs"].as_array().unwrap().len(), 2);   // 저장본 우선
        let _ = fs::remove_dir_all(&root);
    }
}
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
/// 아이패드: 특허 공보 팩(pat_*)은 메타 상주 메모리가 커서(판례+특허 10GB) 목록·다운로드·로드에서 제외. 판례 팩 `patent`(특허법원)은 유지.
fn mobile_blocked(key: &str) -> bool { cfg!(mobile) && key.starts_with("pat_") }
fn excluded_keys(data: &PathBuf) -> Vec<String> { fs::read_to_string(data.join("excluded.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }   // 사용자가 끈 팩만 기억 → 새 팩은 기본 켜짐

#[tauri::command]
async fn packs(st: State<'_, App>) -> Result<Vec<PackInfo>, String> {
    let (m, online) = manifest_any(&st.data);
    let ex = excluded_keys(&st.data); let mut out = Vec::new();
    for p in m["packs"].as_array().cloned().unwrap_or_default() {
        let key = p["key"].as_str().unwrap_or("").to_string(); if mobile_blocked(&key) { continue; }
        let dir = st.data.join("packs").join(&key); let installed = dir.join("meta.json").exists();
        // 갱신 판정: manifest 의 파일 크기와 설치본 크기가 하나라도 다르면(팩 v2 는 append-only 라 크기가 곧 버전)
        let update = installed && p["files"].as_object().map(|fs| fs.iter().any(|(fname, info)| info["bytes"].as_u64().map(|want| std::fs::metadata(dir.join(fname)).map(|m| m.len() != want).unwrap_or(true)).unwrap_or(false))).unwrap_or(false);
        // 실제로 받을 예상량: 미설치 → 전체 / 설치본 → 파일별로 늘어난 만큼(줄었거나 없으면 그 파일 전체). 해시 불일치 폴백은 예측 불가라 제외
        let dl: u64 = if !installed { p["bytes"].as_u64().unwrap_or(0) } else if !update { 0 } else { p["files"].as_object().map(|fs| fs.iter().map(|(f, i)| { let w = i["bytes"].as_u64().unwrap_or(0); let o = fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0); if o > 0 && o <= w { w - o } else { w } }).sum()).unwrap_or(0) };
        out.push(PackInfo { key: key.clone(), label: p["label"].as_str().unwrap_or(&key).to_string(), bytes: p["bytes"].as_u64().unwrap_or(0),
            chunks: p["chunks"].as_u64().unwrap_or(0), recs: p["recs"].as_u64().unwrap_or(0), installed, selected: installed && !ex.contains(&key), update, offline: !online, dl });
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
fn sha256_file(p: &std::path::Path) -> Result<String, String> {
    use sha2::Digest; let mut h = sha2::Sha256::default(); let mut f = fs::File::open(p).map_err(|e| e.to_string())?; let mut buf = vec![0u8; 1 << 20];
    loop { let n = f.read(&mut buf).map_err(|e| e.to_string())?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(format!("{:x}", h.finalize()))
}
/// 팩 파일 하나를 dst 에 준비(델타). 설치본 old 가 있으면: 크기 같음 → 복제 후 해시 대조 / 더 작음 → 복제 후 HTTP Range 로 늘어난 끝부분만 이어받기.
/// 해시가 manifest 와 다르면(중간이 바뀐 파일 등) 그 파일만 전체 다시 받기. 반환: 실제로 내려받은 바이트.
/// 진행 이벤트: 받은 바이트 / 게이지 총량 증가(전체 재수신 폴백) / 단계 전환("dl"·"verify")
enum Ev<'a> { Bytes(u64), Grow(u64), Phase(&'a str) }
fn sync_file(url: &str, old: Option<&std::path::Path>, dst: &std::path::Path, want_bytes: u64, want_sha: Option<&str>, ev: &mut dyn FnMut(Ev)) -> Result<u64, String> {
    let copy_resp = |resp: ureq::Response, f: &mut fs::File, ev: &mut dyn FnMut(Ev)| -> Result<u64, String> {
        let mut r = resp.into_reader(); let mut buf = vec![0u8; 1 << 20]; let mut n_all = 0u64;
        loop { let n = r.read(&mut buf).map_err(|e| e.to_string())?; if n == 0 { break; } f.write_all(&buf[..n]).map_err(|e| e.to_string())?; n_all += n as u64; ev(Ev::Bytes(n as u64)); }
        Ok(n_all)
    };
    let ok = |p: &std::path::Path, ev: &mut dyn FnMut(Ev)| -> bool {
        if !fs::metadata(p).map(|m| m.len() == want_bytes).unwrap_or(false) { return false; }
        let Some(w) = want_sha else { return true };
        ev(Ev::Phase("verify")); let r = sha256_file(p).map(|h| h == w).unwrap_or(false); ev(Ev::Phase("dl")); r
    };
    let mut spent = 0u64;   // 이어받기에 쓴 바이트(폴백해도 실제 수신량에 포함)
    if let Some(old) = old {
        let ol = fs::metadata(old).map(|m| m.len()).unwrap_or(0);
        if ol > 0 && ol <= want_bytes && { ev(Ev::Phase("verify")); let c = fs::copy(old, dst).is_ok(); ev(Ev::Phase("dl")); c } {
            let mut got = 0u64;
            if ol < want_bytes {
                let resp = ureq::get(url).set("Range", &format!("bytes={ol}-")).call().map_err(|e| format!("{url}: {e}"))?;
                let mut f = fs::OpenOptions::new().write(true).open(dst).map_err(|e| e.to_string())?;
                if resp.status() == 206 { use std::io::Seek; f.seek(std::io::SeekFrom::End(0)).map_err(|e| e.to_string())?; }
                else { f.set_len(0).map_err(|e| e.to_string())?; }   // 서버가 Range 무시(200) → 전체가 옴
                got = copy_resp(resp, &mut f, ev)?;
            }
            if ok(dst, ev) { return Ok(got); }
            spent = got; ev(Ev::Grow(want_bytes));   // 이어받기 실패 → 전체 재수신: 게이지 총량을 그만큼 늘림
        }
    }
    let resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let mut f = fs::File::create(dst).map_err(|e| e.to_string())?; let got = copy_resp(resp, &mut f, ev)?; drop(f);
    if !ok(dst, ev) { let _ = fs::remove_file(dst); return Err(format!("{}: 체크섬 불일치", dst.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())); }
    Ok(spent + got)
}
#[tauri::command]
async fn download_pack(key: String, st: State<'_, App>) -> Result<String, String> {
    if mobile_blocked(&key) { return Err("iPad 에서는 특허 공보 팩을 지원하지 않습니다".into()); }
    if key.contains('/') || key.contains("..") { return Err("잘못된 팩".into()); }
    let m = manifest_remote().ok_or("서버 manifest 없음")?; let base = MANIFEST_URL.trim_end_matches("manifest.json");
    let p = m["packs"].as_array().and_then(|a| a.iter().find(|p| p["key"] == key)).ok_or("팩 없음")?.clone();
    let files = p["files"].as_object().ok_or("files 없음")?.clone();
    let installed = st.data.join("packs").join(&key); let has_old = installed.join("meta.json").exists();
    let dir = st.data.join("packs").join(format!("{key}.tmp")); let _ = fs::remove_dir_all(&dir); fs::create_dir_all(&dir).map_err(|e| e.to_string())?;   // 임시 폴더에 준비(설치본·mmap 무손상), 적용 시 교체
    // 게이지 총량 = 새로 받을 예상 바이트(설치본보다 늘어난 만큼, 없으면 전체)
    let total: u64 = files.iter().map(|(f, i)| { let w = i["bytes"].as_u64().unwrap_or(0); let o = if has_old { fs::metadata(installed.join(f)).map(|m| m.len()).unwrap_or(0) } else { 0 }; if o > 0 && o <= w { w - o } else { w } }).sum();
    let (mut done, mut total) = (0u64, total); let mut stage = "dl";
    if let Ok(mut pr) = st.progress.lock() { *pr = ("dl".into(), 0, total.max(1), key.clone()); }
    for (fname, info) in &files {
        let old = installed.join(fname); let old = if has_old && old.exists() { Some(old.as_path().to_owned()) } else { None };
        let prog = st.progress.clone(); let k2 = key.clone();
        let mut on = |e: Ev| { match e { Ev::Bytes(n) => done += n, Ev::Grow(n) => total += n, Ev::Phase(p) => stage = if p == "verify" { "verify" } else { "dl" } }
            if let Ok(mut pr) = prog.lock() { *pr = (stage.into(), done, total.max(done).max(1), k2.clone()); } };
        sync_file(&format!("{base}{key}/{fname}"), old.as_deref(), &dir.join(fname), info["bytes"].as_u64().unwrap_or(0), info["sha256"].as_str(), &mut on)?;
    }
    // 마지막 무결성 전수 확인: 모든 파일 크기·sha256 을 manifest 와 다시 대조한 뒤에만 완료 표시
    if let Ok(mut pr) = st.progress.lock() { *pr = ("verify".into(), 0, 1, key.clone()); }
    for (fname, info) in &files {
        let pth = dir.join(fname); let len = fs::metadata(&pth).map(|m| m.len()).unwrap_or(u64::MAX);
        if Some(len) != info["bytes"].as_u64() || info["sha256"].as_str().map(|w| sha256_file(&pth).map(|h| h != w).unwrap_or(true)).unwrap_or(false) {
            let _ = fs::remove_dir_all(&dir); return Err(format!("{key}/{fname}: 무결성 확인 실패"));
        }
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

/// iOS 메모리 계측: (실사용 phys_footprint MB, 남은 허용량 MB). 진단 로그용.
#[cfg(target_os = "ios")]
fn mem_mb() -> (u64, u64) {
    extern "C" { fn os_proc_available_memory() -> usize; }
    let avail = unsafe { os_proc_available_memory() } as u64 / 1_000_000;
    let mut ri: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, &mut ri as *mut _ as *mut libc::rusage_info_t) } == 0;
    (if ok { ri.ri_phys_footprint / 1_000_000 } else { 0 }, avail)
}
#[cfg(not(target_os = "ios"))]
fn mem_mb() -> (u64, u64) { (0, 0) }
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
        keys.sort(); keys.retain(|k| !ex.contains(k) && !mobile_blocked(k));
        dirs = keys.iter().map(|k| packs_dir.join(k)).collect();
    } else if st.data.join("pack").join("meta.json").exists() { dirs.push(st.data.join("pack")); }
    if dirs.is_empty() { return Err("설치된 데이터 팩이 없습니다. 설정에서 팩을 선택해 내려받으세요.".into()); }
    { let (fp, av) = mem_mb(); logline(&st.data, format!("mem before load: footprint {fp}MB avail {av}MB")); }
    logline(&st.data, format!("load start packs={:?} model={:?}", dirs.iter().map(|d| d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).collect::<Vec<_>>(), model));
    let prog = |done: usize, total: usize, key: &str| { logline(&st.data, format!("load stage {done}/{total} {key}")); if let Ok(mut p) = st.progress.lock() { *p = ("load".into(), done as u64, total as u64, key.to_string()); } };
    let eng = Engine::load_packs_with(&dirs, &model, &prog).map_err(|e| { logline(&st.data, format!("load FAILED {e}")); format!("엔진 로드 실패: {e}") })?;
    let (r, c) = eng.stats(); logline(&st.data, format!("load done recs={} chunks={} in {:.1}s", r, c, t0.elapsed().as_secs_f32()));
    let mut eng = eng; let t2 = std::time::Instant::now(); let _ = eng.warm(); logline(&st.data, format!("onnx warm in {:.2}s", t2.elapsed().as_secs_f32()));
    *g = Some(eng); drop(g);   // 여기서부터 검색 가능
    // 임베딩 파일 페이지-인은 락 없이 뒤에서: 게이지 "캐시 n%" (검색을 막지 않음)
    let data = st.data.clone(); let pr = st.progress.clone(); if let Ok(mut p) = pr.lock() { *p = ("warm".into(), 0, 1, String::new()); }
    let t3 = std::time::Instant::now(); let dirs2 = dirs.clone();
    std::thread::spawn(move || { Engine::warm_files(&dirs2, &|d, t| { if let Ok(mut p) = pr.lock() { *p = ("warm".into(), d, t.max(1), String::new()); } }); if let Ok(mut p) = pr.lock() { *p = ("ready".into(), 1, 1, String::new()); } let (fp, av) = mem_mb(); logline(&data, format!("file warm done in {:.1}s · mem footprint {fp}MB avail {av}MB", t3.elapsed().as_secs_f32())); });
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
async fn app_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {   // 데스크톱 앱 새 버전 안내: R2 manifest 의 app_version 과 비교(UI 가 홈페이지로 연결). 오프라인이면 latest=null
    let latest = manifest_remote().and_then(|m| m["app_version"].as_str().map(|s| s.to_string()));
    Ok(serde_json::json!({"current": app.package_info().version.to_string(), "latest": latest}))
}
#[tauri::command]
async fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {   // 웹뷰는 target=_blank 를 열지 않음 → OS 기본 브라우저로
    if !(url.starts_with("https://") || url.starts_with("http://")) { return Err("http(s)만".into()); }
    open_url_impl(&app, &url).map_err(|e| e.to_string())
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
    Ok(match g.as_ref() { Some(e) => { let (r, c) = e.stats();
            // 팩별 판례 수(meta.json): UI 가 현재 국가 팩만 세어 "준비 완료 · n건 · 팩 m개" 를 표시
            let per: serde_json::Map<String, serde_json::Value> = e.loaded.iter().map(|k| (k.clone(), fs::read(st.data.join("packs").join(k).join("meta.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).map(|m| m["recs"].clone()).unwrap_or(serde_json::Value::Null))).collect();
            serde_json::json!({"recs": r, "chunks": c, "loaded": e.loaded, "per": per, "status": "ready", "model": model_ok, "stage": stage, "done": done, "total": total, "label": label}) },
        None => serde_json::json!({"recs": 0, "chunks": 0, "loaded": [], "status": status, "stage": stage, "done": done, "total": total, "label": label, "model": model_ok}) })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| { let d = data_dir(app.handle()); let _ = fs::create_dir_all(&d);
            { let dl = d.clone(); std::panic::set_hook(Box::new(move |info| { logline(&dl, format!("PANIC {info}")); })); }   // 로드 스레드 패닉 → 락 오염 → UI 무한 "여는 중" 진단용
            migrate_selected(&d); swap_pending(&d.join("packs")); app.manage(App { eng: Mutex::new(None), data: d, status: Mutex::new("loading".into()), progress: Arc::new(Mutex::new(("load".into(), 0, 0, String::new()))) });
            // 시작 즉시 백그라운드 로드 + 워밍업 (첫 검색 대기 제거). 실패해도 첫 검색 때 다시 시도.
            let h = app.handle().clone(); std::thread::spawn(move || { let st: State<App> = h.state(); if let Err(e) = ensure_engine(&h, &st) { if let Ok(mut s) = st.status.lock() { *s = format!("error: {e}"); } } });
            // 스토어 스크린샷용(시뮬레이터 simctl launch 의 SIMCTL_CHILD_NIKE_SHOT_JS): 지정 JS 를 몇 초 뒤 웹뷰에서 실행. 환경변수 없으면 무동작
            if let Ok(js) = std::env::var("NIKE_SHOT_JS") { let h = app.handle().clone(); let wait = std::env::var("NIKE_SHOT_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(6u64);
                std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_secs(wait)); if let Some(w) = h.get_webview_window("main") { let _ = w.eval(&js); } }); }
            if std::env::var("NIKE_DEVTOOLS").is_ok() { if let Some(w) = app.get_webview_window("main") { w.open_devtools(); } }   // 진단용: NIKE_DEVTOOLS=1
            Ok(()) })
        .invoke_handler(tauri::generate_handler![search, stats, packs, download_model, download_pack, delete_packs, select_packs, open_url, app_update, prewarm, pdf_facts])
        .run(tauri::generate_context!())
        .expect("니케 실행 실패");
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn open_url_impl(_app: &tauri::AppHandle, url: &str) -> std::io::Result<()> { open::that(url) }
#[cfg(any(target_os = "ios", target_os = "android"))]
fn open_url_impl(app: &tauri::AppHandle, url: &str) -> std::io::Result<()> {   // iOS: UIApplication openURL (메인 스레드) → Safari
    #[cfg(target_os = "ios")] {
        let url = url.to_string();
        return app.run_on_main_thread(move || unsafe {
            use objc2::{class, msg_send, runtime::AnyObject};
            use objc2_foundation::{NSDictionary, NSString, NSURL};
            let s = NSString::from_str(&url);
            let Some(u) = NSURL::URLWithString(&s) else { return };
            let ua: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
            let opts = NSDictionary::<AnyObject, AnyObject>::new();
            let _: () = msg_send![ua, openURL: &*u, options: &*opts, completionHandler: std::ptr::null::<std::ffi::c_void>()];
        }).map_err(|e| std::io::Error::other(e.to_string()));
    }
    #[allow(unreachable_code)] { let _ = (app, url); Ok(()) }
}

#[cfg(test)]
mod delta_tests {
    use super::*;
    #[test]
    fn sync_file_resume_and_fallback() {   // 실제 R2 파일로: 신규·이어받기·손상 폴백·동일
        let m = manifest_remote().expect("manifest"); let base = MANIFEST_URL.trim_end_matches("manifest.json");
        let p = m["packs"].as_array().unwrap().iter().find(|p| p["key"] == "treaty").unwrap().clone();
        let info = &p["files"]["meta.jsonl"]; let want = info["bytes"].as_u64().unwrap(); let sha = info["sha256"].as_str().unwrap();
        let url = format!("{base}treaty/meta.jsonl");
        let d = std::env::temp_dir().join(format!("nikh_sync_{}", std::process::id())); let _ = fs::remove_dir_all(&d); fs::create_dir_all(&d).unwrap();
        let full = d.join("full"); let mut n = 0u64;
        let g = sync_file(&url, None, &full, want, Some(sha), &mut |e| if let Ev::Bytes(x) = e { n += x }).unwrap(); assert_eq!(g, want);                    // 1) 설치본 없음 → 전체
        let bytes = fs::read(&full).unwrap(); let half = d.join("half"); fs::write(&half, &bytes[..bytes.len() / 2]).unwrap();
        let g = sync_file(&url, Some(&half), &d.join("o2"), want, Some(sha), &mut |_| {}).unwrap();
        assert_eq!(g, want - (bytes.len() / 2) as u64);                                                                              // 2) 앞 절반 보유 → 나머지 절반만
        let mut bad = bytes[..bytes.len() / 2].to_vec(); bad[10] ^= 0xff; let badp = d.join("bad"); fs::write(&badp, &bad).unwrap();
        let mut grow = 0u64; let g = sync_file(&url, Some(&badp), &d.join("o3"), want, Some(sha), &mut |e| if let Ev::Grow(x) = e { grow += x }).unwrap();
        assert_eq!(grow, want);                                                                                                        //    폴백 시 게이지 총량이 파일 크기만큼 늘어남
        assert_eq!(g, want - (bytes.len() / 2) as u64 + want);                                                                       // 3) 앞부분 손상 → 이어받기 후 해시 불일치 → 전체 재수신
        assert_eq!(sha256_file(&d.join("o3")).unwrap(), sha);
        let g = sync_file(&url, Some(&full), &d.join("o4"), want, Some(sha), &mut |_| {}).unwrap(); assert_eq!(g, 0);               // 4) 동일 → 0 바이트
        println!("treaty/meta.jsonl {} B: full={} half-resume={} corrupt-fallback ok, same=0", want, want, want - (bytes.len() / 2) as u64);
        let _ = fs::remove_dir_all(&d);
    }
}
