// 니케 데스크톱 — Tauri 창 + 인프로세스 엔진 + 도메인별 팩 선택 다운로드. 질의는 네트워크 0·로그 0 (다운로드만 네트워크).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod commands;
mod packs;
mod state;
use crate::packs::{migrate_selected, swap_pending};
use crate::state::{data_dir, log_line, spawn_engine_load, App};
use std::{
    fs,
    sync::{Arc, Mutex},
};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let d = data_dir(app.handle());
            let _ = fs::create_dir_all(&d);
            {
                let dl = d.clone();
                std::panic::set_hook(Box::new(move |info| {
                    log_line(&dl, format!("PANIC {info}"));
                }));
            } // 로드 스레드 패닉 → 락 오염 → UI 무한 "여는 중" 진단용
            migrate_selected(&d);
            swap_pending(&d.join("packs"));
            app.manage(App {
                eng: Mutex::new(None),
                data: d,
                status: Mutex::new("loading".into()),
                progress: Arc::new(Mutex::new(("load".into(), 0, 0, String::new()))),
            });
            // 시작 즉시 백그라운드 로드 + 워밍업 (첫 검색 대기 제거). 실패해도 첫 검색 때 다시 시도.
            spawn_engine_load(app.handle());
            // 스토어 스크린샷용(시뮬레이터 simctl launch 의 SIMCTL_CHILD_NIKE_SHOT_JS): 지정 JS 를 몇 초 뒤 웹뷰에서 실행. 환경변수 없으면 무동작
            if let Ok(js) = std::env::var("NIKE_SHOT_JS") {
                let h = app.handle().clone();
                let wait = std::env::var("NIKE_SHOT_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(6u64);
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(wait));
                    if let Some(w) = h.get_webview_window("main") {
                        let _ = w.eval(&js);
                    }
                });
            }
            if std::env::var("NIKE_DEVTOOLS").is_ok() {
                if let Some(w) = app.get_webview_window("main") {
                    w.open_devtools();
                }
            } // 진단용: NIKE_DEVTOOLS=1
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::stats,
            commands::packs,
            commands::download_model,
            commands::download_pack,
            commands::delete_packs,
            commands::select_packs,
            commands::open_url,
            commands::app_update,
            commands::prewarm,
            commands::pdf_facts
        ])
        .run(tauri::generate_context!())
        .expect("니케 실행 실패");
}
