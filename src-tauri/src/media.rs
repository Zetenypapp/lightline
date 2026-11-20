//! What any media player is playing (Spotify, browsers...), read from Windows' own media
//! controls, the same info Windows shows in its volume / media flyout.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as Manager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub app: String,
    pub pos: f64, // seconds into the song, at the moment this was read
    pub playing: bool,
}

/// 100 ns ticks since 1601, like Windows' DateTime.
fn ticks_now() -> i64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_nanos() / 100) as i64 + 116_444_736_000_000_000
}

/// The player that is playing right now (several can be open: Riff, a browser, Spotify...),
/// else the one Windows calls current.
pub fn now_playing(m: &Manager) -> Option<NowPlaying> {
    let all: Vec<NowPlaying> = m.GetSessions().ok().map(|v| v.into_iter().filter_map(|s| read(&s)).collect()).unwrap_or_default();
    all.iter().find(|n| n.playing).cloned().or_else(|| m.GetCurrentSession().ok().and_then(|s| read(&s)))
}

fn read(s: &windows::Media::Control::GlobalSystemMediaTransportControlsSession) -> Option<NowPlaying> {
    let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let tl = s.GetTimelineProperties().ok()?;
    let playing = s.GetPlaybackInfo().ok()?.PlaybackStatus().ok()? == Status::Playing;
    let mut pos = tl.Position().ok()?.Duration as f64 / 1e7;
    if playing {
        // The player reports its position at a moment; carry it forward to now.
        pos += ((ticks_now() - tl.LastUpdatedTime().ok()?.UniversalTime) as f64 / 1e7).clamp(0.0, 24.0 * 3600.0);
    }
    Some(NowPlaying {
        title: props.Title().ok()?.to_string(),
        artist: props.Artist().map(|a| a.to_string()).unwrap_or_default(),
        app: s.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default(),
        pos,
        playing,
    })
}

/// Where the Riff plugin (integrations/riff/lightline.js) posts what Riff is playing.
pub const RIFF_PORT: u16 = 47830;
pub const RIFF_PLUGIN: &str = include_str!("../../integrations/riff/lightline.js");

/// Riff's plugin folder, if Riff is installed.
pub fn riff_plugins_dir() -> Option<std::path::PathBuf> {
    let d = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join("Riff").join("plugins");
    d.is_dir().then_some(d)
}

/// Reads one plugin report: an HTTP POST whose body is {title, artist, pos, playing}.
fn read_report(mut c: std::net::TcpStream) -> Option<NowPlaying> {
    use std::io::{Read, Write};
    c.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let body_at = loop {
        let n = c.read(&mut chunk).ok()?;
        if n == 0 || buf.len() > 16 * 1024 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..body_at]).to_lowercase();
    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0).min(16 * 1024);
    while buf.len() < body_at + len {
        let n = c.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let _ = c.write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n");
    let v: serde_json::Value = serde_json::from_slice(buf.get(body_at..body_at + len)?).ok()?;
    Some(NowPlaying {
        title: v["title"].as_str()?.to_string(),
        artist: v["artist"].as_str().unwrap_or("").to_string(),
        app: "Riff".into(),
        pos: v["pos"].as_f64().unwrap_or(0.0),
        playing: v["playing"].as_bool().unwrap_or(false),
    })
}

/// Sends what's playing (or None) every 0.3 s until `stop` is set. Riff's plugin reports win while
/// they keep coming (Riff tells Windows the song but not its position); else Windows' media controls.
pub fn watch(tx: Sender<Option<NowPlaying>>, stop: Arc<AtomicBool>, wake: impl Fn() + Send + 'static) {
    let riff: Arc<std::sync::Mutex<Option<(std::time::Instant, NowPlaying)>>> = Arc::default();
    if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", RIFF_PORT)) {
        let (riff, stop) = (riff.clone(), stop.clone());
        std::thread::spawn(move || {
            for c in l.incoming() {
                if stop.load(Relaxed) {
                    return;
                }
                if let Some(np) = c.ok().and_then(read_report) {
                    *riff.lock().unwrap() = Some((std::time::Instant::now(), np));
                }
            }
        });
    }
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        let m = Manager::RequestAsync().ok().and_then(|op| op.get().ok());
        while !stop.load(Relaxed) {
            let from_riff = riff.lock().unwrap().clone().filter(|(at, _)| at.elapsed() < Duration::from_millis(1500)).map(|(at, mut np)| {
                if np.playing {
                    np.pos += at.elapsed().as_secs_f64();
                }
                np
            });
            let np = from_riff.or_else(|| m.as_ref().and_then(now_playing));
            if tx.send(np).is_err() {
                break;
            }
            wake();
            std::thread::sleep(Duration::from_millis(300));
        }
        // Unblock the listener so it sees `stop` and frees the port.
        let _ = std::net::TcpStream::connect(("127.0.0.1", RIFF_PORT));
        drop(stop2);
    });
}

/// Words of a title or file name, lower case, for matching a song to a scene.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// The scene for this song: every word of the title appears in the scene's music file name.
/// Of several, the one with the most clips (the most worked-on show).
pub fn match_scene(title: &str, songs: &[(Option<&str>, usize)]) -> Option<usize> {
    // "(Official Video)", "[4K]" and the like aren't part of the song's name.
    let mut core = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => core.push(c),
            _ => {}
        }
    }
    let t = words(&core);
    if t.is_empty() {
        return None;
    }
    songs
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| name.is_some_and(|n| {
            let w = words(n);
            t.iter().all(|x| w.contains(x))
        }))
        .max_by_key(|(_, (_, clips))| *clips)
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_titles_to_song_files() {
        let songs = [
            (Some("Papercut [Official HD Music Video] - Linkin Park.mp3"), 5),
            (Some("Vicetone - Nevada (ft. Cozi Zuehlsdorff).mp3"), 3),
            (Some("Vicetone - Nevada (ft. Cozi Zuehlsdorff).mp3"), 40),
            (None, 99),
        ];
        assert_eq!(match_scene("Papercut", &songs), Some(0));
        assert_eq!(match_scene("Nevada", &songs), Some(2), "the most worked-on scene wins");
        assert_eq!(match_scene("New Divide", &songs), None);
        assert_eq!(match_scene("", &songs), None);
        assert_eq!(match_scene("Papercut (Official Video) [4K] - Linkin Park", &songs), Some(0), "brackets ignored");
    }

    #[test]
    fn reads_a_riff_report() {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let body = r#"{"title":"Papercut","artist":"Linkin Park","pos":12.5,"playing":true}"#;
        let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(c, "POST /riff HTTP/1.1\r\nHost: x\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let np = read_report(l.accept().unwrap().0).unwrap();
        assert_eq!((np.title.as_str(), np.pos, np.playing, np.app.as_str()), ("Papercut", 12.5, true, "Riff"));
    }

    /// Prints what Windows says is playing: cargo test media_check -- --ignored --nocapture
    #[test]
    #[ignore]
    fn media_check() {
        let m = Manager::RequestAsync().unwrap().get().unwrap();
        for s in m.GetSessions().unwrap() {
            println!("session: {:?}", read(&s));
        }
        println!("chosen: {:?}", now_playing(&m));
    }
}
