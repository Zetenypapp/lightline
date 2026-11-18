//! "Build show": a layered light show generated from the song's parts. Per light three tracks:
//! accents (flashes, strobes, the lead dot), a reactive layer (kick bursts or the vocal bar), and a
//! moving base that changes every few bars. Higher tracks win, so dark parts of the top layers show the base.

use crate::analysis::{Analysis, Part};
use crate::scene::{Clip, Track};

/// Name, then deep (dark base), main, alt (second colour), hot (kick / accent).
pub const PALETTES: [(&str, [[u8; 3]; 4]); 6] = [
    ("Aston Martin", [[0x00, 0x3a, 0x30], [0x00, 0x66, 0x5e], [0xce, 0xdc, 0x00], [0xce, 0xdc, 0x00]]),
    ("Linkin Park red", [[0x2a, 0x00, 0x00], [0xc8, 0x10, 0x2e], [0xff, 0x4a, 0x00], [0xff, 0x4a, 0x00]]),
    ("Electric", [[0x00, 0x0a, 0x3a], [0x00, 0x50, 0xff], [0x00, 0xe5, 0xff], [0xff, 0x00, 0x66]]),
    ("Sunset", [[0x2a, 0x0a, 0x3a], [0xff, 0x2a, 0x6d], [0xff, 0xb0, 0x00], [0x7c, 0x6c, 0xff]]),
    ("Ice", [[0x00, 0x14, 0x2a], [0x2f, 0xd8, 0xb4], [0xff, 0xff, 0xff], [0x7c, 0x6c, 0xff]]),
    ("Toxic", [[0x0a, 0x1a, 0x00], [0x39, 0xff, 0x14], [0xb0, 0x00, 0xff], [0xff, 0xff, 0x00]]),
];
pub const SPEEDS: [&str; 3] = ["Chill", "Normal", "Wild"];
pub const FOCUS: [&str; 4] = ["Balanced", "The beat", "The vocals", "The melody"];

/// What the person picked in the Build show window.
#[derive(Clone, Copy, PartialEq)]
pub struct Opts {
    pub pal: usize,
    pub speed: usize, // index into SPEEDS
    pub focus: usize, // index into FOCUS
    pub strobes: bool,
    pub same: bool, // every light does the same thing
    pub seed: u64,  // a new one per build, so two builds differ
}

impl Default for Opts {
    fn default() -> Self {
        Opts { pal: 0, speed: 1, focus: 0, strobes: true, same: false, seed: 0 }
    }
}

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

fn clip(fx: &str, a: f64, b: f64, col: [u8; 3], col2: [u8; 3]) -> Clip {
    Clip { fx: fx.into(), start: a, len: (b - a).max(0.05), col, col2, bri: 100.0, speed: 1.0, rev: false, fin: 0.0, fout: 0.0, pal: vec![] }
}
fn flash(at: f64, l: f64, col: [u8; 3]) -> Clip {
    Clip { fout: l, ..clip("Solid", at, at + l, col, BLACK) }
}

