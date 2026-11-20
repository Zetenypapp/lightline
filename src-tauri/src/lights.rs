//! Drives the lights on its own thread at a fixed 120 Hz (24 Hz for lights that can't keep up), following the audio clock, rendering each
//! frame from the scene with the same effect code as the preview.

use crate::analysis::Analysis;
use crate::audio::Player;
use crate::fx::{self, Ctx, Smoother};
use crate::govee;
use crate::scene::Track;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SEND_HZ: f64 = 120.0;

/// What one light plays: its tracks, and the most colours per frame it accepts.
#[derive(Clone)]
pub struct Output {
    pub ip: String,
    pub max: usize,
    pub lead: f64, // seconds this light shows frames late; it gets them that much early
    pub fast: bool, // measured to keep up with 120 frames/s; others get every 5th frame
    pub smooth: f64, // 0..100, this light's fade
    pub simple: bool, // no real-time mode (e.g. ceiling lights): one colour by the plain colour command, 10/s
    pub tracks: Vec<Track>,
    pub win: Vec<(usize, usize)>, // per track: (skip, take) segments, for tracks chained across lights
}

#[derive(Clone, Default)]
pub struct Show {
    pub outputs: Vec<Output>,
    pub an: Option<Arc<Analysis>>,
    pub bpm: f64,
    pub bri: f64,    // 0..1
    pub off: f64,    // seconds the lights run late (+) or early (-)
    pub black: bool,
    pub flash: bool, // held flash: every light full white
}

struct State {
    show: Show,
    live: Vec<(String, Instant)>,       // lights in real-time mode, since when
    play: Option<(f64, Instant, bool)>, // start time, started at, follow the audio clock
    ticks: u64,
    sm: Smoother,
    peak: HashMap<String, Vec<u8>>, // slow lights: brightest colour per segment since their last frame
    simple: HashMap<String, [u8; 3]>, // simple lights: last colour sent
}

pub struct Lights {
    st: Mutex<State>,
}

