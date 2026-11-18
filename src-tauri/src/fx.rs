//! Effects: one renderer for both the on-screen preview and the lights.

use crate::analysis::{Analysis, SPEC};
use crate::scene::{Clip, Track};
use std::collections::HashMap;

pub type Rgb = [f32; 3];

pub const EFFECTS: [&str; 16] = [
    "Solid", "Gradient", "Chase", "Pulse", "Breathe", "Wave", "Comet", "Sparkle", "Fire", "Alternate", "Beat flash", "Strobe", "Rainbow", "Bass hit", "Spectrum", "Meter",
];

/// Starting colours for effects that need specific ones.
pub fn default_colors(fx: &str) -> Option<([u8; 3], [u8; 3])> {
    Some(match fx {
        "Fire" => ([0xff, 0x2a, 0x00], [0xff, 0xb0, 0x20]),
        "Comet" => ([0x2f, 0xd8, 0xb4], [0xff, 0xff, 0xff]),
        "Sparkle" => ([0x1a, 0x2a, 0x6c], [0xff, 0xff, 0xff]),
        "Strobe" => ([0xff, 0xff, 0xff], [0x00, 0x00, 0x00]),
        "Meter" => ([0x2f, 0xd8, 0xb4], [0xff, 0x2a, 0x4a]),
        _ => return None,
    })
}

/// Song timing for effects: the detected beat grid and energy, or a plain BPM grid without a song.
pub struct Ctx<'a> {
    pub an: Option<&'a Analysis>,
    pub bpm: f64,
}

impl Ctx<'_> {
    fn beat(&self) -> f64 {
        60.0 / self.an.map(|a| a.bpm).filter(|b| *b > 0.0).unwrap_or(self.bpm).max(1.0)
    }

    /// Fractional beat index at time t.
    pub fn beat_pos(&self, t: f64) -> f64 {
        let b = self.an.map(|a| &a.beats[..]).unwrap_or(&[]);
        if b.len() < 2 {
            return t / self.beat();
        }
        let hi = b.len() - 1;
        if t <= b[0] {
            return (t - b[0]) / (b[1] - b[0]);
        }
        if t >= b[hi] {
            return hi as f64 + (t - b[hi]) / (b[hi] - b[hi - 1]);
        }
        let i = b.partition_point(|&x| x <= t) - 1;
        i as f64 + (t - b[i]) / (b[i + 1] - b[i])
    }

    /// Nearest beat to t (for snapping).
    pub fn snap(&self, t: f64) -> f64 {
        let b = self.an.map(|a| &a.beats[..]).unwrap_or(&[]);
        if b.is_empty() {
            return (t / self.beat()).round() * self.beat();
        }
        b[(self.beat_pos(t).round().max(0.0) as usize).min(b.len() - 1)]
    }

    fn frame(&self, a: &Analysis, t: f64) -> Option<usize> {
        let f = ((t - a.lag) * a.fps).floor();
        (f >= 0.0).then_some(f as usize)
    }

    /// Bass (0), mid (1), high (2) level 0..1.
    pub fn band(&self, t: f64, k: usize) -> f32 {
        let Some(a) = self.an else { return 0.5 };
        self.frame(a, t).and_then(|f| a.bands.get(f * 3 + k)).map_or(0.0, |&v| v as f32 / 255.0)
    }

    /// Spectrum level at x (0 = deepest bass, 1 = treble), blended between bands.
    fn spec(&self, t: f64, x: f32) -> f32 {
        let Some(a) = self.an.filter(|a| !a.spec.is_empty()) else { return 0.5 + 0.5 * (x * 9.0 + t as f32 * 4.0).sin() };
        let Some(f) = self.frame(a, t) else { return 0.0 };
        let pos = x * (SPEC - 1) as f32;
        let k = pos.floor() as usize;
        let at = |k: usize| a.spec.get(f * SPEC + k.min(SPEC - 1)).map_or(0.0, |&v| v as f32);
        (at(k) + (at(k + 1) - at(k)) * (pos - k as f32)) / 255.0
    }
}

