//! Plays pre-rendered light frames on its own thread with exact timing, following the
//! audio clock. Independent of the UI, so a covered or minimised window can't stutter it.

use crate::{audio::Player, govee};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SEND_HZ: f64 = 30.0;

pub struct Light {
    pub ip: String,
    pub width: usize,     // segments rendered
    pub max: usize,       // most colours the light accepts per frame: more and it rejects the frame
    pub frames: Vec<u8>,  // frame-major, width*3 bytes per frame
}

struct State {
    gen: u64, // which render the frames belong to: parts of an older render are ignored
    ticks: u64,
    fps: f64,
    lights: Vec<Light>,
    live: Vec<(String, Instant)>,       // lights in real-time mode, since when
    play: Option<(f64, Instant, bool)>, // start time, started at, follow the audio clock
    bri: f64,
    off: f64, // seconds the lights run late (+) or early (-)
    black: bool,
}

pub struct Lights {
    st: Mutex<State>,
}

impl Lights {
    pub fn start(player: Arc<Player>) -> Arc<Self> {
        let st = State { gen: 0, ticks: 0, fps: 30.0, lights: vec![], live: vec![], play: None, bri: 1.0, off: 0.0, black: false };
        let me = Arc::new(Lights { st: Mutex::new(st) });
        let l = me.clone();
        std::thread::spawn(move || {
            let period = Duration::from_secs_f64(1.0 / SEND_HZ);
            let mut next = Instant::now();
            loop {
                l.tick(&player);
                next += period;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now; // fell behind: don't burst to catch up
                }
            }
        });
        me
    }

    /// Starts a new show: dark frames until `part` fills them in.
    pub fn load(&self, gen: u64, fps: f64, frames: usize, lights: Vec<(String, usize, usize)>) {
        eprintln!("[lights] show {gen}: {frames} frames at {fps} fps, lights (ip, rendered, sent) {lights:?}");
        let mut s = self.st.lock().unwrap();
        (s.gen, s.fps) = (gen, fps);
        s.lights = lights.into_iter().map(|(ip, width, max)| Light { ip, width, max, frames: vec![0; frames * width * 3] }).collect();
    }

    /// Writes rendered frames starting at frame `start`, one byte block per light in load order.
    pub fn part(&self, gen: u64, start: usize, data: Vec<Vec<u8>>) -> Result<(), String> {
        let mut s = self.st.lock().unwrap();
        if gen != s.gen {
            return Ok(());
        }
        for (l, d) in s.lights.iter_mut().zip(data) {
            let at = start * l.width * 3;
            if d.len() % (l.width * 3) != 0 || at + d.len() > l.frames.len() {
                return Err("Light frames out of range".into());
            }
            l.frames[at..at + d.len()].copy_from_slice(&d);
        }
        Ok(())
    }

    pub fn play(&self, at: f64, audio: bool) {
        eprintln!("[lights] play at {at:.2} (audio clock: {audio})");
        self.st.lock().unwrap().play = Some((at, Instant::now(), audio));
    }

    pub fn stop(&self) {
        let ips = {
            let mut s = self.st.lock().unwrap();
            s.play = None;
            std::mem::take(&mut s.live).into_iter().map(|(ip, _)| ip).collect::<Vec<_>>()
        };
        set_mode(&ips, false);
    }

    pub fn set(&self, bri: f64, off: f64, black: bool) {
        let mut s = self.st.lock().unwrap();
        (s.bri, s.off, s.black) = (bri.clamp(0.0, 1.0), off, black);
    }

    fn tick(&self, player: &Player) {
        let mut s = self.st.lock().unwrap();
        let Some((at, since, audio)) = s.play else { return };
        let t = if audio {
            let (t, playing) = player.state();
            if !playing {
                return;
            }
            t
        } else {
            at + since.elapsed().as_secs_f64()
        };
        // Real-time mode for exactly the lights in the show (the show can change while playing).
        let want: Vec<String> = s.lights.iter().map(|l| l.ip.clone()).collect();
        let on: Vec<String> = want.iter().filter(|ip| !s.live.iter().any(|(x, _)| x == *ip)).cloned().collect();
        let off: Vec<String> = s.live.iter().map(|(ip, _)| ip).filter(|ip| !want.contains(ip)).cloned().collect();
        let now = Instant::now();
        s.live.retain(|(ip, _)| want.contains(ip));
        s.live.extend(on.iter().map(|ip| (ip.clone(), now)));
        s.ticks += 1;
        // A single "start real-time mode" is sometimes ignored, and each one makes the light blink:
        // repeat it every 0.5 s for the first 3 s only.
        let burst: Vec<String> = if s.ticks % (SEND_HZ as u64 / 2) == 0 {
            s.live.iter().filter(|(_, since)| since.elapsed() < Duration::from_secs(3)).map(|(ip, _)| ip.clone()).collect()
        } else {
            vec![]
        };
        let pos = (t - s.off) * s.fps;
        let k = if s.black { 0.0 } else { s.bri };
        let frames: Vec<(String, Vec<u8>)> = s.lights.iter().map(|l| (l.ip.clone(), stretch(&frame_at(l, pos, k), l.max))).collect();
        drop(s);
        if !on.is_empty() || !off.is_empty() {
            std::thread::spawn(move || {
                set_mode(&on, true);
                set_mode(&off, false);
            });
        }
        for ip in &burst {
            let _ = govee::stream_mode(ip, true);
        }
        let log = (pos as u64) % 30 == 0; // about once a second
        for (ip, rgb) in frames {
            let r = govee::frame(&ip, &rgb);
            if log || r.is_err() {
                eprintln!("[lights] t={t:.2} {ip}: {} segs, brightest {} {r:?}", rgb.len() / 3, rgb.iter().max().unwrap_or(&0));
            }
        }
    }
}

