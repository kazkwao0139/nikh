//! 데이터 팩 — manifest(서버→캐시→폴더 합성), 파일 델타 동기화(이어받기·sha256), 내려받은 .tmp 교체, 제외 목록. 네트워크는 여기(manifest·팩 다운로드)뿐.
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) const MANIFEST_URL: &str = "https://nike-data.hrmk.studio/manifest.json"; // 팩 배포처 (정적). 미정이면 로컬 packs/manifest.json 사용

pub(crate) fn manifest_local(data: &Path) -> Option<serde_json::Value> {
    fs::read(data.join("packs").join("manifest.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
}
pub(crate) fn manifest_remote() -> Option<serde_json::Value> {
    // 연결 4초·전체 10초(캡티브 와이파이에서 오래 멈추지 않게)
    let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(4)).timeout(std::time::Duration::from_secs(10)).build();
    agent.get(MANIFEST_URL).call().ok()?.into_json().ok()
}
/// 오프라인 대비 manifest: 서버 → (성공 시 manifest_cache.json 에 저장) / 실패 → 저장본 → 개발용 packs/manifest.json → 설치된 팩 폴더(meta.json)로 합성.
/// 반환 (manifest, 온라인 여부). 어떤 경우에도 None 이 아님 → 팩 화면이 에러 대신 설치된 팩을 보여줌.
pub(crate) fn manifest_any(data: &Path) -> (serde_json::Value, bool) {
    let cache = data.join("manifest_cache.json");
    if let Some(m) = manifest_remote() {
        if let Ok(b) = serde_json::to_vec(&m) {
            let tmp = data.join("manifest_cache.json.tmp");
            if fs::write(&tmp, b).is_ok() {
                let _ = fs::rename(&tmp, &cache);
            }
        }
        return (m, true);
    }
    (manifest_offline(data), false)
}
pub(crate) fn manifest_offline(data: &Path) -> serde_json::Value {
    let cache = data.join("manifest_cache.json");
    if let Some(m) = fs::read(&cache).ok().and_then(|b| serde_json::from_slice(&b).ok()).or_else(|| manifest_local(data)) {
        return m;
    }
    let mut packs = Vec::new();
    for e in fs::read_dir(data.join("packs")).into_iter().flatten().filter_map(|e| e.ok()) {
        let dir = e.path();
        let key = e.file_name().to_string_lossy().to_string();
        if key.ends_with(".tmp") || key.ends_with(".old") {
            continue;
        }
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
        let root = std::env::temp_dir().join(format!("nikh_off_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pk = root.join("packs").join("treaty");
        fs::create_dir_all(&pk).unwrap();
        fs::write(pk.join("meta.json"), r#"{"key":"treaty","label":"조약","n":7155,"recs":3610}"#).unwrap();
        fs::write(pk.join("emb_i8.bin"), vec![0u8; 1000]).unwrap();
        fs::create_dir_all(root.join("packs").join("civil.tmp")).unwrap();
        let m = manifest_offline(&root);
        let ps = m["packs"].as_array().unwrap(); // 저장본 없음 → 폴더 합성, .tmp 제외
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0]["key"], "treaty");
        assert_eq!(ps[0]["recs"], 3610);
        assert!(ps[0]["bytes"].as_u64().unwrap() >= 1000);
        fs::write(root.join("manifest_cache.json"), r#"{"packs":[{"key":"civil"},{"key":"treaty"}]}"#).unwrap();
        assert_eq!(manifest_offline(&root)["packs"].as_array().unwrap().len(), 2); // 저장본 우선
        let _ = fs::remove_dir_all(&root);
    }
}
/// 내려받기 완료(.complete)된 <key>.tmp 폴더를 설치본으로 교체. 미완성 tmp 는 삭제. 엔진이 내려간 상태에서만 호출.
pub(crate) fn swap_pending(packs_dir: &PathBuf) {
    let Ok(it) = fs::read_dir(packs_dir) else { return };
    for e in it.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".tmp") {
            continue;
        }
        let key = name.trim_end_matches(".tmp").to_string();
        let tmp = e.path();
        let dst = packs_dir.join(&key);
        if !tmp.join(".complete").exists() {
            let _ = fs::remove_dir_all(&tmp);
            continue;
        }
        let _ = fs::remove_file(tmp.join(".complete"));
        let old = packs_dir.join(format!("{key}.old"));
        let _ = fs::remove_dir_all(&old);
        if dst.exists() {
            let _ = fs::rename(&dst, &old);
        }
        if fs::rename(&tmp, &dst).is_ok() {
            let _ = fs::remove_dir_all(&old);
        } else if old.exists() {
            let _ = fs::rename(&old, &dst);
        }
    }
}
/// selected.json(옛 형식: 켤 팩 목록) → excluded.json 1회 이전
pub(crate) fn migrate_selected(data: &Path) {
    let sel = data.join("selected.json");
    let ex = data.join("excluded.json");
    if !sel.exists() || ex.exists() {
        return;
    }
    let Some(keys) = fs::read_to_string(&sel).ok().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()) else { return };
    let installed: Vec<String> = fs::read_dir(data.join("packs"))
        .map(|it| it.filter_map(|e| e.ok()).filter(|e| e.path().join("meta.json").exists()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    let excluded: Vec<String> = installed.into_iter().filter(|k| !keys.contains(k)).collect();
    let _ = fs::write(&ex, serde_json::to_string(&excluded).unwrap_or_default());
    let _ = fs::rename(&sel, data.join("selected.json.migrated"));
}
/// 아이패드: 특허 공보 팩(pat_*)은 메타 상주 메모리가 커서(판례+특허 10GB) 목록·다운로드·로드에서 제외. 판례 팩 `patent`(특허법원)은 유지.
pub(crate) fn mobile_blocked(key: &str) -> bool {
    cfg!(mobile) && key.starts_with("pat_")
}
pub(crate) fn excluded_keys(data: &Path) -> Vec<String> {
    fs::read_to_string(data.join("excluded.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
} // 사용자가 끈 팩만 기억 → 새 팩은 기본 켜짐

pub(crate) fn sha256_file(p: &std::path::Path) -> Result<String, String> {
    use sha2::Digest;
    let mut h = sha2::Sha256::default();
    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
/// 팩 파일 하나를 dst 에 준비(델타). 설치본 old 가 있으면: 크기 같음 → 복제 후 해시 대조 / 더 작음 → 복제 후 HTTP Range 로 늘어난 끝부분만 이어받기.
/// 해시가 manifest 와 다르면(중간이 바뀐 파일 등) 그 파일만 전체 다시 받기. 반환: 실제로 내려받은 바이트.
/// 진행 이벤트: 받은 바이트 / 게이지 총량 증가(전체 재수신 폴백) / 단계 전환("dl"·"verify")
pub(crate) enum Ev<'a> {
    Bytes(u64),
    Grow(u64),
    Phase(&'a str),
}
pub(crate) fn sync_file(
    url: &str,
    old: Option<&std::path::Path>,
    dst: &std::path::Path,
    want_bytes: u64,
    want_sha: Option<&str>,
    ev: &mut dyn FnMut(Ev),
) -> Result<u64, String> {
    let copy_resp = |resp: ureq::Response, f: &mut fs::File, ev: &mut dyn FnMut(Ev)| -> Result<u64, String> {
        let mut r = resp.into_reader();
        let mut buf = vec![0u8; 1 << 20];
        let mut n_all = 0u64;
        loop {
            let n = r.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            n_all += n as u64;
            ev(Ev::Bytes(n as u64));
        }
        Ok(n_all)
    };
    let ok = |p: &std::path::Path, ev: &mut dyn FnMut(Ev)| -> bool {
        if !fs::metadata(p).map(|m| m.len() == want_bytes).unwrap_or(false) {
            return false;
        }
        let Some(w) = want_sha else { return true };
        ev(Ev::Phase("verify"));
        let r = sha256_file(p).map(|h| h == w).unwrap_or(false);
        ev(Ev::Phase("dl"));
        r
    };
    let mut spent = 0u64; // 이어받기에 쓴 바이트(폴백해도 실제 수신량에 포함)
    if let Some(old) = old {
        let ol = fs::metadata(old).map(|m| m.len()).unwrap_or(0);
        if ol > 0 && ol <= want_bytes && {
            ev(Ev::Phase("verify"));
            let c = fs::copy(old, dst).is_ok();
            ev(Ev::Phase("dl"));
            c
        } {
            let mut got = 0u64;
            if ol < want_bytes {
                let resp = ureq::get(url).set("Range", &format!("bytes={ol}-")).call().map_err(|e| format!("{url}: {e}"))?;
                let mut f = fs::OpenOptions::new().write(true).open(dst).map_err(|e| e.to_string())?;
                if resp.status() == 206 {
                    use std::io::Seek;
                    f.seek(std::io::SeekFrom::End(0)).map_err(|e| e.to_string())?;
                } else {
                    f.set_len(0).map_err(|e| e.to_string())?;
                } // 서버가 Range 무시(200) → 전체가 옴
                got = copy_resp(resp, &mut f, ev)?;
            }
            if ok(dst, ev) {
                return Ok(got);
            }
            spent = got;
            ev(Ev::Grow(want_bytes)); // 이어받기 실패 → 전체 재수신: 게이지 총량을 그만큼 늘림
        }
    }
    let resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let mut f = fs::File::create(dst).map_err(|e| e.to_string())?;
    let got = copy_resp(resp, &mut f, ev)?;
    drop(f);
    if !ok(dst, ev) {
        let _ = fs::remove_file(dst);
        return Err(format!("{}: 체크섬 불일치", dst.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    }
    Ok(spent + got)
}

#[cfg(test)]
mod delta_tests {
    use super::*;
    #[test]
    fn sync_file_resume_and_fallback() {
        // 실제 R2 파일로: 신규·이어받기·손상 폴백·동일
        let m = manifest_remote().expect("manifest");
        let base = MANIFEST_URL.trim_end_matches("manifest.json");
        let p = m["packs"].as_array().unwrap().iter().find(|p| p["key"] == "treaty").unwrap().clone();
        let info = &p["files"]["meta.jsonl"];
        let want = info["bytes"].as_u64().unwrap();
        let sha = info["sha256"].as_str().unwrap();
        let url = format!("{base}treaty/meta.jsonl");
        let d = std::env::temp_dir().join(format!("nikh_sync_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let full = d.join("full");
        let mut n = 0u64;
        let g = sync_file(&url, None, &full, want, Some(sha), &mut |e| {
            if let Ev::Bytes(x) = e {
                n += x
            }
        })
        .unwrap();
        assert_eq!(g, want); // 1) 설치본 없음 → 전체
        let bytes = fs::read(&full).unwrap();
        let half = d.join("half");
        fs::write(&half, &bytes[..bytes.len() / 2]).unwrap();
        let g = sync_file(&url, Some(&half), &d.join("o2"), want, Some(sha), &mut |_| {}).unwrap();
        assert_eq!(g, want - (bytes.len() / 2) as u64); // 2) 앞 절반 보유 → 나머지 절반만
        let mut bad = bytes[..bytes.len() / 2].to_vec();
        bad[10] ^= 0xff;
        let badp = d.join("bad");
        fs::write(&badp, &bad).unwrap();
        let mut grow = 0u64;
        let g = sync_file(&url, Some(&badp), &d.join("o3"), want, Some(sha), &mut |e| {
            if let Ev::Grow(x) = e {
                grow += x
            }
        })
        .unwrap();
        assert_eq!(grow, want); //    폴백 시 게이지 총량이 파일 크기만큼 늘어남
        assert_eq!(g, want - (bytes.len() / 2) as u64 + want); // 3) 앞부분 손상 → 이어받기 후 해시 불일치 → 전체 재수신
        assert_eq!(sha256_file(&d.join("o3")).unwrap(), sha);
        let g = sync_file(&url, Some(&full), &d.join("o4"), want, Some(sha), &mut |_| {}).unwrap();
        assert_eq!(g, 0); // 4) 동일 → 0 바이트
        println!("treaty/meta.jsonl {} B: full={} half-resume={} corrupt-fallback ok, same=0", want, want, want - (bytes.len() / 2) as u64);
        let _ = fs::remove_dir_all(&d);
    }
}
