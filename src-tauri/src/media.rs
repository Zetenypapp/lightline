//! What any media player is playing (Riff, Spotify, browsers...), read from Windows' own media
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

pub fn now_playing(m: &Manager) -> Option<NowPlaying> {
    let s = m.GetCurrentSession().ok()?;
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

/// Sends what's playing (or None) every 0.3 s until `stop` is set.
pub fn watch(tx: Sender<Option<NowPlaying>>, stop: Arc<AtomicBool>, wake: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let Some(m) = Manager::RequestAsync().ok().and_then(|op| op.get().ok()) else {
            let _ = tx.send(None);
            return;
        };
        while !stop.load(Relaxed) {
            if tx.send(now_playing(&m)).is_err() {
                return;
            }
            wake();
            std::thread::sleep(Duration::from_millis(300));
        }
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

    /// Prints what Windows says is playing: cargo test media_check -- --ignored --nocapture
    #[test]
    #[ignore]
    fn media_check() {
        let m = Manager::RequestAsync().unwrap().get().unwrap();
        println!("{:?}", now_playing(&m));
    }
}