impl Lights {
    pub fn start(player: Arc<Player>) -> Arc<Self> {
        let st = State { show: Show::default(), live: vec![], play: None, ticks: 0, sm: Smoother::default(), peak: HashMap::new(), simple: HashMap::new() };
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

    fn is_simple(&self, ip: &str) -> bool {
        self.st.lock().unwrap().simple.contains_key(ip)
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
            let simple = std::mem::take(&mut s.simple);
            std::mem::take(&mut s.live).into_iter().map(|(ip, _)| ip).filter(|ip| !simple.contains_key(ip)).collect::<Vec<_>>()
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
        let want: Vec<String> = s.show.outputs.iter().filter(|o| !o.simple).map(|o| o.ip.clone()).collect();
        // Simple lights: switch them on once when they join.
        let new_simple: Vec<String> = s.show.outputs.iter().filter(|o| o.simple && !s.simple.contains_key(&o.ip)).map(|o| o.ip.clone()).collect();
        for ip in &new_simple {
            s.simple.insert(ip.clone(), [1, 2, 3]); // nothing sent yet
        }
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
        let s_ticks = s.ticks;
        let State { show, sm, peak, simple, .. } = &mut *s;
        let tenth = s_ticks % 12 == 0; // simple lights: 10 colours a second at most
        let cx = Ctx { an: show.an.as_deref(), bpm: show.bpm };
        let k = if show.black { 0.0 } else { show.bri as f32 };
        let ms = epoch.elapsed().as_secs_f64() * 1000.0;
        let even = s_ticks % 5 == 0; // slow lights: 24 frames/s (H61F6 stutters even near 30)
        let frames: Vec<(String, Vec<u8>)> = show
            .outputs
            .iter()
            .filter_map(|o| {
                let cols = sm.step(&o.ip, &fx::light_colors(&o.tracks, &o.win, t - show.off + o.lead, &cx), ms, o.smooth);
                let rgb: Vec<u8> = if show.flash { vec![255; cols.len() * 3] } else { cols.iter().flat_map(|c| c.unwrap_or([0.0; 3]).map(|v| (v * k).round() as u8)).collect() };
                let rgb = stretch(&rgb, o.max);
                if o.fast && !o.simple {
                    return Some((o.ip.clone(), rgb));
                }
                // A slow light gets every 5th frame: keep each segment's brightest colour in between,
                // so a short hit between two sent frames still shows.
                let p = peak.entry(o.ip.clone()).or_default();
                if p.len() != rgb.len() {
                    *p = rgb;
                } else {
                    keep_brightest(p, &rgb);
                }
                if !o.simple {
                    return even.then(|| (o.ip.clone(), std::mem::take(p)));
                }
                // One colour for the whole light: the average, sent only when it changes.
                let px = std::mem::take(p);
                if !tenth || px.is_empty() {
                    if !tenth { *p = px; }
                    return None;
                }
                let n = (px.len() / 3).max(1) as u32;
                let avg = [0, 1, 2].map(|k| (px.chunks(3).map(|c| c[k] as u32).sum::<u32>() / n) as u8);
                let last = simple.insert(o.ip.clone(), avg);
                (last.map_or(true, |l| l.iter().zip(avg).any(|(a, b)| a.abs_diff(b) > 4))).then(|| (o.ip.clone(), avg.to_vec()))
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
        if !new_simple.is_empty() {
            std::thread::spawn(move || new_simple.iter().for_each(|ip| { let _ = govee::turn(ip, true); }));
        }
        for (ip, rgb) in frames {
            let sent = if rgb.len() == 3 && self.is_simple(&ip) { govee::colour(&ip, [rgb[0], rgb[1], rgb[2]]) } else { govee::frame(&ip, &rgb) };
            if let Err(e) = sent {
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

/// Per segment, keeps whichever colour is brighter (never mixes two colours into a third).
fn keep_brightest(acc: &mut [u8], rgb: &[u8]) {
    for (a, b) in acc.chunks_mut(3).zip(rgb.chunks(3)) {
        if b.iter().max() > a.iter().max() {
            a.copy_from_slice(b);
        }
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

    #[test]
    fn slow_lights_keep_the_brightest() {
        let mut acc = vec![0, 0, 50, 200, 0, 0];
        keep_brightest(&mut acc, &[255, 255, 255, 0, 0, 100]);
        assert_eq!(acc, vec![255, 255, 255, 200, 0, 0]);
    }
}

/// On-light rate check: every frame toggles the whole strip on/off, so the flicker rate shows how many
/// frames per second the light really displays. Red 30, green 60, blue 120 frames/s, 4 s each.
#[test]
#[ignore]
fn rate_check() {
    // LIGHTS="ip:colours,..." picks the lights and frame size; required.
    let spec = std::env::var("LIGHTS").expect("LIGHTS=ip:colours,... (e.g. 192.168.1.20:40)");
    let lights: Vec<(&str, usize)> = spec.split(',').map(|l| l.split_once(':').map(|(ip, n)| (ip, n.parse().unwrap())).unwrap()).collect();
    let ips: Vec<String> = lights.iter().map(|l| l.0.to_string()).collect();
    set_mode(&ips, true);
    for _ in 0..6 {
        for ip in &ips {
            let _ = govee::stream_mode(ip, true);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let bri: f32 = std::env::var("BRI").map_or(1.0, |v| v.parse().unwrap()); // BRI=0.25: dim test
    for (hz, col) in [(30.0, [255u8, 0, 0]), (60.0, [0, 255, 0]), (120.0, [0, 0, 255])] {
        let col = col.map(|v| (v as f32 * bri) as u8);
        let (t0, period) = (Instant::now(), Duration::from_secs_f64(1.0 / hz));
        let (mut next, mut on) = (Instant::now(), false);
        while t0.elapsed() < Duration::from_secs(4) {
            on = !on;
            for &(ip, n) in &lights {
                let _ = govee::frame(ip, &[if on { col } else { [0, 0, 0] }].repeat(n).concat());
            }
            next += period;
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        for &(ip, n) in &lights {
            let _ = govee::frame(ip, &vec![0; n * 3]);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    set_mode(&ips, false);
}

/// Which frame sizes a light accepts: 3 s each of red, green, blue at SIZES (default 24,30,36) colours.
/// A colour that stays dark = rejected. LIGHT=ip cargo test max_colours -- --ignored
#[test]
#[ignore]
fn max_colours() {
    let ip = std::env::var("LIGHT").expect("LIGHT=ip of the light");
    set_mode(&[ip.clone()], true);
    for _ in 0..6 {
        let _ = govee::stream_mode(&ip, true);
        std::thread::sleep(Duration::from_millis(500));
    }
    let sizes: Vec<usize> = std::env::var("SIZES").unwrap_or("24,30,36".into()).split(',').map(|v| v.parse().unwrap()).collect();
    for (n, col) in sizes.into_iter().zip([[255u8, 0, 0], [0, 255, 0], [0, 0, 255]]) {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) {
            let _ = govee::frame(&ip, &col.repeat(n));
            std::thread::sleep(Duration::from_millis(33));
        }
    }
    set_mode(&[ip], false);
}
