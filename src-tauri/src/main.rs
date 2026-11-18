#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod audio;
mod govee;

use audio::Player;
use serde_json::json;
use std::{fs, io::ErrorKind, path::{Path, PathBuf}, sync::Arc};
use tauri::{Manager, State};

#[tauri::command]
async fn pick_audio() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("Music", &["mp3", "wav", "flac", "ogg", "m4a", "aac"])
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// Decodes, analyses and loads the song into the player.
#[tauri::command]
async fn load_audio(path: String, player: State<'_, Arc<Player>>) -> Result<serde_json::Value, String> {
    let player = player.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (track, mono) = audio::decode(Path::new(&path))?;
        let a = analysis::analyze(&mono, track.rate);
        drop(mono);
        let duration = track.pcm.len() as f64 / 2.0 / track.rate as f64;
        player.set(track);
        Ok(json!({ "duration": duration, "bpm": a.bpm, "beats": a.beats, "downbeats": a.downbeats, "fps": a.fps, "lag": a.lag, "bands": a.bands,
            "spec": govee::b64(&a.spec), "nspec": analysis::SPEC }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn audio_play(at: f64, player: State<Arc<Player>>) -> Result<(), String> {
    player.play(at)
}

#[tauri::command]
fn audio_pause(player: State<Arc<Player>>) {
    player.pause()
}

#[tauri::command]
fn audio_seek(at: f64, player: State<Arc<Player>>) {
    player.seek(at)
}

#[tauri::command]
fn govee_stream(ips: Vec<String>, on: bool) -> Result<(), String> {
    for ip in &ips {
        if on {
            govee::turn(ip, true)?;
        }
        govee::stream_mode(ip, on)?;
    }
    Ok(())
}

#[tauri::command]
fn govee_frame(frames: Vec<(String, Vec<u8>)>) -> Result<(), String> {
    frames.iter().try_for_each(|(ip, rgb)| govee::frame(ip, rgb))
}

static BLINKING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Starts or stops blinking a light so you can tell which one it is. Stopping turns it back on.
#[tauri::command]
fn govee_identify(ip: String, on: bool) -> Result<(), String> {
    let mut set = BLINKING.lock().unwrap();
    let was = set.contains(&ip);
    set.retain(|x| x != &ip);
    if !on {
        return govee::turn(&ip, true);
    }
    set.push(ip.clone());
    if !was {
        std::thread::spawn(move || {
            let ms = std::time::Duration::from_millis(400);
            while BLINKING.lock().unwrap().contains(&ip) {
                let _ = govee::turn(&ip, false);
                std::thread::sleep(ms);
                let _ = govee::turn(&ip, true);
                std::thread::sleep(ms);
            }
        });
    }
    Ok(())
}

#[tauri::command]
fn audio_state(player: State<Arc<Player>>) -> (f64, bool) {
    player.state()
}

fn scenes_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("scenes.json"))
}

#[tauri::command]
fn load_scenes(app: tauri::AppHandle) -> Result<String, String> {
    match fs::read_to_string(scenes_path(&app)?) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok("null".into()),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
fn save_scenes(app: tauri::AppHandle, json: String) -> Result<(), String> {
    serde_json::from_str::<serde_json::Value>(&json).map_err(|e| e.to_string())?;
    let path = scenes_path(&app)?;
    // Write then rename, so a crash mid-write never leaves a half-written file.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(tmp, path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn scan() -> Result<Vec<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(govee::scan)
        .await
        .map_err(|e| e.to_string())?
}

fn main() {
    tauri::Builder::default()
        .manage(Player::start())
        .invoke_handler(tauri::generate_handler![
            scan, load_scenes, save_scenes, pick_audio, load_audio, audio_play, audio_pause, audio_seek, audio_state,
            govee_stream, govee_frame, govee_identify
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lightline");
}
