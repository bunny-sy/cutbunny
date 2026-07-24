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
    let tracks = v["tracks"].as_array().unwrap_or(&empty);
    let main = tracks
        .iter()
        .filter(|t| t["type"] == "video")
        .max_by_key(|t| t["segments"].as_array().map(|s| s.len()).unwrap_or(0));
    let mut cuts = Vec::new();
    if let Some(track) = main {
        if let Some(segs) = track["segments"].as_array() {
            for s in segs {
                cuts.push(Cut {
                    start: s["target_timerange"]["start"].as_f64().unwrap_or(0.0) / 1e6,
                    dur: s["target_timerange"]["duration"].as_f64().unwrap_or(0.0) / 1e6,
                });
            }
        }
    }
    cuts.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    Ok(CutsResult { mtime, cuts })
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
        .invoke_handler(tauri::generate_handler![get_root, list_projects, read_cuts])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
