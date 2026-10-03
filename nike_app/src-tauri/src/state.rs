//! 앱 상태 — 데이터·모델 폴더, 엔진 로드(ensure_engine)·워밍업, 진단 로그(질의 내용 기록 없음), iOS 메모리 계측.
use crate::packs::{excluded_keys, installed_pack_keys, mobile_blocked};
use nike_core::Engine;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Manager, State};

pub(crate) struct App {
    pub(crate) eng: Mutex<Option<Engine>>,
    pub(crate) data: PathBuf,
    pub(crate) status: Mutex<String>,
    pub(crate) progress: Arc<Mutex<(String, u64, u64, String)>>,
    pub(crate) failsafe_hold: std::sync::atomic::AtomicBool, // 지난 실행이 팩을 싣다 죽었음 → 사용자가 팩을 다시 고를 때까지 아무것도 싣지 않음
} // progress: (stage, done, total, label) — UI 게이지

pub(crate) fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("NIKE_DATA") {
        return PathBuf::from(p);
    }
    if let Ok(r) = app.path().resource_dir() {
        if r.join("packs").exists() || r.join("pack").exists() {
            return r;
        }
    }
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("data"))
}
pub(crate) fn model_dir(app: &tauri::AppHandle, data: &Path) -> PathBuf {
    let rd = app.path().resource_dir().unwrap_or_default();
    for c in [data.join("onnx").join("bge-m3"), rd.join("onnx").join("bge-m3"), rd.join("assets").join("onnx").join("bge-m3")] {
        if c.join("tokenizer.json").exists() {
            return c;
        }
    } // iOS 번들은 assets/ 아래
    data.join("onnx").join("bge-m3")
}

/// iOS 메모리 계측: (실사용 phys_footprint MB, 남은 허용량 MB). 진단 로그용.
#[cfg(target_os = "ios")]
pub(crate) fn memory_mb() -> (u64, u64) {
    extern "C" {
        fn os_proc_available_memory() -> usize;
    }
    let avail = unsafe { os_proc_available_memory() } as u64 / 1_000_000;
    let mut ri: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, &mut ri as *mut _ as *mut libc::rusage_info_t) } == 0;
    (if ok { ri.ri_phys_footprint / 1_000_000 } else { 0 }, avail)
}
#[cfg(not(target_os = "ios"))]
pub(crate) fn memory_mb() -> (u64, u64) {
    (0, 0)
}
pub(crate) fn log_line(data: &Path, msg: String) {
    // 진단 로그: 데이터 폴더의 nike.log (질의 내용은 기록하지 않음)
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(data.join("nike.log")) {
        let _ = writeln!(f, "{} {}", log_timestamp(), msg);
    }
}
pub(crate) fn log_timestamp() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("[t={}]", s)
}

/// 페일세이프 표식: 팩을 싣기 시작할 때 만들고(내용 = 이 로드의 토큰), 캐시 채우기까지 끝나면 지운다.
/// 앱을 **켤 때** 남아 있으면 지난 실행이 싣는 도중 죽은 것 → 이번 실행은 사용자가 팩을 다시 고를 때까지 아무것도 싣지 않는다.
/// 같은 실행 안에서 남아 있는 표식(캐시 채우는 중 재로드 등)은 크래시가 아니므로 검사는 시작 시 한 번만 한다.
pub(crate) const LOADING_FLAG: &str = "loading.flag";
pub(crate) const FAILSAFE_MSG: &str = "지난 실행이 팩을 싣다가 종료되어 모든 팩을 내렸습니다. 필요한 팩만 다시 골라 주세요.";
static LOAD_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 앱 시작 시 1회 호출. 표식이 남아 있으면 설치된 팩을 전부 '꺼짐'으로 기록하고 true(= 이번 실행은 로드 보류).
/// 꺼짐 기록에 실패하면 표식을 남겨 둔다(다음 실행에서도 보류되도록). 기록 성공 여부와 무관하게 이번 실행은 보류한다.
pub(crate) fn failsafe_check_at_startup(data: &Path) -> bool {
    let flag = data.join(LOADING_FLAG);
    if !flag.exists() {
        return false;
    }
    let installed: Vec<String> =
        installed_pack_keys(&data.join("packs")).unwrap_or_default().into_iter().filter(|k| !k.ends_with(".tmp") && !k.ends_with(".old")).collect();
    if fs::write(data.join("excluded.json"), serde_json::to_string(&installed).unwrap_or_default()).is_ok() {
        let _ = fs::remove_file(&flag);
    }
    true
}

