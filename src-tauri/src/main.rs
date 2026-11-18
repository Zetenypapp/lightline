#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod audio;
mod govee;
mod lights;

use audio::Player;
use lights::Lights;
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
async fn govee_stream(ips: Vec<String>, on: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || lights::set_mode(&ips, on)).await.map_err(|e| e.to_string())
}

/// Starts a show of `frames` frames at `fps` for these lights (ip, segments); `lights_part` fills it in.
#[tauri::command]
fn lights_load(gen: u64, fps: f64, frames: usize, lights: Vec<(String, usize)>, l: State<Arc<Lights>>) -> Result<(), String> {
    // Bounds keep a bad call from allocating gigabytes: 2 h at 120 fps, 84 segments.
    if !(1.0..=120.0).contains(&fps) || frames > 120 * 7200 || lights.iter().any(|(_, w)| !(1..=84).contains(w)) {
        return Err("Bad light show".into());
    }
    l.load(gen, fps, frames, lights);
    Ok(())
}

/// Rendered frames from frame `start`, base64, one entry per light in `lights_load` order.
#[tauri::command]
fn lights_part(gen: u64, start: usize, lights: Vec<String>, l: State<Arc<Lights>>) -> Result<(), String> {
    let data = lights.iter().map(|d| govee::unb64(d)).collect::<Result<Vec<_>, _>>()?;
    l.part(gen, start, data)
}

#[tauri::command]
fn lights_play(at: f64, audio: bool, l: State<Arc<Lights>>) {
    l.play(at, audio)
}

#[tauri::command]
async fn lights_stop(l: State<'_, Arc<Lights>>) -> Result<(), String> {
    let l = l.inner().clone();
    tauri::async_runtime::spawn_blocking(move || l.stop()).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn lights_set(bri: f64, off: f64, black: bool, l: State<Arc<Lights>>) {
    l.set(bri, off, black)
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
    let player = Player::start();
    tauri::Builder::default()
        .manage(player.clone())
        .manage(Lights::start(player))
        .invoke_handler(tauri::generate_handler![
            scan, load_scenes, save_scenes, pick_audio, load_audio, audio_play, audio_pause, audio_seek, audio_state,
            govee_stream, govee_identify, lights_load, lights_part, lights_play, lights_stop, lights_set
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lightline");
}
