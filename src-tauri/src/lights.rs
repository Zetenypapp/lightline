//! Drives the lights on its own thread at a fixed 30 Hz, following the audio clock, rendering each
//! frame from the scene with the same effect code as the preview.

use crate::analysis::Analysis;
use crate::audio::Player;
use crate::fx::{self, Ctx, Smoother};
use crate::govee;
use crate::scene::Track;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SEND_HZ: f64 = 30.0;

/// What one light plays: its tracks, and the most colours per frame it accepts.
#[derive(Clone)]
pub struct Output {
    pub ip: String,
    pub max: usize,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Default)]
pub struct Show {
    pub outputs: Vec<Output>,
    pub an: Option<Arc<Analysis>>,
    pub bpm: f64,
    pub bri: f64,    // 0..1
    pub off: f64,    // seconds the lights run late (+) or early (-)
    pub smooth: f64, // 0..100
    pub black: bool,
}

struct State {
    show: Show,
    live: Vec<(String, Instant)>,       // lights in real-time mode, since when
    play: Option<(f64, Instant, bool)>, // start time, started at, follow the audio clock
    ticks: u64,
    sm: Smoother,
}

pub struct Lights {
    st: Mutex<State>,
}

impl Lights {
    pub fn start(player: Arc<Player>) -> Arc<Self> {
        let st = State { show: Show::default(), live: vec![], play: None, ticks: 0, sm: Smoother::default() };
        let me = Arc::new(Lights { st: Mutex::new(st) });
        let l = me.clone();
        std::thread::spawn(move || {
            let (period, epoch) = (Duration::from_secs_f64(1.0 / SEND_HZ), Instant::now());
            let mut next = Instant::now();
            loop {
                l.tick(&player, epoch);
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

    pub fn set_show(&self, show: Show) {
        self.st.lock().unwrap().show = show;
    }

    pub fn play(&self, at: f64, audio: bool) {
        let mut s = self.st.lock().unwrap();
        s.play = Some((at, Instant::now(), audio));
        s.sm.reset();
    }

    pub fn stop(&self) {
        let ips = {
            let mut s = self.st.lock().unwrap();
            s.play = None;
            std::mem::take(&mut s.live).into_iter().map(|(ip, _)| ip).collect::<Vec<_>>()
        };
        set_mode(&ips, false);
    }

    fn tick(&self, player: &Player, epoch: Instant) {
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
        // Real-time mode for exactly the lights in the show (it can change while playing).
        let want: Vec<String> = s.show.outputs.iter().map(|o| o.ip.clone()).collect();
        let on: Vec<String> = want.iter().filter(|ip| !s.live.iter().any(|(x, _)| x == *ip)).cloned().collect();
        let off: Vec<String> = s.live.iter().map(|(ip, _)| ip).filter(|ip| !want.contains(ip)).cloned().collect();
        let now = Instant::now();
        s.live.retain(|(ip, _)| want.contains(ip));
        s.live.extend(on.iter().map(|ip| (ip.clone(), now)));
        s.ticks += 1;
        // One "start real-time mode" is sometimes ignored, and each one blinks the light:
        // repeat it every 0.5 s for the first 3 s only.
        let burst: Vec<String> = if s.ticks % (SEND_HZ as u64 / 2) == 0 {
            s.live.iter().filter(|(_, at)| at.elapsed() < Duration::from_secs(3)).map(|(ip, _)| ip.clone()).collect()
        } else {
            vec![]
        };
        let State { show, sm, .. } = &mut *s;
        let cx = Ctx { an: show.an.as_deref(), bpm: show.bpm };
        let k = if show.black { 0.0 } else { show.bri as f32 };
        let ms = epoch.elapsed().as_secs_f64() * 1000.0;
        let frames: Vec<(String, Vec<u8>)> = show
            .outputs
            .iter()
            .map(|o| {
                let cols = sm.step(&o.ip, &fx::light_colors(&o.tracks, t - show.off, &cx), ms, show.smooth);
                let rgb: Vec<u8> = cols.iter().flat_map(|c| c.unwrap_or([0.0; 3]).map(|v| (v * k).round() as u8)).collect();
                (o.ip.clone(), stretch(&rgb, o.max))
            })
            .collect();
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
        for (ip, rgb) in frames {
            if let Err(e) = govee::frame(&ip, &rgb) {
                eprintln!("[lights] {ip}: {e}");
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
    if w == 0 {
        return vec![0; n * 3];
    }
    (0..n).flat_map(|i| {
        let j = i * w / n * 3;
        [rgb[j], rgb[j + 1], rgb[j + 2]]
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretches_to_full_frames() {
        assert_eq!(stretch(&[1, 2, 3, 4, 5, 6], 4), vec![1, 2, 3, 1, 2, 3, 4, 5, 6, 4, 5, 6]);
        assert_eq!(stretch(&[1, 2, 3, 4, 5, 6, 7, 8, 9], 1), vec![1, 2, 3]);
        assert_eq!(stretch(&[7, 8, 9], 84).len(), 84 * 3);
    }
}