/// 로드 시작: 표식에 새 토큰을 적고 그 토큰을 돌려준다.
pub(crate) fn mark_loading(data: &Path) -> String {
    let token = format!("{}:{}", std::process::id(), LOAD_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let _ = fs::write(data.join(LOADING_FLAG), &token);
    token
}

/// 로드가 끝났거나 처리된 오류로 끝남: 표식이 **이 로드의 것일 때만** 지운다(더 나중에 시작한 로드의 표식은 건드리지 않음).
pub(crate) fn clear_loading(data: &Path, token: &str) {
    let flag = data.join(LOADING_FLAG);
    if fs::read_to_string(&flag).map(|c| c == token).unwrap_or(false) {
        let _ = fs::remove_file(&flag);
    }
}

pub(crate) fn ensure_engine(app: &tauri::AppHandle, st: &State<App>) -> Result<(), String> {
    let mut g = st.eng.lock().map_err(|e| e.to_string())?;
    if g.is_some() {
        return Ok(());
    }
    if st.failsafe_hold.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(FAILSAFE_MSG.into()); // 사용자가 팩을 다시 고르기(select_packs) 전에는 아무것도 싣지 않음
    }
    let t0 = std::time::Instant::now();
    let model = model_dir(app, &st.data);
    let mut dirs: Vec<PathBuf> = Vec::new();
    let packs_dir = st.data.join("packs");
    if packs_dir.exists() {
        let ex = excluded_keys(&st.data);
        let mut keys: Vec<String> = installed_pack_keys(&packs_dir).map_err(|e| e.to_string())?;
        keys.sort();
        keys.retain(|k| !ex.contains(k) && !mobile_blocked(k) && !k.ends_with(".tmp") && !k.ends_with(".old")); // 받는 중(.tmp)·교체 전(.old) 폴더는 팩으로 싣지 않음
        dirs = keys.iter().map(|k| packs_dir.join(k)).collect();
    } else if st.data.join("pack").join("meta.json").exists() {
        dirs.push(st.data.join("pack"));
    }
    if dirs.is_empty() {
        return Err("설치된 데이터 팩이 없습니다. 설정에서 팩을 선택해 내려받으세요.".into());
    }
    {
        let (fp, av) = memory_mb();
        log_line(&st.data, format!("mem before load: footprint {fp}MB avail {av}MB"));
    }
    log_line(
        &st.data,
        format!(
            "load start packs={:?} model={:?}",
            dirs.iter().map(|d| d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).collect::<Vec<_>>(),
            model
        ),
    );
    let prog = |done: usize, total: usize, key: &str| {
        log_line(&st.data, format!("load stage {done}/{total} {key}"));
        if let Ok(mut p) = st.progress.lock() {
            *p = ("load".into(), done as u64, total as u64, key.to_string());
        }
    };
    let load_token = mark_loading(&st.data);
    let eng = Engine::load_packs_with(&dirs, &model, &prog).map_err(|e| {
        clear_loading(&st.data, &load_token); // 처리된 실패는 크래시가 아님
        log_line(&st.data, format!("load FAILED {e}"));
        format!("엔진 로드 실패: {e}")
    })?;
    let (r, c) = eng.stats();
    log_line(&st.data, format!("load done recs={} chunks={} in {:.1}s", r, c, t0.elapsed().as_secs_f32()));
    let mut eng = eng;
    let t2 = std::time::Instant::now();
    let _ = eng.warm();
    log_line(&st.data, format!("onnx warm in {:.2}s", t2.elapsed().as_secs_f32()));
    *g = Some(eng);
    drop(g); // 여기서부터 검색 가능
             // 임베딩 파일 페이지-인은 락 없이 뒤에서: 게이지 "캐시 n%" (검색을 막지 않음)
    let data = st.data.clone();
    let pr = st.progress.clone();
    if let Ok(mut p) = pr.lock() {
        *p = ("warm".into(), 0, 1, String::new());
    }
    let t3 = std::time::Instant::now();
    let dirs2 = dirs.clone();
    std::thread::spawn(move || {
        Engine::warm_files(&dirs2, &|d, t| {
            if let Ok(mut p) = pr.lock() {
                *p = ("warm".into(), d, t.max(1), String::new());
            }
        });
        if let Ok(mut p) = pr.lock() {
            *p = ("ready".into(), 1, 1, String::new());
        }
        clear_loading(&data, &load_token); // 여기까지 살아 있으면 정상 로드
        let (fp, av) = memory_mb();
        log_line(&data, format!("file warm done in {:.1}s · mem footprint {fp}MB avail {av}MB", t3.elapsed().as_secs_f32()));
    });
    Ok(())
}

