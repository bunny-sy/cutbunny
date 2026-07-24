// 컷길이버니 — 캡컷 draft를 읽어 컷 길이를 제공하는 백엔드
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

/// 캡컷 draft 루트 자동 탐색 (globalSetting → 문서 → AppData)
fn draft_root() -> Option<PathBuf> {
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
    if let Some(up) = std::env::var_os("USERPROFILE") {
        let pb =
            PathBuf::from(&up).join("Documents\\CapCut\\User Data\\Projects\\com.lveditor.draft");
        if pb.is_dir() {
            return Some(pb);
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let pb = PathBuf::from(&local).join("CapCut\\User Data\\Projects\\com.lveditor.draft");
        if pb.is_dir() {
            return Some(pb);
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

/// 컷 썸네일 생성 (번들된 ffmpeg로 원본 영상에서 프레임 추출) → 캐시 경로 반환
#[tauri::command]
fn thumbnail(app: tauri::AppHandle, video: String, src_sec: f64) -> Result<String, String> {
    use std::process::Command;
    if video.is_empty() || !std::path::Path::new(&video).exists() {
        return Err("영상 없음".into());
    }
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(|l| PathBuf::from(l).join("CutBunny\\thumbs"))
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
    let ff = ffmpeg_path(&app)?;
    let status = Command::new(&ff)
        .args([
            "-y",
            "-v",
            "error",
            "-ss",
            &format!("{:.2}", src_sec),
            "-i",
            &video,
            "-frames:v",
            "1",
            "-vf",
            "scale=120:-1",
            &out.to_string_lossy(),
        ])
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() && out.exists() {
        // 새로 만든 jpg를 data URI로 반환 (asset 프로토콜 scope 문제 없이 확실히 표시)
        let bytes = fs::read(&out).map_err(|e| e.to_string())?;
        let b = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(format!("data:image/jpeg;base64,{}", b))
    } else {
        Err("썸네일 생성 실패".into())
    }
}

use std::os::windows::process::CommandExt;

fn ffmpeg_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    // 번들된 리소스(bin/ffmpeg.exe) 우선
    if let Ok(res) = app.path().resource_dir() {
        let p = res.join("bin").join("ffmpeg.exe");
        if p.exists() {
            return Ok(p);
        }
    }
    // 개발 중 fallback
    let dev = PathBuf::from("bin/ffmpeg.exe");
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
            thumbnail
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
