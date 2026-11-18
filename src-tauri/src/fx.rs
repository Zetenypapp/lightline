//! Effects: one renderer for both the on-screen preview and the lights.

use crate::analysis::{Analysis, SPEC};
use crate::scene::{Clip, Track};
use std::collections::HashMap;

pub type Rgb = [f32; 3];

pub const EFFECTS: [&str; 33] = [
    "Solid", "Gradient", "Chase", "Pulse", "Breathe", "Wave", "Comet", "Sparkle", "Fire", "Alternate", "Beat flash", "Strobe", "Rainbow",
    "Riser", "Converge", "Scanner", "Stack", "Heartbeat", "Colour jump",
    "Bass hit", "Spectrum", "Meter", "Visualizer", "Kick", "Voice", "Lead",
    "Ripple", "Hi-hats", "Glitch", "Snare", "Bass VU", "Drop flash", "Treble",
];

/// Effects that follow the song itself (the rest are patterns timed to the beat).
pub const REACTIVE: [&str; 14] = [
    "Bass hit", "Spectrum", "Meter", "Visualizer", "Kick", "Voice", "Lead", "Ripple", "Hi-hats", "Glitch", "Snare", "Bass VU", "Drop flash", "Treble",
];

/// Starting colours for effects that need specific ones.
pub fn default_colors(fx: &str) -> Option<([u8; 3], [u8; 3])> {
    Some(match fx {
        "Fire" => ([0xff, 0x2a, 0x00], [0xff, 0xb0, 0x20]),
        "Comet" => ([0x2f, 0xd8, 0xb4], [0xff, 0xff, 0xff]),
        "Sparkle" => ([0x1a, 0x2a, 0x6c], [0xff, 0xff, 0xff]),
        "Strobe" => ([0xff, 0xff, 0xff], [0x00, 0x00, 0x00]),
        "Meter" => ([0x2f, 0xd8, 0xb4], [0xff, 0x2a, 0x4a]),
        "Hi-hats" => ([0x1a, 0x2a, 0x6c], [0xff, 0xff, 0xff]),
        "Snare" | "Drop flash" => ([0xff, 0xff, 0xff], [0x00, 0x00, 0x00]),
        "Ripple" => ([0x2f, 0xd8, 0xb4], [0xff, 0xff, 0xff]),
        _ => return None,
    })
}

/// Song timing for effects: the detected beat grid and energy, or a plain BPM grid without a song.
pub struct Ctx<'a> {
    pub an: Option<&'a Analysis>,
    pub bpm: f64,
}

/// How hard spectrum bands `ks` just hit at frame f: the jump above their last 0.3 s, held and
/// fading over 0.15 s, 0..1.
fn band_hit(a: &Analysis, f: usize, ks: std::ops::Range<usize>) -> f32 {
    let fr = |s: f64| ((s * a.fps).round() as usize).max(1);
    let (w, d) = (fr(0.3), fr(0.15));
    let lvl = |g: usize| ks.clone().map(|k| a.spec[g * SPEC + k] as f32 / 255.0).sum::<f32>() / ks.len() as f32;
    let mut h = 0f32;
    for back in 0..d.min(f) {
        let g = f - back;
        let lo = g.saturating_sub(w);
        let avg = (lo..g).map(lvl).sum::<f32>() / (g - lo) as f32;
        h = h.max(((lvl(g) - avg) * 4.0).clamp(0.0, 1.0) * (1.0 - back as f32 / d as f32));
    }
    h
}

/// What the visualizer hears at one moment.
struct Vis {
    hit: [f32; GROUPS], // how hard each section's sound just hit, 0..1
    flash: f32,         // drop: sudden jump in loudness, 0..1
    laser: f32,         // lead note loudness, 0..1
    at: f32,            // lead note pitch (0 low .. 1 high)
}
const GROUPS: usize = SPEC; // one section per spectrum band
const DROP: f32 = 0.12; // loudness jump (0..1) that counts as a drop

