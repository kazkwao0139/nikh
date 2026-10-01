//! 앱 상태 — 데이터·모델 폴더, 엔진 로드(ensure_engine)·워밍업, 진단 로그(질의 내용 기록 없음), iOS 메모리 계측.
use nike_core::Engine;
use std::{fs, io::Write, path::PathBuf, sync::{Arc, Mutex}};
use tauri::{Manager, State};
use crate::packs::{excluded_keys, mobile_blocked};

pub(crate) struct App { pub(crate) eng: Mutex<Option<Engine>>, pub(crate) data: PathBuf, pub(crate) status: Mutex<String>, pub(crate) progress: Arc<Mutex<(String, u64, u64, String)>> }   // progress: (stage, done, total, label) — UI 게이지

pub(crate) fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("NIKE_DATA") { return PathBuf::from(p); }
    if let Ok(r) = app.path().resource_dir() { if r.join("packs").exists() || r.join("pack").exists() { return r; } }
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("data"))
}
pub(crate) fn model_dir(app: &tauri::AppHandle, data: &PathBuf) -> PathBuf {
    let rd = app.path().resource_dir().unwrap_or_default();
    for c in [data.join("onnx").join("bge-m3"), rd.join("onnx").join("bge-m3"), rd.join("assets").join("onnx").join("bge-m3")] { if c.join("tokenizer.json").exists() { return c; } }   // iOS 번들은 assets/ 아래
    data.join("onnx").join("bge-m3")
}

/// iOS 메모리 계측: (실사용 phys_footprint MB, 남은 허용량 MB). 진단 로그용.
#[cfg(target_os = "ios")]
pub(crate) fn mem_mb() -> (u64, u64) {
    extern "C" { fn os_proc_available_memory() -> usize; }
    let avail = unsafe { os_proc_available_memory() } as u64 / 1_000_000;
    let mut ri: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, &mut ri as *mut _ as *mut libc::rusage_info_t) } == 0;
    (if ok { ri.ri_phys_footprint / 1_000_000 } else { 0 }, avail)
}
#[cfg(not(target_os = "ios"))]
pub(crate) fn mem_mb() -> (u64, u64) { (0, 0) }
pub(crate) fn logline(data: &PathBuf, msg: String) {   // 진단 로그: 데이터 폴더의 nike.log (질의 내용은 기록하지 않음)
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(data.join("nike.log")) { let _ = writeln!(f, "{} {}", chrono_like(), msg); }
}
pub(crate) fn chrono_like() -> String { let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0); format!("[t={}]", s) }

pub(crate) fn ensure_engine(app: &tauri::AppHandle, st: &State<App>) -> Result<(), String> {
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
