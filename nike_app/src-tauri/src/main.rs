// 데스크톱 진입점 — 모든 로직은 lib.rs(nike_app_lib::run), iOS 는 mobile_entry_point 로 같은 run() 사용
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() { nike_app_lib::run() }