pub fn hsl(h: f32, s: f32, l: f32) -> Rgb {
    let (h, s, l) = (h.rem_euclid(360.0), s / 100.0, l / 100.0);
    let a = s * l.min(1.0 - l);
    let f = |n: f32| {
        let k = (n + h / 30.0) % 12.0;
        255.0 * (l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0))
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn rgb(c: [u8; 3]) -> Rgb { [c[0] as f32, c[1] as f32, c[2] as f32] }
fn mix(a: Rgb, b: Rgb, f: f32) -> Rgb { [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f] }
fn lvl(c: Rgb, f: f32) -> Rgb { [c[0] * f, c[1] * f, c[2] * f] }
fn frac(v: f64) -> f64 { v.rem_euclid(1.0) }
/// Stable pseudo-random value per segment and time step.
fn rnd(a: f64, b: f64) -> f64 { frac((a * 12.9898 + b * 78.233).sin() * 43758.5453) }

/// Light blend ("screen"): colours add up without clipping harshly; None = nothing there.
pub fn screen(a: Option<Rgb>, b: Option<Rgb>) -> Option<Rgb> {
    match (a, b) {
        (Some(a), Some(b)) => Some([0, 1, 2].map(|k| 255.0 - (255.0 - a[k]) * (255.0 - b[k]) / 255.0)),
        (a, b) => a.or(b),
    }
}

fn clip_colors(c: &Clip, n: usize, t: f64, cx: &Ctx) -> Vec<Rgb> {
    let (a, b2) = (rgb(c.col), rgb(c.col2));
    let p = (t - c.start) * c.speed;
    let b = (cx.beat_pos(t) - cx.beat_pos(c.start)) * c.speed;
    let fb = frac(b) as f32;
    let fade_in = if c.fin > 0.0 { (t - c.start) / c.fin } else { 1.0 };
    let fade_out = if c.fout > 0.0 { (c.start + c.len - t) / c.fout } else { 1.0 };
    let env = (c.bri / 100.0 * fade_in.min(fade_out).min(1.0)).max(0.0) as f32;
    (0..n)
        .map(|j| {
            let i = if c.rev { n - 1 - j } else { j };
            let x = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
            let o = match c.fx.as_str() {
                "Gradient" => mix(a, b2, x),
                "Chase" => if (i as i64 - (b * 4.0).floor() as i64).rem_euclid(4) == 0 { a } else { lvl(b2, 0.15) },
                "Pulse" => lvl(a, 0.12 + 0.88 * (1.0 - fb)),
                "Breathe" => lvl(a, 0.04 + 0.96 * (0.5 - 0.5 * (std::f64::consts::PI * b / 2.0).cos() as f32)),
                "Wave" => lvl(mix(a, b2, x), 0.08 + 0.92 * (0.5 + 0.5 * (std::f64::consts::TAU * (2.0 * x as f64 - b / 2.0)).sin() as f32)),
                "Comet" => {
                    let d = frac(b / 4.0 - i as f64 / n as f64) as f32 * n as f32;
                    if d < 1.0 { b2 } else { lvl(a, (1.0 - d / (n as f32 / 4.0)).max(0.03)) }
                }
                "Sparkle" => if rnd(i as f64, (p * 8.0).floor()) > 0.85 { b2 } else { lvl(a, 0.12) },
                "Fire" => {
                    let f = (0.5 * rnd(i as f64, (p * 10.0).floor()) + 0.5 * rnd(i as f64 + 0.5, (p * 4.0).floor())) as f32;
                    lvl(mix(a, b2, f), 0.35 + 0.65 * f)
                }
                "Alternate" => if (i as i64 + b.floor() as i64).rem_euclid(2) == 1 { a } else { b2 },
                "Beat flash" => lvl(if (b.floor() as i64).rem_euclid(2) == 1 { b2 } else { a }, (-fb * 5.0).exp()),
                "Strobe" => if frac(b * 4.0) < 0.5 { a } else { [0.0; 3] },
                "Rainbow" => hsl(i as f32 * 24.0 + p as f32 * 90.0, 90.0, 55.0),
                "Bass hit" => lvl(a, cx.band(t, 0)),
                "Spectrum" => lvl(mix(a, b2, x), cx.spec(t, x).powf(1.5)), // ^1.5: quiet bands drop out, peaks pop
                "Meter" => if x <= (cx.band(t, 0) + cx.band(t, 1) + cx.band(t, 2)) / 3.0 { mix(a, b2, x) } else { [0.0; 3] },
                _ => a,
            };
            o.map(|v| (v * env).clamp(0.0, 255.0))
        })
        .collect()
}

/// All clips playing on a track at time t, layered.
pub fn track_colors(tr: &Track, t: f64, cx: &Ctx) -> Vec<Option<Rgb>> {
    let n = tr.segs.max(1);
    let mut out = vec![None; n];
    for c in tr.clips.iter().filter(|c| t >= c.start && t < c.start + c.len) {
        for (o, col) in out.iter_mut().zip(clip_colors(c, n, t, cx)) {
            *o = screen(*o, Some(col));
        }
    }
    out
}

/// One light's colours: each track lands at its start segment, overlapping tracks layer.
pub fn light_colors(tracks: &[Track], t: f64, cx: &Ctx) -> Vec<Option<Rgb>> {
    let w = tracks.iter().map(|tr| tr.from + tr.segs).max().unwrap_or(0).clamp(1, 84);
    let mut out = vec![None; w];
    for tr in tracks {
        for (i, c) in track_colors(tr, t, cx).into_iter().enumerate() {
            if let Some(o) = out.get_mut(tr.from + i) {
                *o = screen(*o, c);
            }
        }
    }
    out
}

/// Glides colour changes so jumps don't look harsh: rises in ~20 ms, falls over the smoothing time.
/// Time-based, so it looks the same at any frame rate.
#[derive(Default)]
pub struct Smoother {
    st: HashMap<String, (Vec<Rgb>, f64)>,
}

impl Smoother {
    pub fn reset(&mut self) {
        self.st.clear();
    }

    /// `smooth` 0..100 (fade = smooth * 3 ms, 0 = raw); `now` in ms.
    pub fn step(&mut self, key: &str, cols: &[Option<Rgb>], now: f64, smooth: f64) -> Vec<Option<Rgb>> {
        let fall = smooth * 3.0;
        match self.st.get_mut(key) {
            Some((v, last)) if v.len() == cols.len() => {
                let dt = (now - *last).max(0.0);
                *last = now;
                let (up, down) = if fall > 0.0 { (1.0 - (-dt / 20.0).exp(), 1.0 - (-dt / fall).exp()) } else { (1.0, 1.0) };
                for (o, c) in v.iter_mut().zip(cols) {
                    let c = c.unwrap_or([0.0; 3]);
                    for k in 0..3 {
                        o[k] += (c[k] - o[k]) * if c[k] > o[k] { up } else { down } as f32;
                    }
                }
                v.iter().zip(cols).map(|(o, c)| if c.is_none() && o.iter().all(|x| *x < 2.0) { None } else { Some(*o) }).collect()
            }
            _ => {
                self.st.insert(key.into(), (cols.iter().map(|c| c.unwrap_or([0.0; 3])).collect(), now));
                cols.to_vec()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Clip, Track};

    fn clip(fx: &str, col: [u8; 3]) -> Clip {
        Clip { fx: fx.into(), start: 0.0, len: 4.0, col, col2: [124, 108, 255], bri: 100.0, speed: 2.0, rev: true, fin: 1.0, fout: 1.0 }
    }

    #[test]
    fn every_effect_gives_valid_colours() {
        let cx = Ctx { an: None, bpm: 120.0 };
        for fx in EFFECTS {
            for n in [1, 15, 84] {
                for t in [0.0, 0.37, 3.9] {
                    let tr = Track { name: String::new(), segs: n, from: 0, dev: None, clips: vec![clip(fx, [47, 216, 180])] };
                    let out = track_colors(&tr, t, &cx);
                    assert_eq!(out.len(), n);
                    assert!(out.iter().flatten().all(|c| c.iter().all(|v| (0.0..=255.0).contains(v))), "{fx}");
                }
            }
        }
    }

    #[test]
    fn layering_and_zones() {
        let cx = Ctx { an: None, bpm: 120.0 };
        let mut red = clip("Solid", [255, 0, 0]);
        let mut blue = clip("Solid", [0, 0, 255]);
        for c in [&mut red, &mut blue] {
            (c.fin, c.fout) = (0.0, 0.0);
        }
        let t = |from, c: &Clip| Track { name: String::new(), segs: 2, from, dev: None, clips: vec![c.clone()] };
        let out = light_colors(&[t(0, &red), t(1, &blue)], 1.0, &cx);
        let px: Vec<[u8; 3]> = out.iter().map(|c| c.unwrap().map(|v| v.round() as u8)).collect();
        assert_eq!(px, vec![[255, 0, 0], [255, 0, 255], [0, 0, 255]]);
    }

    #[test]
    fn smoothing_fades_down_fast_up() {
        let mut s = Smoother::default();
        s.step("a", &[Some([255.0, 0.0, 0.0])], 0.0, 40.0);
        let mid = s.step("a", &[Some([0.0; 3])], 33.0, 40.0)[0].unwrap()[0];
        let later = s.step("a", &[Some([0.0; 3])], 400.0, 40.0)[0].unwrap()[0];
        assert!(mid > 150.0 && mid < 255.0 && later < 10.0, "{mid} {later}");
        let mut raw = Smoother::default();
        raw.step("b", &[Some([255.0, 0.0, 0.0])], 0.0, 0.0);
        assert_eq!(raw.step("b", &[Some([0.0; 3])], 33.0, 0.0)[0].unwrap()[0], 0.0);
    }
}
