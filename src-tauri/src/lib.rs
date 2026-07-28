// 컷체크 — 캡컷 draft를 읽어 컷 길이를 제공하는 백엔드
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

/// 캡컷 draft 루트 자동 탐색 (globalSetting에서 사용자 지정 경로 우선 → 기본 경로)
/// OS별 분기: Windows는 기존 로직, macOS는 ~/Movies/CapCut/User Data 구조
fn draft_root() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // 1) globalSetting에서 사용자가 바꾼 저장 경로 파싱
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let cfg = PathBuf::from(&local).join("CapCut\\User Data\\Config\\globalSetting");
            if let Ok(txt) = fs::read_to_string(&cfg) {
                for line in txt.lines() {
                    if let Some(rest) = line.trim().strip_prefix("currentCustomDraftPath=") {
                        let p = rest.trim().replace("\\\\", "\\");
                        let pb = PathBuf::from(p);
                        if pb.is_dir() {
                            return Some(pb);
                        }
                    }
                }
            }
        }
        // 2) 문서 폴더 기본 경로
        if let Some(up) = std::env::var_os("USERPROFILE") {
            let pb = PathBuf::from(&up)
                .join("Documents\\CapCut\\User Data\\Projects\\com.lveditor.draft");
            if pb.is_dir() {
                return Some(pb);
            }
        }
        // 3) AppData 기본 경로
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let pb = PathBuf::from(&local).join("CapCut\\User Data\\Projects\\com.lveditor.draft");
            if pb.is_dir() {
                return Some(pb);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            // 맥 캡컷 기본 구조: ~/Movies/CapCut/User Data
            let base = PathBuf::from(&home)
                .join("Movies")
                .join("CapCut")
                .join("User Data");
            // 1) globalSetting에서 사용자 지정 저장 경로 파싱
            let cfg = base.join("Config").join("globalSetting");
            if let Ok(txt) = fs::read_to_string(&cfg) {
                for line in txt.lines() {
                    if let Some(rest) = line.trim().strip_prefix("currentCustomDraftPath=") {
                        let pb = PathBuf::from(rest.trim());
                        if pb.is_dir() {
                            return Some(pb);
                        }
                    }
                }
            }
            // 2) 기본 Projects 경로
            let pb = base.join("Projects").join("com.lveditor.draft");
            if pb.is_dir() {
                return Some(pb);
            }
            // 3) 혹시 Movies 대신 다른 위치를 쓰는 경우 대비 (Application Support)
            let alt = PathBuf::from(&home)
                .join("Library")
                .join("Application Support")
                .join("CapCut")
                .join("User Data")
                .join("Projects")
                .join("com.lveditor.draft");
            if alt.is_dir() {
                return Some(alt);
            }
        }
    }
    None
}

#[tauri::command]
fn get_root() -> Option<String> {
    draft_root().map(|p| p.to_string_lossy().to_string())
}

#[derive(Serialize)]
struct Proj {
    name: String,
    mtime: u64,
}

