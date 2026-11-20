//! Effects: one renderer for both the on-screen preview and the lights.

use crate::analysis::{Analysis, SPEC};
use crate::scene::{Clip, Track};
use std::collections::HashMap;

pub type Rgb = [f32; 3];

pub const EFFECTS: [&str; 60] = [
    "Solid", "Gradient", "Chase", "Pulse", "Breathe", "Wave", "Comet", "Sparkle", "Fire", "Alternate", "Beat flash", "Strobe", "Rainbow",
    "Riser", "Converge", "Scanner", "Stack", "Heartbeat", "Colour jump",
    "Plasma", "Meteor shower", "Ping-pong", "Colour wipe", "Twinkle", "Police",
    "Lava lamp", "Aurora", "Matrix rain", "Candle", "DNA", "Theatre chase",
    "Bass hit", "Spectrum", "Meter", "Visualizer", "Kick", "Voice", "Lead",
    "Ripple", "Hi-hats", "Glitch", "Snare", "Bass VU", "Drop flash", "Treble",
    "Fireworks", "Lightning", "EQ bars", "Bouncing ball", "Sparks", "Energy",
    "Vocal colour", "Drop countdown", "Bass wobble", "Clap",
    "Kick chase", "Pitch trail", "Split bands", "Tug of war", "Heat",
];

