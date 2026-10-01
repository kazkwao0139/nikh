//! Tauri 커맨드 — UI 가 invoke 로 부르는 함수들. 검색·상태·팩 목록/다운로드/삭제/선택·모델 다운로드·링크 열기·앱 업데이트 확인·PDF 사실관계.
use crate::packs::{excluded_keys, manifest_any, manifest_remote, mobile_blocked, sha256_file, swap_pending, sync_file, Ev, MANIFEST_URL};
use crate::state::{ensure_engine, log_line, model_dir, App};
use nike_core::Engine;
use serde::Serialize;
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};
use tauri::{Manager, State};

#[derive(Serialize)]
pub struct PackInfo {
    key: String,
    label: String,
    bytes: u64,
    chunks: u64,
    recs: u64,
    installed: bool,
    selected: bool,
    update: bool,
    offline: bool,
    dl: u64,
}

#[tauri::command]
pub async fn packs(st: State<'_, App>) -> Result<Vec<PackInfo>, String> {
    let (m, online) = manifest_any(&st.data);
    let ex = excluded_keys(&st.data);
    let mut out = Vec::new();
    for p in m["packs"].as_array().cloned().unwrap_or_default() {
        let key = p["key"].as_str().unwrap_or("").to_string();
        if mobile_blocked(&key) {
            continue;
        }
        let dir = st.data.join("packs").join(&key);
        let installed = dir.join("meta.json").exists();
        // 갱신 판정: manifest 의 파일 크기와 설치본 크기가 하나라도 다르면(팩 v2 는 append-only 라 크기가 곧 버전)
        let update = installed
            && p["files"]
                .as_object()
                .map(|fs| {
                    fs.iter().any(|(fname, info)| {
                        info["bytes"].as_u64().map(|want| std::fs::metadata(dir.join(fname)).map(|m| m.len() != want).unwrap_or(true)).unwrap_or(false)
                    })
                })
                .unwrap_or(false);
        // 실제로 받을 예상량: 미설치 → 전체 / 설치본 → 파일별로 늘어난 만큼(줄었거나 없으면 그 파일 전체). 해시 불일치 폴백은 예측 불가라 제외
        let dl: u64 = if !installed {
            p["bytes"].as_u64().unwrap_or(0)
        } else if !update {
            0
        } else {
            p["files"]
                .as_object()
                .map(|fs| {
                    fs.iter()
                        .map(|(f, i)| {
                            let w = i["bytes"].as_u64().unwrap_or(0);
                            let o = fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0);
                            if o > 0 && o <= w {
                                w - o
                            } else {
                                w
                            }
                        })
                        .sum()
                })
                .unwrap_or(0)
        };
        out.push(PackInfo {
            key: key.clone(),
            label: p["label"].as_str().unwrap_or(&key).to_string(),
            bytes: p["bytes"].as_u64().unwrap_or(0),
            chunks: p["chunks"].as_u64().unwrap_or(0),
            recs: p["recs"].as_u64().unwrap_or(0),
            installed,
            selected: installed && !ex.contains(&key),
            update,
            offline: !online,
            dl,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn download_model(app: tauri::AppHandle, st: State<'_, App>) -> Result<String, String> {
    // 첫 실행: bge-m3(int8)+tokenizer 를 R2 에서 data/onnx/bge-m3 로
    let dir = model_dir(&app, &st.data);
    if dir.join("tokenizer.json").exists() && dir.join("model_int8.onnx").exists() {
        return Ok("present".into());
    }
    let m = manifest_remote().ok_or("서버 manifest 없음")?;
    let base = MANIFEST_URL.trim_end_matches("manifest.json");
    let files = m["model_files"]["files"].as_object().ok_or("manifest 에 model_files 없음")?.clone();
    let sub = m["model_files"]["dir"].as_str().unwrap_or("onnx/bge-m3").to_string();
    let dst_dir = st.data.join("onnx").join("bge-m3");
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let total: u64 = files.values().filter_map(|v| v["bytes"].as_u64()).sum();
    let mut got = 0u64;
    for (fname, info) in &files {
        let dst = dst_dir.join(fname);
        if dst.exists() && fs::metadata(&dst).map(|m| Some(m.len()) == info["bytes"].as_u64()).unwrap_or(false) {
            got += info["bytes"].as_u64().unwrap_or(0);
            continue;
        }
        let url = format!("{base}{sub}/{fname}");
        let tmp = dst_dir.join(format!("{fname}.part"));
        let mut resp = ureq::get(&url).call().map_err(|e| format!("{fname}: {e}"))?.into_reader();
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut hasher = sha2::Sha256::default();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            use sha2::Digest;
            hasher.update(&buf[..n]);
            got += n as u64;
            if let Ok(mut pr) = st.progress.lock() {
                *pr = ("dl".into(), got, total.max(1), "model".into());
            }
        }
        use sha2::Digest;
        let hex = format!("{:x}", hasher.finalize());
        if let Some(want) = info["sha256"].as_str() {
            if want != hex {
                let _ = fs::remove_file(&tmp);
                return Err(format!("{fname}: 체크섬 불일치"));
            }
        }
        fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    }
    if let Ok(mut pr) = st.progress.lock() {
        *pr = (String::new(), 0, 0, String::new());
    }
    Ok("downloaded".into())
}
#[tauri::command]
pub async fn download_pack(key: String, st: State<'_, App>) -> Result<String, String> {
    if mobile_blocked(&key) {
        return Err("iPad 에서는 특허 공보 팩을 지원하지 않습니다".into());
    }
    if key.contains('/') || key.contains("..") {
        return Err("잘못된 팩".into());
    }
    let m = manifest_remote().ok_or("서버 manifest 없음")?;
    let base = MANIFEST_URL.trim_end_matches("manifest.json");
    let p = m["packs"].as_array().and_then(|a| a.iter().find(|p| p["key"] == key)).ok_or("팩 없음")?.clone();
    let files = p["files"].as_object().ok_or("files 없음")?.clone();
    let installed = st.data.join("packs").join(&key);
    let has_old = installed.join("meta.json").exists();
    let dir = st.data.join("packs").join(format!("{key}.tmp"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?; // 임시 폴더에 준비(설치본·mmap 무손상), 적용 시 교체
                                                          // 게이지 총량 = 새로 받을 예상 바이트(설치본보다 늘어난 만큼, 없으면 전체)
    let total: u64 = files
        .iter()
        .map(|(f, i)| {
            let w = i["bytes"].as_u64().unwrap_or(0);
            let o = if has_old { fs::metadata(installed.join(f)).map(|m| m.len()).unwrap_or(0) } else { 0 };
            if o > 0 && o <= w {
                w - o
            } else {
                w
            }
        })
        .sum();
    let (mut done, mut total) = (0u64, total);
    let mut stage = "dl";
    if let Ok(mut pr) = st.progress.lock() {
        *pr = ("dl".into(), 0, total.max(1), key.clone());
    }
    for (fname, info) in &files {
        let old = installed.join(fname);
        let old = if has_old && old.exists() { Some(old.as_path().to_owned()) } else { None };
        let prog = st.progress.clone();
        let k2 = key.clone();
        let mut on = |e: Ev| {
            match e {
                Ev::Bytes(n) => done += n,
                Ev::Grow(n) => total += n,
                Ev::Phase(p) => stage = if p == "verify" { "verify" } else { "dl" },
            }
            if let Ok(mut pr) = prog.lock() {
                *pr = (stage.into(), done, total.max(done).max(1), k2.clone());
            }
        };
        sync_file(&format!("{base}{key}/{fname}"), old.as_deref(), &dir.join(fname), info["bytes"].as_u64().unwrap_or(0), info["sha256"].as_str(), &mut on)?;
    }
    // 마지막 무결성 전수 확인: 모든 파일 크기·sha256 을 manifest 와 다시 대조한 뒤에만 완료 표시
    if let Ok(mut pr) = st.progress.lock() {
        *pr = ("verify".into(), 0, 1, key.clone());
    }
    for (fname, info) in &files {
        let pth = dir.join(fname);
        let len = fs::metadata(&pth).map(|m| m.len()).unwrap_or(u64::MAX);
        if Some(len) != info["bytes"].as_u64() || info["sha256"].as_str().map(|w| sha256_file(&pth).map(|h| h != w).unwrap_or(true)).unwrap_or(false) {
            let _ = fs::remove_dir_all(&dir);
            return Err(format!("{key}/{fname}: 무결성 확인 실패"));
        }
    }
    fs::write(dir.join(".complete"), b"1").map_err(|e| e.to_string())?;
    if let Ok(mut pr) = st.progress.lock() {
        *pr = (String::new(), 0, 0, String::new());
    }
    Ok(key)
}

#[tauri::command]
pub async fn delete_packs(app: tauri::AppHandle, keys: Vec<String>, st: State<'_, App>) -> Result<u64, String> {
    // 설치된 팩 삭제(디스크에서 제거) → 엔진 재로드. 반환: 지운 바이트
    let packs_dir = st.data.join("packs");
    let mut freed = 0u64;
    *st.eng.lock().map_err(|e| e.to_string())? = None; // mmap 해제 후 삭제
    for k in &keys {
        if k.contains('/') || k.contains("..") {
            continue;
        }
        let d = packs_dir.join(k);
        if !d.join("meta.json").exists() {
            continue;
        }
        if let Ok(it) = fs::read_dir(&d) {
            for e in it.filter_map(|e| e.ok()) {
                freed += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        fs::remove_dir_all(&d).map_err(|e| format!("{k}: {e}"))?;
    }
    let ex: Vec<String> = excluded_keys(&st.data).into_iter().filter(|k| !keys.contains(k)).collect();
    let _ = fs::write(st.data.join("excluded.json"), serde_json::to_string(&ex).unwrap_or_default());
    if let Ok(mut s) = st.status.lock() {
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
    Ok(freed)
}
#[tauri::command]
pub async fn select_packs(app: tauri::AppHandle, keys: Vec<String>, st: State<'_, App>) -> Result<(), String> {
    let packs_dir = st.data.join("packs"); // 설치된 팩 중 체크 해제된 것만 기록
    let installed: Vec<String> = fs::read_dir(&packs_dir)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter(|e| e.path().join("meta.json").exists())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| !n.ends_with(".tmp") && !n.ends_with(".old"))
                .collect()
        })
        .unwrap_or_default();
    let excluded: Vec<String> = installed.into_iter().filter(|k| !keys.contains(k)).collect();
    fs::write(st.data.join("excluded.json"), serde_json::to_string(&excluded).unwrap()).map_err(|e| e.to_string())?;
    *st.eng.lock().map_err(|e| e.to_string())? = None;
    swap_pending(&packs_dir); // 엔진을 내린 뒤에야 내려받은 <key>.tmp 를 설치본과 교체(mmap 중 덮어쓰기 금지)
    if let Ok(mut s) = st.status.lock() {
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
    }); // 적용 즉시 백그라운드 재로드 → 푸터 게이지·준비 완료
    Ok(())
}

#[tauri::command]
pub async fn search(
    app: tauri::AppHandle,
    q: String,
    k: usize,
    filter: Option<String>,
    mode: Option<String>,
    st: State<'_, App>,
) -> Result<serde_json::Value, String> {
    ensure_engine(&app, &st)?;
    let mut g = st.eng.lock().map_err(|e| e.to_string())?;
    let eng = g.as_mut().unwrap();
    let f = filter.as_deref().and_then(|s| s.split_once('=')).map(|(a, b)| (a.to_string(), b.to_string()));
    let t1 = std::time::Instant::now();
    let hits = eng.search_mode(&q, k.clamp(1, 200), f.as_ref().map(|(a, b)| (a.as_str(), b.as_str())), mode.as_deref()).map_err(|e| e.to_string())?;
    log_line(&st.data, format!("search k={} hits={} in {}ms (질의 길이 {}자)", k, hits.len(), t1.elapsed().as_millis(), q.chars().count()));
    Ok(serde_json::json!({ "rows": hits, "loaded": eng.loaded }))
}

#[tauri::command]
pub async fn app_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    // 데스크톱 앱 새 버전 안내: R2 manifest 의 app_version 과 비교(UI 가 홈페이지로 연결). 오프라인이면 latest=null
    let latest = manifest_remote().and_then(|m| m["app_version"].as_str().map(|s| s.to_string()));
    Ok(serde_json::json!({"current": app.package_info().version.to_string(), "latest": latest}))
}
#[tauri::command]
pub async fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    // 웹뷰는 target=_blank 를 열지 않음 → OS 기본 브라우저로
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("http(s)만".into());
    }
    open_url_impl(&app, &url).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prewarm(mode: String, st: State<'_, App>) -> Result<(), String> {
    // 모드 탭 전환 시 그 모드 팩 임베딩을 미리 페이지-인 (첫 검색 6초 → 0.2초)
    let packs_dir = st.data.join("packs");
    let ex = excluded_keys(&st.data);
    let dirs: Vec<PathBuf> = fs::read_dir(&packs_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("meta.json").exists())
        .filter(|p| {
            let k = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            !ex.contains(&k)
                && (match mode.as_str() {
                    "patent" => k.starts_with("pat_"),
                    "us" => k.starts_with("us_"),
                    _ => !k.starts_with("pat_") && !k.starts_with("us_"),
                })
        })
        .collect();
    let data = st.data.clone();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        Engine::warm_files(&dirs, &|_, _| {});
        log_line(&data, format!("prewarm {} done in {:.1}s", mode, t.elapsed().as_secs_f32()));
    });
    Ok(())
}