#[tauri::command]
fn list_projects() -> Vec<Proj> {
    let mut out = Vec::new();
    if let Some(root) = draft_root() {
        if let Ok(rd) = fs::read_dir(&root) {
            for e in rd.flatten() {
                let dc = e.path().join("draft_content.json");
                if let Ok(meta) = fs::metadata(&dc) {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    out.push(Proj {
                        name: e.file_name().to_string_lossy().to_string(),
                        mtime,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out
}

#[derive(Serialize)]
struct Cut {
    start: f64,
    dur: f64,
    video: String,
    src: f64,
}

#[derive(Serialize)]
struct CutsResult {
    mtime: u64,
    cuts: Vec<Cut>,
}

#[tauri::command]
fn read_cuts(project: String) -> Result<CutsResult, String> {
    let root = draft_root().ok_or("캡컷 폴더를 찾지 못했어요")?;
    let path = root.join(&project).join("draft_content.json");
    let mtime = fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let txt = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let empty = Vec::new();
    // 영상 소재 id → 경로 매핑 (##_draftpath_placeholder_..._## 치환)
    let folder = root.join(&project);
    let folder_fwd = folder.to_string_lossy().replace('\\', "/");
    let re = regex_lite(&folder_fwd);
    let mut vids = std::collections::HashMap::new();
    if let Some(mats) = v["materials"]["videos"].as_array() {
        for m in mats {
            let p = m["path"]
                .as_str()
                .or_else(|| m["file_Path"].as_str())
                .unwrap_or("");
            let p = re(p);
            if let Some(id) = m["id"].as_str() {
                vids.insert(id.to_string(), p);
            }
        }
    }
    let tracks = v["tracks"].as_array().unwrap_or(&empty);
    let main = tracks
        .iter()
        .filter(|t| t["type"] == "video")
        .max_by_key(|t| t["segments"].as_array().map(|s| s.len()).unwrap_or(0));
    let mut cuts = Vec::new();
    if let Some(track) = main {
        if let Some(segs) = track["segments"].as_array() {
            for s in segs {
                let mid = s["material_id"].as_str().unwrap_or("");
                cuts.push(Cut {
                    start: s["target_timerange"]["start"].as_f64().unwrap_or(0.0) / 1e6,
                    dur: s["target_timerange"]["duration"].as_f64().unwrap_or(0.0) / 1e6,
                    video: vids.get(mid).cloned().unwrap_or_default(),
                    src: s["source_timerange"]["start"].as_f64().unwrap_or(0.0) / 1e6,
                });
            }
        }
    }
    cuts.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    Ok(CutsResult { mtime, cuts })
}

/// ##_draftpath_placeholder_XXX_## 를 프로젝트 폴더 경로로 치환하는 클로저
fn regex_lite(folder: &str) -> impl Fn(&str) -> String + '_ {
    move |p: &str| {
        if let Some(start) = p.find("##_draftpath_placeholder_") {
            if let Some(end_rel) = p[start..].find("_##") {
                let end = start + end_rel + 3;
                return format!("{}{}{}", &p[..start], folder, &p[end..]);
            }
        }
        p.to_string()
    }
}

/// 컷 썸네일 생성 (비동기 커맨드) — ffmpeg를 블로킹 스레드풀로 돌려
/// 길이 조회(read_cuts) 등 다른 커맨드를 절대 막지 않게 함
#[tauri::command]
async fn thumbnail(app: tauri::AppHandle, video: String, src_sec: f64) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || make_thumb(&app, &video, src_sec))
        .await
        .map_err(|e| e.to_string())?
}

fn make_thumb(app: &tauri::AppHandle, video: &str, src_sec: f64) -> Result<String, String> {
    use std::process::Command;
    if video.is_empty() || !std::path::Path::new(video).exists() {
        return Err("영상 없음".into());
    }
    #[cfg(target_os = "windows")]
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(|l| PathBuf::from(l).join("CutBunny").join("thumbs"))
        .ok_or("캐시 경로 없음")?;
    #[cfg(target_os = "macos")]
    let cache = std::env::var_os("HOME")
        .map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Caches")
                .join("CutBunny")
                .join("thumbs")
        })
        .ok_or("캐시 경로 없음")?;
    let _ = fs::create_dir_all(&cache);
    use base64::Engine;
    let key = format!("{:x}", md5_like(&format!("{}|{:.2}", video, src_sec)));
    let out = cache.join(format!("{}.jpg", key));
    if out.exists() {
        // 캐시 히트: 바로 data URI로
        let bytes = fs::read(&out).map_err(|e| e.to_string())?;
        let b = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(format!("data:image/jpeg;base64,{}", b));
    }
    let ff = ffmpeg_path(app)?;
    let mut cmd = Command::new(&ff);
    cmd.args([
        "-y",
        "-v",
        "error",
        "-ss",
        &format!("{:.2}", src_sec),
        "-i",
        video,
        "-frames:v",
        "1",
        "-vf",
        "scale=120:-1",
        &out.to_string_lossy(),
    ]);
    // 윈도우에서만 콘솔창 숨김 플래그 (CREATE_NO_WINDOW)
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    let status = cmd.status().map_err(|e| e.to_string())?;
    if status.success() && out.exists() {
        // 새로 만든 jpg를 data URI로 반환 (asset 프로토콜 scope 문제 없이 확실히 표시)
        let bytes = fs::read(&out).map_err(|e| e.to_string())?;
        let b = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(format!("data:image/jpeg;base64,{}", b))
    } else {
        Err("썸네일 생성 실패".into())
    }
}

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

fn ffmpeg_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    // OS별 ffmpeg 실행파일 이름
    #[cfg(target_os = "windows")]
    let name = "ffmpeg.exe";
    #[cfg(not(target_os = "windows"))]
    let name = "ffmpeg";
    // 번들된 리소스(bin/ffmpeg) 우선
    if let Ok(res) = app.path().resource_dir() {
        let p = res.join("bin").join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    // 개발 중 fallback
    let dev = PathBuf::from("bin").join(name);
    if dev.exists() {
        return Ok(dev);
    }
    Ok(PathBuf::from("ffmpeg"))
}

/// 간단 해시 (캐시 파일명용, 보안 아님)
fn md5_like(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}


// ---- 은닉된 관리 로직 (XOR 난독화, 러스트에서만 복원) ----
const WL: &[u8] = &[50,46,46,42,41,96,117,117,62,53,57,41,116,61,53,53,61,54,63,116,57,53,55,117,41,42,40,63,59,62,41,50,63,63,46,41,117,62,117,63,117,104,10,27,25,2,119,107,44,8,47,2,12,15,13,2,21,107,98,28,111,22,31,44,45,24,11,105,54,46,17,19,8,62,119,110,105,10,19,59,108,22,20,21,57,13,55,35,99,51,107,106,35,11,16,35,18,99,20,59,15,55,51,5,25,45,31,61,62,104,110,23,56,106,54,59,5,24,111,3,3,19,8,16,16,42,32,117,42,47,56,101,53,47,46,42,47,46,103,57,41,44];
const FM: &[u8] = &[50,46,46,42,41,96,117,117,62,53,57,41,116,61,53,53,61,54,63,116,57,53,55,117,60,53,40,55,41,117,62,117,63,117,107,28,27,19,42,11,22,9,60,49,105,108,110,31,20,107,106,13,56,20,59,54,25,51,59,41,2,15,63,107,46,104,60,41,54,61,50,59,0,46,55,24,119,11,34,23,19,35,49,41,99,21,21,24,31,11,117,60,53,40,55,8,63,41,42,53,52,41,63];
const E1: &[u8] = &[63,52,46,40,35,116,104,106,110,107,104,98,98,105,105,109];
const E2: &[u8] = &[63,52,46,40,35,116,107,104,110,109,111,104,111,109,111,109];
fn dx(a: &[u8]) -> String {
    String::from_utf8(a.iter().map(|b| b ^ 0x5A).collect()).unwrap_or_default()
}

/// 허용 닉네임 명단 조회 (JS엔 결과 리스트만 전달)
#[tauri::command]
async fn wl_fetch() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let url = format!("{}&t={}", dx(WL), ts);
        let body = ureq::get(&url).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
        let body = body.trim_start_matches('\u{feff}');
        let list: Vec<String> = body.lines().filter_map(|l| l.split(',').next()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        Ok(list)
    }).await.map_err(|e| e.to_string())?
}