/// Effects that follow the song itself (the rest are patterns timed to the beat).
pub const REACTIVE: [&str; 29] = [
    "Bass hit", "Spectrum", "Meter", "Visualizer", "Kick", "Voice", "Lead", "Ripple", "Hi-hats", "Glitch", "Snare", "Bass VU", "Drop flash", "Treble",
    "Fireworks", "Lightning", "EQ bars", "Bouncing ball", "Sparks", "Energy",
    "Vocal colour", "Drop countdown", "Bass wobble", "Clap",
    "Kick chase", "Pitch trail", "Split bands", "Tug of war", "Heat",
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
        "Ripple" | "Meteor shower" | "Drop countdown" => ([0x2f, 0xd8, 0xb4], [0xff, 0xff, 0xff]),
        "Fireworks" => ([0xff, 0x6a, 0x10], [0xc0, 0x30, 0xff]),
        "Lightning" => ([0x90, 0xa8, 0xff], [0xff, 0xff, 0xff]),
        "Twinkle" => ([0xff, 0xd0, 0x80], [0xa0, 0xc8, 0xff]),
        "Police" => ([0xff, 0x00, 0x00], [0x00, 0x30, 0xff]),
        "Lava lamp" => ([0xff, 0x20, 0x60], [0xff, 0x90, 0x00]),
        "Aurora" => ([0x10, 0xff, 0x90], [0x80, 0x30, 0xff]),
        "Matrix rain" => ([0x00, 0xff, 0x40], [0xd0, 0xff, 0xd0]),
        "Candle" => ([0xff, 0x50, 0x00], [0xff, 0xc0, 0x50]),
        "Heat" => ([0x40, 0x00, 0xa0], [0xff, 0x30, 0x00]),
        "Tug of war" => ([0xff, 0x20, 0x40], [0x20, 0x80, 0xff]),
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

    /// Kick drums in the last 0.9 s: (seconds ago, strength 0..1, frame it hit at), for rings that spread out.
    fn kick_onsets(&self, t: f64) -> Vec<(f32, f32, usize)> {
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
            .map(|k| ((f - (g0 + k)) as f32 / a.fps as f32, (r[k] * 4.0).min(1.0), g0 + k + (a.lag * a.fps).round() as usize))
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
        let nf = (a.bands.len() / 3).max(1);
        // Just past the newest frame (live input still arriving): use the newest one.
        (f >= 0.0).then_some((f as usize).min(if (f as usize) < nf + (a.fps * 0.25) as usize { nf - 1 } else { usize::MAX }))
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
pub const PALETTE_FX: [&str; 7] = ["Spectrum", "Meter", "Visualizer", "Colour jump", "EQ bars", "Colour wipe", "Vocal colour"];

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

/// A clip's colours on n segments, with its repeat, mirror and "reacts to" applied.
fn clip_colors(c: &Clip, n: usize, t: f64, cx: &Ctx) -> Vec<Rgb> {
    let span = n.div_ceil((c.repeat.max(1) as usize).min(n.max(1)));
    let base = pattern(c, if c.mirror { span.div_ceil(2) } else { span }, t, cx);
    let m = match c.react {
        1 => 0.12 + 0.88 * cx.kick(t).powf(0.6),
        2 => cx.band(t, 0),
        3 => cx.voice(t),
        4 => cx.treble(t),
        _ => 1.0,
    };
    (0..n)
        .map(|j| {
            let k = j % span;
            let k = if c.mirror { (2 * k + 1).abs_diff(span) / 2 } else { k }; // 0 in the middle, out to both sides
            lvl(base[k.min(base.len() - 1)], m)
        })
        .collect()
}

/// The effect itself on n segments.
fn pattern(c: &Clip, n: usize, t: f64, cx: &Ctx) -> Vec<Rgb> {
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
        "Snare" | "Glitch" | "Clap" => cx.snare(t),
        "Vocal colour" => cx.voice(t),
        "Bass wobble" => cx.band(t, 0),
        "Hi-hats" => cx.hats(t),
        "Treble" => cx.treble(t),
        "Bass VU" => cx.band(t, 0),
        "Lightning" => cx.snare(t).max(cx.vis(t).flash),
        // Overall loudness over the last 0.4 s: smooth, no flicker.
        "Energy" | "Heat" => (0..8).map(|k| (0..3).map(|j| cx.band(t - k as f64 * 0.05, j)).sum::<f32>() / 3.0).sum::<f32>() / 8.0,
        _ => 0.0,
    };
    let rings = if matches!(c.fx.as_str(), "Ripple" | "Fireworks" | "Bouncing ball" | "Kick chase") { cx.kick_onsets(t) } else { vec![] };
    // Without a song: a kick every beat, so the effect still shows something.
    let rings = if cx.an.is_none() && rings.is_empty() && c.fx != "Ripple" { let bp = cx.beat_pos(t); vec![((frac(bp) * cx.beat()) as f32, 1.0, bp.floor() as usize)] } else { rings };
    let lead = matches!(c.fx.as_str(), "Lead" | "Drop flash" | "Sparks" | "Vocal colour").then(|| cx.vis(t));
    // Drop countdown: (fill over the 4 beats before the next chorus, flash just after one starts).
    // No song sections (live, or none found): the clip's own end is the drop.
    let drop = (c.fx == "Drop countdown").then(|| {
        let ch = cx.an.map(|a| a.sections.iter().filter(|s| s.1 == crate::analysis::Part::Chorus).map(|s| s.0).collect::<Vec<_>>()).unwrap_or_default();
        let next = ch.iter().copied().find(|&s| s > t).unwrap_or(c.start + c.len);
        let prev = ch.iter().copied().filter(|&s| s <= t).last().unwrap_or(f64::MIN);
        ((1.0 - (next - t) / (4.0 * cx.beat())).clamp(0.0, 1.0) as f32, (-(t - prev) * 3.0).exp() as f32)
    });
    // Pitch trail: where the lead note was over the last 0.3 s (pitch 0..1, loudness 0..1), newest first.
    let trail: Vec<(f32, f32)> = if c.fx != "Pitch trail" { vec![] } else {
        (0..10).filter_map(|k| {
            let (a, f) = cx.at(t - k as f64 * 0.03)?;
            let l = a.lead.get(f * 2..f * 2 + 2)?;
            Some((l[0] as f32 / 255.0, (l[1] as f32 / 255.0).powf(0.35) * (1.0 - k as f32 / 10.0)))
        }).collect()
    };
    let bars = (c.fx == "EQ bars").then(|| (0..8).map(|k| cx.spec(t, (k as f32 + 0.5) / 8.0).powf(1.3)).collect::<Vec<_>>());
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
                    let v = rings.iter().map(|&(age, s, _)| s * (1.0 - age / 0.9).max(0.0) * (-((d - age * 1.6) / 0.08).powi(2)).exp()).fold(0.0, f32::max);
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
                // A burst at a random spot per kick, spreading out and fading.
                "Fireworks" => rings.iter().fold([0.0; 3], |o, &(age, s, g)| {
                    let (at, life) = (0.15 + 0.7 * rnd(g as f64, 3.0) as f32, (1.0 - age / 0.9).max(0.0));
                    let d = if n == 1 { 0.0 } else { (x - at).abs() };
                    let v = s * life * life * ((-((d - age * 0.5) / 0.05).powi(2)).exp() + (-d / 0.04).exp() * (1.0 - age * 4.0).max(0.0));
                    screen(Some(o), Some(lvl(mix(a, b2, rnd(g as f64, 7.0) as f32), v.min(1.0)))).unwrap()
                }),
                // A jagged streak over a random stretch on each snare / drop, flickering.
                "Lightning" => {
                    let step = (t * 14.0).floor();
                    let (from, len) = (rnd(step, 1.0) as f32 * 0.6, 0.25 + 0.4 * rnd(step, 2.0) as f32);
                    let on = n == 1 || (x >= from && x <= from + len && rnd((i / 2) as f64, step) > 0.3);
                    if on && level > 0.05 { lvl(mix(a, b2, level), level) } else { [0.0; 3] }
                }
                // 8 blocks, one per stretch of the spectrum, lit as hard as it plays, with dark gaps.
                "EQ bars" => {
                    let k = ((x * 8.0) as usize).min(7);
                    let gap = n >= 16 && i + 1 < n && (((i + 1) as f32 / (n - 1) as f32 * 8.0) as usize).min(7) != k;
                    if gap { [0.0; 3] } else { lvl(pal_at(&c.pal, k as f32 / 7.0), bars.as_ref().unwrap()[k]) }
                }
                // Thrown up by the latest kick, falls back under gravity.
                "Bouncing ball" => {
                    let h = rings.iter().map(|&(age, s, _)| s.max(0.5) * 4.0 * (age / 0.9) * (1.0 - age / 0.9)).next_back().unwrap_or(0.0).max(0.0);
                    let d = (i as f32 - h * (n - 1) as f32).abs();
                    screen(Some(lvl(a, (1.0 - d / 1.5).max(0.0))), Some(lvl(b2, (1.0 - d / 4.0).max(0.0) * 0.3))).unwrap()
                }
                // A spark wherever in the spectrum something hits: bass at the start, treble at the end.
                "Sparks" => {
                    let v = lead.as_ref().unwrap();
                    let g = ((x * GROUPS as f32) as usize).min(GROUPS - 1);
                    let hit = if cx.an.is_none() { if rnd(i as f64, (p * 6.0).floor()) > 0.9 { 1.0 } else { 0.0 } } else { v.hit[g] };
                    if rnd(i as f64, (t * 20.0).floor()) < hit as f64 * 0.7 { mix(a, b2, x) } else { [0.0; 3] }
                }
                "Vocal colour" => lvl(pal_at(&c.pal, lead.as_ref().unwrap().at * 0.7 + x * 0.3), 0.08 + 0.92 * level),
                "Drop countdown" => {
                    let (fill, fl) = drop.unwrap();
                    let bar = if fill > 0.0 && edge(x) <= fill { lvl(mix(a, b2, fill), 0.3 + 0.7 * fill) } else { [0.0; 3] };
                    mix(bar, [255.0; 3], fl)
                }
                "Bass wobble" => lvl(mix(a, b2, x), level * (0.5 + 0.5 * (std::f64::consts::TAU * (x as f64 * 3.0 - p * 2.0)).sin() as f32)),
                // Both ends snap in towards the middle on each snare.
                "Clap" => if level > 0.05 && (n == 1 || edge(x) <= level) { lvl(mix(a, b2, edge(x)), level) } else { [0.0; 3] },
                "Lava lamp" => {
                    let v = (0..3).map(|k| {
                        let c0 = 0.5 + 0.4 * (p * (0.13 + 0.07 * k as f64) + k as f64 * 2.1).sin();
                        (-((x as f64 - c0) / 0.15).powi(2)).exp()
                    }).sum::<f64>().min(1.0) as f32;
                    lvl(mix(a, b2, v), 0.25 + 0.75 * v)
                }
                "Aurora" => {
                    let x = x as f64;
                    let hue = 0.5 + 0.5 * (x * 5.0 + p * 0.8 + (x * 11.0 - p * 0.5).sin()).sin();
                    let glow = (0.5 + 0.5 * (x * 3.0 - p * 0.3).sin()).powi(2);
                    lvl(mix(a, b2, hue as f32), (0.15 + 0.85 * glow) as f32)
                }
                // Drops at random spots running down the strip, a bright head with a short tail.
                "Matrix rain" => (0..4).fold([0.0; 3], |o, k| {
                    let ph = p * 0.6 + k as f64 * 0.25;
                    let (cyc, f) = (ph.floor(), frac(ph) as f32);
                    let head = rnd(k as f64, cyc) as f32 * 0.5 + f * 0.6;
                    let d = head - x;
                    let v = if d < 0.0 { 0.0 } else { (1.0 - d / 0.2).max(0.0) * (1.0 - f) };
                    screen(Some(o), Some(if d * n as f32 <= 1.0 && d >= 0.0 { lvl(b2, 1.0 - f) } else { lvl(a, v) })).unwrap()
                }),
                // Smooth warm flicker: random levels per group of segments, blended over time.
                "Candle" => {
                    let (g, s) = ((i / 3) as f64, p * 6.0);
                    let k = frac(s) as f32;
                    let f = (rnd(g, s.floor()) as f32 * (1.0 - k) + rnd(g, s.floor() + 1.0) as f32 * k).powf(0.7);
                    lvl(mix(a, b2, f), 0.45 + 0.55 * f)
                }
                // Two colours twisting around each other.
                "DNA" => {
                    let s = (std::f64::consts::TAU * (x as f64 * 2.0 - p * 0.5)).sin() as f32;
                    screen(Some(lvl(a, s.max(0.0).powi(3))), Some(lvl(b2, (-s).max(0.0).powi(3)))).unwrap()
                }
                // Blocks of 3 lit, 3 dark, colours alternating, crawling half a block per beat.
                "Theatre chase" => {
                    let k = (i as i64 + (b * 1.5).floor() as i64).div_euclid(3);
                    if k.rem_euclid(2) == 0 { if k.rem_euclid(4) == 0 { a } else { b2 } } else { [0.0; 3] }
                }
                // Each kick fires a pulse from one end to the other.
                "Kick chase" => {
                    let v = rings.iter().map(|&(age, s, _)| s * (1.0 - age / 0.9) * (-((x - age / 0.9) / 0.05).powi(2)).exp()).fold(0.0, f32::max);
                    lvl(mix(a, b2, x), v.min(1.0))
                }
                "Pitch trail" => trail.iter().enumerate().fold([0.0; 3], |o, (k, &(at, l))| {
                    let v = (1.0 - (i as f32 - at * (n - 1) as f32).abs() / 1.5).max(0.0) * l;
                    screen(Some(o), Some(lvl(mix(a, b2, k as f32 / 10.0), v))).unwrap()
                }),
                // Bass on the first third, mids in the middle, treble on the last.
                "Split bands" => {
                    let z = ((x * 3.0) as usize).min(2);
                    lvl(mix(a, b2, z as f32 / 2.0), cx.band(t, z).powf(1.3))
                }
                // Bass pushes in from the start, treble from the end.
                "Tug of war" => {
                    let (lo, hi) = (cx.band(t, 0), cx.band(t, 2));
                    let l = if n == 1 || x <= lo * 0.6 { lvl(a, lo) } else { [0.0; 3] };
                    let r = if n == 1 || 1.0 - x <= hi * 0.6 { lvl(b2, hi) } else { [0.0; 3] };
                    screen(Some(l), Some(r)).unwrap()
                }
                // Loudness as colour: quiet = first colour, loud = second, loudest = white.
                "Heat" => {
                    let col = if level < 0.5 { mix(a, b2, level * 2.0) } else { mix(b2, [255.0; 3], (level - 0.5) * 1.6) };
                    lvl(col, 0.15 + 0.85 * level)
                }
                "Energy" => lvl(mix(a, b2, x * 0.3 + level * 0.7), 0.05 + 0.95 * level.powf(1.5)),
                // --- patterns ---
                "Plasma" => {
                    let (x, p) = (x as f64, p);
                    let v = 0.5 + 0.25 * ((x * 7.0 + p * 1.3).sin() + (x * 13.0 - p * 2.1 + (p * 0.7).sin() * 2.0).sin());
                    lvl(mix(a, b2, v as f32), 0.35 + 0.65 * v as f32)
                }
                "Meteor shower" => (0..3).fold([0.0; 3], |o, m| {
                    let pos = frac(b * (0.15 + 0.1 * m as f64) + m as f64 * 0.37) as f32 * 1.3; // runs off the end before coming back
                    let d = pos - x;
                    let v = if d < 0.0 { 0.0 } else if d * n as f32 <= 1.0 { 1.0 } else { (1.0 - d / 0.3).max(0.0).powi(2) * 0.6 };
                    screen(Some(o), Some(if d * n as f32 <= 1.0 && d >= 0.0 { lvl(b2, v) } else { lvl(a, v) })).unwrap()
                }),
                // Hits each end on the beat, a flash at the end it hits.
                "Ping-pong" => {
                    let k = b.floor() as i64;
                    let pos = if k.rem_euclid(2) == 0 { fb } else { 1.0 - fb };
                    let ball = (1.0 - (x - pos).abs() * n as f32 / 2.0).max(0.0);
                    let wall = if (k.rem_euclid(2) == 0) == (x < 0.5) { 0.0 } else { (-fb * 8.0).exp() * (1.0 - edge(x) * 4.0).max(0.0) };
                    let col = if k.rem_euclid(2) == 0 { a } else { b2 };
                    screen(Some(lvl(col, ball)), Some(lvl(if k.rem_euclid(2) == 0 { b2 } else { a }, wall))).unwrap()
                }
                // A new colour sweeps over the old one every bar.
                "Colour wipe" => {
                    let (k, f) = ((b / 4.0).floor() as i64, frac(b / 4.0) as f32);
                    let col = |k: i64| match c.pal.len() {
                        0 => if k.rem_euclid(2) == 0 { a } else { b2 },
                        l => rgb(c.pal[k.rem_euclid(l as i64) as usize]),
                    };
                    let f = (f * 2.0).min(1.0); // sweep over half a bar, then hold
                    if x <= f { col(k) } else { col(k - 1) }
                }
                "Twinkle" => {
                    let ph = p * (0.3 + 0.4 * rnd(i as f64, 9.0)) + rnd(i as f64, 4.0);
                    let s = (std::f64::consts::PI * frac(ph)).sin().powi(6) as f32 * if rnd(i as f64, ph.floor()) > 0.45 { 1.0 } else { 0.0 };
                    lvl(mix(a, b2, rnd(i as f64, 5.0) as f32), 0.03 + 0.97 * s)
                }
                // Red half, blue half: three quick flashes each, taking turns every beat.
                "Police" => {
                    let side = (b.floor() as i64).rem_euclid(2) == 0;
                    let on = (x < 0.5) == side && fb < 0.75 && frac(fb as f64 * 4.0) < 0.55;
                    if n == 1 { if fb < 0.75 && frac(fb as f64 * 4.0) < 0.55 { if side { a } else { b2 } } else { [0.0; 3] } } else if on { if side { a } else { b2 } } else { [0.0; 3] }
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
        let mut cols = clip_colors(c, n, t, cx);
        // Transition in from the clip that ends where this one starts: crossfade or wipe.
        if c.trans > 0 && t < c.start + c.tlen {
            if let Some(p) = tr.clips.iter().find(|p| !std::ptr::eq(*p, c) && (p.start + p.len - c.start).abs() < 0.05) {
                let k = ((t - c.start) / c.tlen.max(0.01)).clamp(0.0, 1.0) as f32;
                let prev = clip_colors(&Clip { len: p.len + c.tlen, fout: 0.0, ..p.clone() }, n, t, cx);
                cols = cols.iter().zip(prev).enumerate().map(|(j, (a, b))| {
                    if c.trans == 2 { if (j as f32 + 0.5) / n as f32 <= k { *a } else { b } } else { mix(b, *a, k) }
                }).collect();
            }
        }
        for (o, col) in out.iter_mut().zip(cols) {
            *o = screen(*o, Some(col));
        }
    }
    out
}

/// One light's colours: each track lands at its start segment. Where tracks overlap, the higher
/// track (earlier in the list) wins; its dark parts let the tracks below show through.
/// `win[k]` = (skip, take): only segments skip..skip+take of track k land on this light (a light's
/// slice of a track chained across several lights). Missing = the whole track.
pub fn light_colors(tracks: &[Track], win: &[(usize, usize)], t: f64, cx: &Ctx) -> Vec<Option<Rgb>> {
    let wk = |k: usize| win.get(k).copied().unwrap_or((0, usize::MAX));
    let w = tracks.iter().enumerate().map(|(k, tr)| tr.from + tr.segs.saturating_sub(wk(k).0).min(wk(k).1)).max().unwrap_or(0).clamp(1, 84);
    let mut out = vec![None; w];
    for (k, tr) in tracks.iter().enumerate().rev() {
        let (skip, take) = wk(k);
        for (i, c) in track_colors(tr, t, cx).into_iter().skip(skip).take(take).enumerate() {
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
        Clip { fx: fx.into(), start: 0.0, len: 4.0, col, col2: [124, 108, 255], bri: 100.0, speed: 2.0, rev: true, fin: 1.0, fout: 1.0, pal: vec![], ..Default::default() }
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
        let out = light_colors(&[t(0, &red), t(1, &blue)], &[], 1.0, &cx);
        let px: Vec<[u8; 3]> = out.iter().map(|c| c.unwrap().map(|v| v.round() as u8)).collect();
        assert_eq!(px, vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]], "higher track wins where they overlap");
        let black = Clip { col: [0, 0, 0], ..red.clone() };
        let out = light_colors(&[t(0, &black), t(0, &blue)], &[], 1.0, &cx);
        assert_eq!(out[0].unwrap().map(|v| v.round() as u8), [0, 0, 255], "dark parts of the top track show the one below");
        let long = Track { segs: 4, clips: vec![Clip { fx: "Gradient".into(), col: [255, 0, 0], col2: [0, 0, 255], ..Default::default() }], ..t(0, &red) };
        let second = light_colors(&[long.clone()], &[(2, 2)], 1.0, &cx);
        assert_eq!((second.len(), second[1]), (2, track_colors(&long, 1.0, &cx)[3]), "a light's slice of a chained track");
        assert_eq!(out[0].unwrap().map(|v| v.round() as u8), [0, 0, 255], "dark parts of the top track show the one below");
    }

    #[test]
    fn mirror_repeat_and_transitions() {
        let cx = Ctx { an: None, bpm: 120.0 };
        let grad = Clip { fx: "Gradient".into(), col: [255, 0, 0], col2: [0, 0, 255], ..Default::default() };
        let m = clip_colors(&Clip { mirror: true, ..grad.clone() }, 8, 1.0, &cx);
        assert_eq!(m[0], m[7], "mirrored ends match");
        assert_eq!(m[3], m[4], "mirrored middle matches");
        let r = clip_colors(&Clip { repeat: 2, ..grad.clone() }, 8, 1.0, &cx);
        assert_eq!(r[0], r[4], "repeated twice");
        let red = Clip { fx: "Solid".into(), col: [255, 0, 0], start: 0.0, len: 2.0, ..Default::default() };
        let blue = Clip { fx: "Solid".into(), col: [0, 0, 255], start: 2.0, len: 2.0, trans: 1, tlen: 1.0, ..Default::default() };
        let tr = Track { name: String::new(), segs: 4, from: 0, dev: None, clips: vec![red, blue] };
        let mid = track_colors(&tr, 2.5, &cx)[0].unwrap();
        assert!(mid[0] > 100.0 && mid[2] > 100.0, "halfway through a crossfade: both {mid:?}");
        let mut wipe = tr.clone();
        wipe.clips[1].trans = 2;
        let w = track_colors(&wipe, 2.5, &cx);
        assert_eq!((w[0].unwrap()[2], w[3].unwrap()[0]), (255.0, 255.0), "wipe: new clip on one side, old on the other");
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
    let clip = Clip { fx: "Visualizer".into(), start: 0.0, len: 999.0, col: [0; 3], col2: [0; 3], bri: 100.0, speed: 1.0, rev: false, fin: 0.0, fout: 0.0, pal: vec![], ..Default::default() };
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
        let row: String = light_colors(&tracks, &[], t, &cx).iter().map(|c| {
            let [r, g, b] = c.unwrap_or([0.0; 3]);
            let m = r.max(g).max(b);
            if m < 20.0 { ' ' } else if m < 60.0 { '.' } else if r.min(g).min(b) > 170.0 { 'W' } else if r > 110.0 && g > 140.0 && b < 90.0 { 'L' } else if g > r && g >= b { if m > 140.0 { 'G' } else { 'g' } } else { 'o' }
        }).collect();
        println!("{t:6.2} |{row}|");
        t += 0.1;
    }
}