impl Ctx<'_> {
    fn vis(&self, t: f64) -> Vis {
        let mut v = Vis { hit: [0.0; GROUPS], flash: 0.0, laser: 0.0, at: 0.5 };
        let Some(a) = self.an.filter(|a| !a.spec.is_empty()) else { return v };
        let nf = (a.spec.len() / SPEC).min(a.bands.len() / 3);
        let Some(f) = self.frame(a, t).filter(|&f| f < nf) else { return v };
        let fr = |s: f64| ((s * a.fps).round() as usize).max(1);
        for (gi, h) in v.hit.iter_mut().enumerate() {
            *h = band_hit(a, f, gi * SPEC / GROUPS..(gi + 1) * SPEC / GROUPS);
        }
        // Drop = the last 0.15 s much louder than the 4 s before; fades as the song settles at the new level.
        let loud = |r: std::ops::Range<usize>| r.clone().map(|g| a.bands[g * 3..g * 3 + 3].iter().map(|&b| b as f32).sum::<f32>()).sum::<f32>() / (r.len() * 3 * 255).max(1) as f32;
        if f > fr(4.0) {
            let (fast, slow) = (loud(f - fr(0.15)..f + 1), loud(f - fr(4.0)..f - fr(0.5)));
            v.flash = ((fast - slow - DROP) * 4.0).clamp(0.0, 1.0);
        }
        if let Some(l) = a.lead.get(f * 2..f * 2 + 2) {
            (v.at, v.laser) = (l[0] as f32 / 255.0, (l[1] as f32 / 255.0).powf(0.35)); // ^0.35: quiet notes still clearly visible
        }
        v
    }

    /// The analysis and the frame at t, when there is a spectrum for it.
    fn at(&self, t: f64) -> Option<(&Analysis, usize)> {
        let a = self.an.filter(|a| !a.spec.is_empty())?;
        let f = self.frame(a, t).filter(|&f| f < (a.spec.len() / SPEC).min(a.bands.len() / 3))?;
        Some((a, f))
    }

    /// Bass drum hits (43-115 Hz jumps), 0..1.
    fn kick(&self, t: f64) -> f32 {
        self.at(t).map_or(0.0, |(a, f)| band_hit(a, f, 0..4))
    }

    /// Snare hits (about 1-3 kHz jumps), 0..1.
    fn snare(&self, t: f64) -> f32 {
        self.at(t).map_or(0.0, |(a, f)| band_hit(a, f, 12..17))
    }

    /// Hi-hat / cymbal hits (the top bands), 0..1.
    fn hats(&self, t: f64) -> f32 {
        self.at(t).map_or(0.0, |(a, f)| band_hit(a, f, 19..SPEC))
    }

    /// Treble loudness right now, 0..1.
    fn treble(&self, t: f64) -> f32 {
        let Some((a, f)) = self.at(t) else { return 0.0 };
        (16..SPEC).map(|k| a.spec[f * SPEC + k] as f32).sum::<f32>() / (SPEC - 16) as f32 / 255.0
    }

    /// Kick drums in the last 0.9 s: (seconds ago, strength 0..1), for rings that spread out.
    fn kick_onsets(&self, t: f64) -> Vec<(f32, f32)> {
        let Some((a, f)) = self.at(t) else { return vec![] };
        let (w, back) = ((0.3 * a.fps) as usize, (0.9 * a.fps) as usize);
        let lvl = |g: usize| (0..4).map(|k| a.spec[g * SPEC + k] as f32).sum::<f32>() / 4.0 / 255.0;
        let rise = |g: usize| {
            let lo = g.saturating_sub(w);
            if lo == g { 0.0 } else { lvl(g) - (lo..g).map(lvl).sum::<f32>() / (g - lo) as f32 }
        };
        let g0 = f.saturating_sub(back + 1);
        let r: Vec<f32> = (g0..=f).map(rise).collect();
        (1..r.len())
            .filter(|&k| r[k] > 0.12 && r[k] >= r[k - 1] && r[k] > r.get(k + 1).copied().unwrap_or(f32::MIN))
            .map(|k| ((f - (g0 + k)) as f32 / a.fps as f32, (r[k] * 4.0).min(1.0)))
            .collect()
    }

    /// Vocal-range loudness (about 400 Hz-1.7 kHz) over the last 0.1 s, 0..1.
    fn voice(&self, t: f64) -> f32 {
        let Some((a, f)) = self.at(t) else { return 0.0 };
        let r = f.saturating_sub((0.1 * a.fps) as usize)..f + 1;
        let v = r.clone().map(|g| (9..15).map(|k| a.spec[g * SPEC + k] as f32).sum::<f32>() / 6.0 / 255.0).sum::<f32>() / r.len() as f32;
        ((v - 0.3) / 0.7).clamp(0.0, 1.0)
    }

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