/// 엔진을 백그라운드에서 (재)로드. status 를 "loading" 으로 돌려놓고 시작, 실패하면 "error: …" 를 남겨 UI 가 표시.
pub(crate) fn spawn_engine_load(app: &tauri::AppHandle) {
    if let Ok(mut s) = app.state::<App>().status.lock() {
        *s = "loading".into();
    }
    let h = app.clone();
    std::thread::spawn(move || {
        let st: State<App> = h.state();
        if let Err(e) = ensure_engine(&h, &st) {
            if let Ok(mut s) = st.status.lock() {
                *s = format!("error: {e}");
            }
        }
    });
}

/// 진행 상태 스냅샷 (stage, done, total, label)
pub(crate) fn progress_snapshot(st: &State<App>) -> (String, u64, u64, String) {
    st.progress.lock().map(|p| p.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp_data(name: &str) -> PathBuf {
        let data = std::env::temp_dir().join(format!("nikh_failsafe_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&data);
        for k in ["us_ca_modern", "us_ny_modern", "us_tx_modern.tmp"] {
            fs::create_dir_all(data.join("packs").join(k)).unwrap();
            fs::write(data.join("packs").join(k).join("meta.json"), "{}").unwrap();
        }
        data
    }
    fn excluded(data: &Path) -> Vec<String> {
        let mut ex: Vec<String> = serde_json::from_str(&fs::read_to_string(data.join("excluded.json")).unwrap()).unwrap();
        ex.sort();
        ex
    }
    #[test]
    fn failsafe_no_flag_does_nothing() {
        let data = temp_data("noflag");
        assert!(!failsafe_check_at_startup(&data));
        assert!(!data.join("excluded.json").exists());
        let _ = fs::remove_dir_all(&data);
    }
    #[test]
    fn failsafe_crash_during_load_unloads_everything_once() {
        let data = temp_data("crash");
        let _token = mark_loading(&data); // 로드 시작 후 프로세스가 죽었다고 가정(지우지 않음)
        assert!(failsafe_check_at_startup(&data)); // 다음 실행: 보류
        assert_eq!(excluded(&data), vec!["us_ca_modern", "us_ny_modern"]); // 받는 중(.tmp) 폴더는 팩이 아님
        assert!(!data.join(LOADING_FLAG).exists());
        assert!(!failsafe_check_at_startup(&data)); // 그다음 실행: 정상
        let _ = fs::remove_dir_all(&data);
    }
    #[test]
    fn failsafe_clean_load_leaves_no_flag() {
        let data = temp_data("clean");
        let token = mark_loading(&data);
        clear_loading(&data, &token);
        assert!(!data.join(LOADING_FLAG).exists());
        assert!(!failsafe_check_at_startup(&data));
        let _ = fs::remove_dir_all(&data);
    }
    #[test]
    fn failsafe_older_load_cannot_clear_newer_flag() {
        let data = temp_data("overlap");
        let old = mark_loading(&data); // 첫 로드(캐시 채우는 중)
        let new = mark_loading(&data); // 사용자가 팩을 바꿔 재로드 시작
        clear_loading(&data, &old); // 첫 로드의 캐시 채우기가 뒤늦게 끝남
        assert!(data.join(LOADING_FLAG).exists()); // 새 로드의 표식은 살아 있어야 함
        clear_loading(&data, &new);
        assert!(!data.join(LOADING_FLAG).exists());
        let _ = fs::remove_dir_all(&data);
    }
    #[test]
    fn failsafe_keeps_flag_when_excluded_cannot_be_written() {
        let data = temp_data("nowrite");
        fs::create_dir_all(data.join("excluded.json")).unwrap(); // 같은 이름의 폴더 → 쓰기 실패
        let _token = mark_loading(&data);
        assert!(failsafe_check_at_startup(&data)); // 그래도 이번 실행은 보류
        assert!(data.join(LOADING_FLAG).exists()); // 다음 실행에서도 보류되도록 표식 유지
        assert!(failsafe_check_at_startup(&data));
        let _ = fs::remove_dir_all(&data);
    }
}