/// Switching on: "turn on" first and let it settle, because a "turn on" that lands after real-time mode
/// started puts a light back on its normal scene. The tick then repeats the mode for 3 s.
pub fn set_mode(ips: &[String], on: bool) {
    if ips.is_empty() {
        return;
    }
    if on {
        for ip in ips {
            let _ = govee::turn(ip, true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    for _ in 0..3 {
        for ip in ips {
            let _ = govee::stream_mode(ip, on);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// Resamples a frame to exactly `n` colours: a light rejects frames with more colours than it has
/// segments, and spreads the colours it gets over its whole strip.
fn stretch(rgb: &[u8], n: usize) -> Vec<u8> {
    let w = rgb.len() / 3;
    (0..n).flat_map(|i| { let j = i * w / n * 3; [rgb[j], rgb[j + 1], rgb[j + 2]] }).collect()
}

/// The light's colours at fractional frame `pos`, blended between the two nearest frames.
fn frame_at(l: &Light, pos: f64, k: f64) -> Vec<u8> {
    let w = l.width * 3;
    let n = l.frames.len() / w.max(1);
    if n == 0 || pos < 0.0 {
        return vec![0; w];
    }
    let i = (pos.floor() as usize).min(n - 1);
    let j = (i + 1).min(n - 1);
    let f = pos - pos.floor();
    let (a, b) = (&l.frames[i * w..(i + 1) * w], &l.frames[j * w..(j + 1) * w]);
    a.iter().zip(b).map(|(&a, &b)| ((a as f64 + (b as f64 - a as f64) * f) * k).round() as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_fill_the_current_show_only() {
        let l = Lights { st: Mutex::new(State { gen: 0, ticks: 0, fps: 30.0, lights: vec![], live: vec![], play: None, bri: 1.0, off: 0.0, black: false }) };
        l.load(2, 30.0, 3, vec![("x".into(), 1, 1)]);
        l.part(2, 1, vec![vec![9, 9, 9]]).unwrap();
        l.part(1, 0, vec![vec![5, 5, 5]]).unwrap(); // an older render: ignored
        assert_eq!(l.st.lock().unwrap().lights[0].frames, vec![0, 0, 0, 9, 9, 9, 0, 0, 0]);
        assert!(l.part(2, 2, vec![vec![1, 1, 1, 1, 1, 1]]).is_err()); // past the end
    }

    #[test]
    fn stretches_to_full_frames() {
        assert_eq!(stretch(&[1, 2, 3, 4, 5, 6], 4), vec![1, 2, 3, 1, 2, 3, 4, 5, 6, 4, 5, 6]);
        assert_eq!(stretch(&[7, 8, 9], 84).len(), 84 * 3);
    }

    #[test]
    fn blends_between_frames_and_scales() {
        let l = Light { ip: String::new(), width: 1, max: 1, frames: vec![0, 0, 0, 200, 100, 50] };
        assert_eq!(frame_at(&l, 0.5, 1.0), vec![100, 50, 25]);
        assert_eq!(frame_at(&l, 1.0, 0.5), vec![100, 50, 25]);
        assert_eq!(frame_at(&l, 9.0, 1.0), vec![200, 100, 50]); // past the end: hold the last frame
        assert_eq!(frame_at(&l, -1.0, 1.0), vec![0, 0, 0]); // before the start (timing offset): dark
    }
}