/// Effects coloured by the clip's colour list (rainbow when it is empty).
pub const PALETTE_FX: [&str; 4] = ["Spectrum", "Meter", "Visualizer", "Colour jump"];

/// Colour at x (0..1) along the clip's colour list; empty list = rainbow.
fn pal_at(pal: &[[u8; 3]], x: f32) -> Rgb {
    match pal {
        [] => hsl(x * 300.0, 100.0, 50.0),
        [c] => rgb(*c),
        _ => {
            let p = x.clamp(0.0, 1.0) * (pal.len() - 1) as f32;
            let k = (p.floor() as usize).min(pal.len() - 2);
            mix(rgb(pal[k]), rgb(pal[k + 1]), p - k as f32)
        }
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
    let level = match c.fx.as_str() {
        "Kick" => cx.kick(t).powf(0.6),
        "Voice" => cx.voice(t),
        "Snare" | "Glitch" => cx.snare(t),
        "Hi-hats" => cx.hats(t),
        "Treble" => cx.treble(t),
        "Bass VU" => cx.band(t, 0),
        _ => 0.0,
    };
    let rings = if c.fx == "Ripple" { cx.kick_onsets(t) } else { vec![] };
    let lead = matches!(c.fx.as_str(), "Lead" | "Drop flash").then(|| cx.vis(t));
    let q = ((t - c.start) / c.len.max(1e-6)).clamp(0.0, 1.0) as f32; // how far into the clip
    let edge = |x: f32| x.min(1.0 - x) * 2.0; // 0 at both ends, 1 in the middle
    let vis = (c.fx == "Visualizer").then(|| (cx.vis(t), frac(cx.beat_pos(t).floor() * 0.13) as f32)); // colours shift each beat
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
                "Spectrum" => lvl(pal_at(&c.pal, x), cx.spec(t, x).powf(1.5)), // ^1.5: quiet bands drop out, peaks pop
                "Meter" => if x <= (cx.band(t, 0) + cx.band(t, 1) + cx.band(t, 2)) / 3.0 { pal_at(&c.pal, x) } else { [0.0; 3] },
                // Burst from the centre on each kick / bar from the centre that widens with the vocals.
                "Kick" | "Voice" => {
                    let d = if n == 1 { 0.0 } else { (x - 0.5).abs() * 2.0 }; // 1 segment: just its brightness
                    if d <= level && level > 0.02 { lvl(mix(a, b2, d / level), 0.3 + 0.7 * level) } else { [0.0; 3] }
                }
                // Just the lead note: a bright dot at its pitch with a soft halo.
                "Lead" => {
                    let v = lead.as_ref().unwrap();
                    let dist = (i as f32 - v.at * (n - 1) as f32).abs();
                    lvl(mix(lvl(b2, (1.0 - dist / 4.0).max(0.0) * 0.35), a, (1.0 - dist).max(0.0)), v.laser)
                }
                // --- patterns ---
                "Riser" => if edge(x) <= q { lvl(mix(a, b2, q), 0.25 + 0.75 * q) } else { [0.0; 3] },
                "Converge" => {
                    let f4 = frac(b / 4.0) as f32;
                    let (e, pos) = (x.min(1.0 - x), f4 * 0.5);
                    let head = (1.0 - (e - pos).abs() * n as f32 / 2.0).max(0.0);
                    let tail = if e <= pos { (1.0 - (pos - e) * 4.0).max(0.0) * 0.5 } else { 0.0 };
                    screen(Some(lvl(a, head.max(tail))), Some(lvl(b2, (-f4 * 12.0).exp()))).unwrap() // burst as they meet
                }
                "Scanner" => {
                    let pos = 1.0 - (2.0 * frac(b * 0.5) as f32 - 1.0).abs();
                    let d = (x - pos).abs();
                    screen(Some(lvl(a, (1.0 - d * n as f32 / 3.0).max(0.0))), Some(lvl(b2, (-d * 6.0).exp() * 0.35))).unwrap()
                }
                "Stack" => {
                    let f8 = frac(b / 4.0) * 8.0;
                    let (idx, sub, bs) = (f8.floor() as usize, frac(f8), (n / 8).max(1));
                    let blk = |k: usize| if k % 2 == 0 { a } else { b2 };
                    let jb = (n - 1 - i) / bs;
                    if jb < idx {
                        blk(jb)
                    } else {
                        let top = (sub * n.saturating_sub((idx + 1) * bs) as f64) as usize;
                        if i >= top && i < top + bs { blk(idx) } else { [0.0; 3] }
                    }
                }
                "Heartbeat" => {
                    let second = if fb > 0.28 { 0.75 * (-(fb - 0.28) * 14.0).exp() } else { 0.0 };
                    lvl(a, (-fb * 14.0).exp().max(second))
                }
                "Colour jump" => {
                    let k = b.floor();
                    let col = if c.pal.is_empty() { hsl(k as f32 * 67.0, 100.0, 50.0) } else { rgb(c.pal[(k.rem_euclid(c.pal.len() as f64)) as usize]) };
                    lvl(col, 1.0 - 0.35 * fb)
                }
                // --- react to the song ---
                "Ripple" => {
                    let d = if n == 1 { 0.0 } else { (x - 0.5).abs() * 2.0 };
                    let v = rings.iter().map(|&(age, s)| s * (1.0 - age / 0.9).max(0.0) * (-((d - age * 1.6) / 0.08).powi(2)).exp()).fold(0.0, f32::max);
                    lvl(mix(a, b2, d), v)
                }
                "Hi-hats" => if rnd(i as f64, (p * 16.0).floor()) > 1.0 - level as f64 * 0.6 { b2 } else { lvl(a, 0.08) },
                "Glitch" => {
                    let (blk, step) = ((i / 4) as f64, (t * 24.0).floor());
                    if rnd(blk, step) < level as f64 * 0.8 { if rnd(blk + 0.5, step) > 0.5 { a } else { b2 } } else { [0.0; 3] }
                }
                "Snare" => lvl(a, level.powf(0.8)),
                "Bass VU" => if level > 0.02 && edge(x) <= level { mix(a, b2, edge(x) / level) } else { [0.0; 3] },
                "Drop flash" => lvl(a, lead.as_ref().unwrap().flash),
                "Treble" => lvl(mix(a, b2, x), level.powf(1.2)),
                "Visualizer" => {
                    let (v, shift) = vis.as_ref().unwrap();
                    let g = ((x * GROUPS as f32) as usize).min(GROUPS - 1);
                    let sec = lvl(pal_at(&c.pal, (g as f32 / GROUPS as f32 + shift).fract()), v.hit[g].powf(0.7)); // dark until hit
                    // The lead note: 1-2 segments at its pitch, white, over the sections.
                    let beam = (1.0 - (i as f32 - v.at * (n - 1) as f32).abs()).max(0.0) * v.laser;
                    mix(mix(sec, [255.0; 3], beam), [255.0; 3], v.flash)
                }
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

/// One light's colours: each track lands at its start segment. Where tracks overlap, the higher
/// track (earlier in the list) wins; its dark parts let the tracks below show through.
pub fn light_colors(tracks: &[Track], t: f64, cx: &Ctx) -> Vec<Option<Rgb>> {
    let w = tracks.iter().map(|tr| tr.from + tr.segs).max().unwrap_or(0).clamp(1, 84);
    let mut out = vec![None; w];
    for tr in tracks.iter().rev() {
        for (i, c) in track_colors(tr, t, cx).into_iter().enumerate() {
            if let Some(o) = out.get_mut(tr.from + i) {
                *o = over(c, *o);
            }
        }
    }
    out
}

/// `top` over `below`, using top's brightness as its opacity.
fn over(top: Option<Rgb>, below: Option<Rgb>) -> Option<Rgb> {
    match (top, below) {
        (Some(a), Some(b)) => {
            let k = 1.0 - a.iter().copied().fold(0.0, f32::max) / 255.0;
            Some([0, 1, 2].map(|i| a[i] + b[i] * k))
        }
        (a, b) => a.or(b),
    }
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
        Clip { fx: fx.into(), start: 0.0, len: 4.0, col, col2: [124, 108, 255], bri: 100.0, speed: 2.0, rev: true, fin: 1.0, fout: 1.0, pal: vec![] }
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
        assert_eq!(px, vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]], "higher track wins where they overlap");
        let black = Clip { col: [0, 0, 0], ..red.clone() };
        let out = light_colors(&[t(0, &black), t(0, &blue)], 1.0, &cx);
        assert_eq!(out[0].unwrap().map(|v| v.round() as u8), [0, 0, 255], "dark parts of the top track show the one below");
    }

    #[test]
    fn colour_list() {
        let p = [[255, 0, 0], [0, 0, 255]];
        assert_eq!(pal_at(&p, 0.0), [255.0, 0.0, 0.0]);
        assert_eq!(pal_at(&p, 0.5), [127.5, 0.0, 127.5]);
        assert_eq!(pal_at(&p, 1.0), [0.0, 0.0, 255.0]);
        assert_eq!(pal_at(&[], 0.0), hsl(0.0, 100.0, 50.0));
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

/// Prints what the visualizer does on a real song: SONG=path FROM=secs cargo test vis_on_song -- --ignored --nocapture
#[test]
#[ignore]
fn vis_on_song() {
    let path = std::env::var("SONG").unwrap();
    let from: f64 = std::env::var("FROM").map_or(60.0, |v| v.parse().unwrap());
    let (track, mono) = crate::audio::decode(std::path::Path::new(&path)).unwrap();
    let an = crate::analysis::analyze(&mono, track.rate);
    let cx = Ctx { an: Some(&an), bpm: 120.0 };
    let clip = Clip { fx: "Visualizer".into(), start: 0.0, len: 999.0, col: [0; 3], col2: [0; 3], bri: 100.0, speed: 1.0, rev: false, fin: 0.0, fout: 0.0, pal: vec![] };
    let mut flashes = 0;
    for i in 0..(mono.len() as f64 / track.rate as f64 * 20.0) as usize {
        let t = i as f64 / 20.0;
        let v = cx.vis(t);
        if v.flash > 0.5 { flashes += 1; }
        if t >= from && t < from + 12.0 {
            let row: String = clip_colors(&clip, 48, t, &cx).iter().map(|c| [' ', '.', ':', '*', '#'][((c[0] + c[1] + c[2]) / 3.0 / 52.0) as usize]).collect();
            println!("{t:6.2} |{row}| flash {:.2} lead {:.2} {:.2}", v.flash, v.at, v.laser);
        }
    }
    println!("frames with drop flash: {flashes} of {}", (mono.len() as f64 / track.rate as f64 * 20.0) as usize);
}

/// Text render of one light in a saved scene: SCENES=file SCENE=n(1-based) DEV=id FROM=s TO=s cargo test light_on_song -- --ignored --nocapture
/// W white, L lime, G/g green (bright/dim), o other, . dim, space off.
#[test]
#[ignore]
fn light_on_song() {
    let env = |k: &str| std::env::var(k).unwrap();
    let scenes: Vec<crate::scene::Scene> = serde_json::from_str(&std::fs::read_to_string(env("SCENES")).unwrap()).unwrap();
    let sc = &scenes[env("SCENE").parse::<usize>().unwrap() - 1];
    let (track, mono) = crate::audio::decode(std::path::Path::new(&sc.audio.as_ref().unwrap().path)).unwrap();
    let an = crate::analysis::analyze(&mono, track.rate);
    let cx = Ctx { an: Some(&an), bpm: sc.bpm };
    let tracks: Vec<Track> = sc.tracks.iter().filter(|t| t.dev.as_deref() == Some(&env("DEV"))).cloned().collect();
    let (from, to): (f64, f64) = (env("FROM").parse().unwrap(), env("TO").parse().unwrap());
    let mut t = from;
    while t < to {
        let row: String = light_colors(&tracks, t, &cx).iter().map(|c| {
            let [r, g, b] = c.unwrap_or([0.0; 3]);
            let m = r.max(g).max(b);
            if m < 20.0 { ' ' } else if m < 60.0 { '.' } else if r.min(g).min(b) > 170.0 { 'W' } else if r > 110.0 && g > 140.0 && b < 90.0 { 'L' } else if g > r && g >= b { if m > 140.0 { 'G' } else { 'g' } } else { 'o' }
        }).collect();
        println!("{t:6.2} |{row}|");
        t += 0.1;
    }
}