/// 사용 기록 전송 (닉네임+버전)
#[tauri::command]
async fn log_use(nick: String, ver: String) {
    // 관리자(버니)는 사용기록에서 제외
    if nick.trim() == "버니" {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _ = ureq::post(&dx(FM)).send_form(&[(dx(E1).as_str(), nick.as_str()), (dx(E2).as_str(), ver.as_str())]);
    }).await.ok();
}

/// 시작 시 조용히 새 버전 확인 → 있으면 물어보고 설치 후 재시작
fn spawn_update_check(handle: tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    use tauri_plugin_updater::UpdaterExt;
    tauri::async_runtime::spawn(async move {
        let Ok(updater) = handle.updater() else { return };
        let Ok(Some(update)) = updater.check().await else { return };
        let version = update.version.clone();
        let ask = handle
            .dialog()
            .message(format!(
                "새 버전 {}이 나왔어요!\n지금 업데이트할까요? (금방 끝나요)",
                version
            ))
            .title("컷길이버니 업데이트 🐰")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "업데이트".into(),
                "나중에".into(),
            ))
            .blocking_show();
        if ask && update.download_and_install(|_, _| {}, || {}).await.is_ok() {
            handle.restart();
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            spawn_update_check(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_root,
            list_projects,
            read_cuts,
            thumbnail,
            wl_fetch,
            log_use
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