#[tauri::command]
pub async fn pdf_facts(app: tauri::AppHandle, request: tauri::ipc::Request<'_>, st: State<'_, App>) -> Result<serde_json::Value, String> {
    let bytes: Vec<u8> = match request.body() {
        tauri::ipc::InvokeBody::Raw(b) => b.clone(),
        tauri::ipc::InvokeBody::Json(v) => serde_json::from_value(v.get("bytes").cloned().unwrap_or(v.clone())).map_err(|e| e.to_string())?,
    }; // 웹뷰에서 ArrayBuffer 그대로(40MB 도 JSON 배열 없이)   // 소장·공소장 PDF → 사실 문단 (표제 규칙 → 없으면 문단 임베딩 분류; 메모리에서만, 로그 0)
    ensure_engine(&app, &st)?;
    let mut g = st.eng.lock().map_err(|e| e.to_string())?;
    let eng = g.as_mut().unwrap();
    let helper = app.path().resource_dir().map(|r| r.join("ocr").join("nike_ocr")).unwrap_or_else(|_| PathBuf::from("ocr/nike_ocr"));
    let (label, body, _) = eng.pdf_facts_smart_ocr(&bytes, Some(&helper)).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"label": label, "text": body}))
}

#[tauri::command]
pub async fn stats(app: tauri::AppHandle, st: State<'_, App>) -> Result<serde_json::Value, String> {
    let model_ok = {
        let d = model_dir(&app, &st.data);
        d.join("tokenizer.json").exists() && d.join("model_int8.onnx").exists()
    };
    // 로드 중엔 엔진 락이 잡혀 있음 → 기다리지 않고(try_lock) 진행률만 돌려줘야 게이지가 움직임
    let g = match st.eng.try_lock() {
        Ok(g) => g,
        Err(_) => {
            let (stage, done, total, label) = st.progress.lock().map(|p| p.clone()).unwrap_or_default();
            return Ok(
                serde_json::json!({"recs": 0, "chunks": 0, "loaded": [], "status": "loading", "stage": stage, "done": done, "total": total, "label": label, "model": model_ok}),
            );
        }
    };
    let status = st.status.lock().map(|s| s.clone()).unwrap_or_default();
    let (stage, done, total, label) = st.progress.lock().map(|p| p.clone()).unwrap_or_default();
    Ok(match g.as_ref() {
        Some(e) => {
            let (r, c) = e.stats();
            // 팩별 판례 수(meta.json): UI 가 현재 국가 팩만 세어 "준비 완료 · n건 · 팩 m개" 를 표시
            let per: serde_json::Map<String, serde_json::Value> = e
                .loaded
                .iter()
                .map(|k| {
                    (
                        k.clone(),
                        fs::read(st.data.join("packs").join(k).join("meta.json"))
                            .ok()
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                            .map(|m| m["recs"].clone())
                            .unwrap_or(serde_json::Value::Null),
                    )
                })
                .collect();
            serde_json::json!({"recs": r, "chunks": c, "loaded": e.loaded, "per": per, "status": "ready", "model": model_ok, "stage": stage, "done": done, "total": total, "label": label})
        }
        None => {
            serde_json::json!({"recs": 0, "chunks": 0, "loaded": [], "status": status, "stage": stage, "done": done, "total": total, "label": label, "model": model_ok})
        }
    })
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn open_url_impl(_app: &tauri::AppHandle, url: &str) -> std::io::Result<()> {
    open::that(url)
}
#[cfg(any(target_os = "ios", target_os = "android"))]
fn open_url_impl(app: &tauri::AppHandle, url: &str) -> std::io::Result<()> {
    // iOS: UIApplication openURL (메인 스레드) → Safari
    #[cfg(target_os = "ios")]
    {
        let url = url.to_string();
        return app
            .run_on_main_thread(move || unsafe {
                use objc2::{class, msg_send, runtime::AnyObject};
                use objc2_foundation::{NSDictionary, NSString, NSURL};
                let s = NSString::from_str(&url);
                let Some(u) = NSURL::URLWithString(&s) else { return };
                let ua: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
                let opts = NSDictionary::<AnyObject, AnyObject>::new();
                let _: () = msg_send![ua, openURL: &*u, options: &*opts, completionHandler: std::ptr::null::<std::ffi::c_void>()];
            })
            .map_err(|e| std::io::Error::other(e.to_string()));
    }
    #[allow(unreachable_code)]
    {
        let _ = (app, url);
        Ok(())
    }
}