/// Tracks for each light `(name, light id, segments)`, for a song `len` seconds long.
pub fn build(an: &Analysis, len: f64, lights: &[(String, String, usize)], o: Opts) -> Vec<Track> {
    let [deep, main, alt, hot] = PALETTES[o.pal.min(PALETTES.len() - 1)].1;
    let (chill, wild) = (o.speed == 0, o.speed == 2);
    let (beat, vocals, melody) = (o.focus == 1, o.focus == 2, o.focus == 3);
    // Bars per pattern change: verses / choruses.
    let (verse_every, chorus_every) = if chill { (8, 4) } else if wild { (2, 1) } else { (4, 2) };
    let base_speed = if chill { 0.5 } else if wild { 2.0 } else { 1.0 };
    let verse_pool: [&'static str; 4] = if beat { ["Beat flash", "Alternate", "Pulse", "Chase"] } else if vocals { ["Wave", "Comet", "Breathe", "Gradient"] } else if melody { ["Comet", "Wave", "Chase", "Sparkle"] } else { ["Beat flash", "Comet", "Alternate", "Chase"] };
    let chorus_pool: [&'static str; 4] = if beat { ["Beat flash", "Alternate", "Pulse", "Chase"] } else if vocals { ["Wave", "Alternate", "Sparkle", "Comet"] } else if melody { ["Comet", "Chase", "Sparkle", "Wave"] } else { ["Alternate", "Chase", "Beat flash", "Sparkle"] };
    let rnd = |k: u64| (o.seed ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 33;
    let strobe = |a: f64, b: f64, col: [u8; 3]| if o.strobes { Clip { speed: 0.5, ..clip("Strobe", a, b, col, BLACK) } } else { flash(a, (b - a).min(0.8), col) };

    let mut parts: Vec<(f64, f64, Part)> = an.sections.windows(2).map(|w| (w[0].0, w[1].0, w[0].1)).collect();
    if let Some(&(t, p)) = an.sections.last() {
        parts.push((t, len, p));
    }
    let bars_in = |a: f64, b: f64, every: usize| -> Vec<(f64, f64)> {
        let mut cuts: Vec<f64> = an.downbeats.iter().copied().filter(|&d| d > a + 0.05 && d < b - 0.05).collect();
        cuts.insert(0, a);
        let mut cuts: Vec<f64> = cuts.into_iter().step_by(every.max(1)).collect();
        cuts.push(b);
        cuts.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let mut out = vec![];
    for (li, (name, id, segs)) in lights.iter().enumerate() {
        let (one, dot) = (*segs == 1, *segs >= 20);
        let lk = if o.same { 0 } else { li as u64 + 1 };
        let (mut acc, mut react, mut base) = (vec![], vec![], vec![]);
        for (pi, &(a, b, part)) in parts.iter().enumerate() {
            let after_quiet = pi > 0 && matches!(parts[pi - 1].2, Part::Intro | Part::Gap | Part::Break);
            if parts.get(pi + 1).map(|p| p.2) == Some(Part::Chorus) && !matches!(part, Part::Gap | Part::Break) && !chill {
                if let Some(&(x, _)) = bars_in(a, b, 1).last().filter(|bar| bar.0 > a) {
                    acc.push(strobe(x, b, hot)); // into the chorus
                }
            }
            let pick = |pool: &[&'static str; 4], i: usize| pool[(rnd(pi as u64 * 64 + i as u64) + lk) as usize % pool.len()];
            let swap = |i: usize| rnd(pi as u64 * 64 + i as u64 + 7 + lk) % 2 == 1;
            match part {
                Part::Intro | Part::Break => {
                    base.push(Clip { bri: 55.0, speed: 0.5 * base_speed, fin: if part == Part::Intro { (b - a).min(3.0) } else { 0.0 }, ..clip("Wave", a, b, main, deep) });
                    react.push(clip("Voice", a, b, alt, main));
                    if dot && !beat {
                        acc.push(clip("Lead", a, b, WHITE, alt));
                    }
                }
                Part::Verse => {
                    acc.push(flash(a, if after_quiet { 0.8 } else { 0.4 }, if after_quiet { WHITE } else { alt }));
                    if dot && melody {
                        acc.push(clip("Lead", a + 0.4, b, WHITE, alt));
                    }
                    react.push(if vocals { clip("Voice", a, b, alt, main) } else { Clip { bri: if chill { 70.0 } else { 100.0 }, ..clip("Kick", a, b, hot, main) } });
                    for (i, (x, y)) in bars_in(a, b, verse_every).into_iter().enumerate() {
                        let (c1, c2) = if swap(i) { (alt, deep) } else { (main, deep) };
                        base.push(if one { clip("Solid", x, y, deep, BLACK) } else { Clip { bri: 75.0, speed: base_speed, rev: swap(i + 100), ..clip(pick(&verse_pool, i), x, y, c1, c2) } });
                    }
                }
                Part::Chorus => {
                    acc.push(flash(a, if after_quiet { 0.8 } else { 0.5 }, WHITE));
                    if !chill {
                        for (x, _) in bars_in(a, b, if wild { 2 } else { 4 }).into_iter().skip(1) {
                            acc.push(flash(x, 0.35, WHITE));
                        }
                    }
                    if dot && !beat {
                        acc.push(clip("Lead", a + 0.5, b, WHITE, hot));
                    }
                    react.push(if vocals { clip("Voice", a, b, WHITE, alt) } else { clip("Kick", a, b, WHITE, alt) });
                    for (i, (x, y)) in bars_in(a, b, chorus_every).into_iter().enumerate() {
                        let (c1, c2) = if swap(i) { (hot, main) } else { (alt, hot) };
                        base.push(if one { clip("Alternate", x, y, hot, alt) } else { Clip { speed: base_speed, rev: swap(i + 100), ..clip(pick(&chorus_pool, i), x, y, c1, c2) } });
                    }
                }
                Part::Gap => acc.push(strobe(a, b, WHITE)),
                Part::Outro => {
                    acc.push(flash(a, 0.6, WHITE));
                    base.push(Clip { fout: (b - a).min(4.0), ..clip("Solid", a, b, main, BLACK) });
                }
            }
        }
        let react_name = if vocals { "vocals" } else { "kick" };
        for (layer, clips) in [("accents", acc), (react_name, react), ("base", base)] {
            out.push(Track { name: format!("Show · {name} {layer}"), segs: *segs, from: 0, dev: Some(id.clone()), clips });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Analysis {
        let beats: Vec<f64> = (0..240).map(|b| b as f64 * 0.5).collect();
        let downbeats: Vec<f64> = beats.iter().step_by(4).copied().collect();
        let sections = vec![(0.0, Part::Intro), (16.0, Part::Verse), (48.0, Part::Gap), (50.0, Part::Chorus), (90.0, Part::Outro)];
        Analysis { bpm: 120.0, beats, downbeats, fps: 50.0, lag: 0.0, bands: vec![], spec: vec![], lead: vec![], sections }
    }

    #[test]
    fn builds_layers_inside_the_song() {
        let lights = vec![("Strip".to_string(), "A".to_string(), 84), ("Neon".to_string(), "B".to_string(), 1)];
        let t = build(&song(), 120.0, &lights, Opts::default());
        assert_eq!(t.len(), 6);
        for tr in &t {
            assert!(tr.clips.iter().all(|c| c.start >= 0.0 && c.len > 0.0 && c.start + c.len <= 121.0), "{}", tr.name);
        }
        assert!(t[0].clips.iter().any(|c| c.fx == "Strobe" && c.start == 48.0), "stop gets a strobe");
        assert!(t[0].clips.iter().any(|c| c.fx == "Lead"), "strip gets the lead dot");
        assert!(!t[3].clips.iter().any(|c| c.fx == "Lead"), "1-segment light doesn't");
        assert!(t[2].clips.windows(2).all(|w| w[0].start + w[0].len <= w[1].start + 1e-9), "base clips don't overlap");
    }

    #[test]
    fn options_change_the_show() {
        let lights = vec![("Strip".to_string(), "A".to_string(), 84)];
        let fxs = |o: Opts| build(&song(), 120.0, &lights, o).iter().flat_map(|t| t.clips.iter().map(|c| (c.fx.clone(), (c.start * 10.0) as i64))).collect::<Vec<_>>();
        let d = Opts::default();
        assert_ne!(fxs(d), fxs(Opts { seed: 99, ..d }), "a new seed gives a new variation");
        assert_ne!(fxs(d), fxs(Opts { speed: 2, ..d }), "wild changes more often");
        assert!(!fxs(Opts { strobes: false, ..d }).iter().any(|(f, _)| f == "Strobe"), "strobes off");
        assert!(fxs(Opts { focus: 2, ..d }).iter().any(|(f, _)| f == "Voice"), "vocal focus uses the vocal bar");
    }
}
