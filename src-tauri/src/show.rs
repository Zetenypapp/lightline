//! "Build show": a layered light show generated from the song's parts. Per light three tracks:
//! accents (flashes, strobes, the lead dot), a reactive layer (kick bursts or the vocal bar), and a
//! moving base that changes every few bars. Higher tracks win, so dark parts of the top layers show the base.

use crate::analysis::{Analysis, Part};
use crate::scene::{Clip, Track};

/// Name, then deep (dark base), main, alt (second colour), hot (kick / accent).
pub const PALETTES: [(&str, [[u8; 3]; 4]); 20] = [
    ("Racing green", [[0x00, 0x3a, 0x30], [0x00, 0x66, 0x5e], [0xce, 0xdc, 0x00], [0xce, 0xdc, 0x00]]),
    ("Blood red", [[0x2a, 0x00, 0x00], [0xc8, 0x10, 0x2e], [0xff, 0x4a, 0x00], [0xff, 0x4a, 0x00]]),
    ("Electric", [[0x00, 0x0a, 0x3a], [0x00, 0x50, 0xff], [0x00, 0xe5, 0xff], [0xff, 0x00, 0x66]]),
    ("Sunset", [[0x2a, 0x0a, 0x3a], [0xff, 0x2a, 0x6d], [0xff, 0xb0, 0x00], [0x7c, 0x6c, 0xff]]),
    ("Ice", [[0x00, 0x14, 0x2a], [0x2f, 0xd8, 0xb4], [0xff, 0xff, 0xff], [0x7c, 0x6c, 0xff]]),
    ("Toxic", [[0x0a, 0x1a, 0x00], [0x39, 0xff, 0x14], [0xb0, 0x00, 0xff], [0xff, 0xff, 0x00]]),
    ("Fire", [[0x2a, 0x05, 0x00], [0xff, 0x30, 0x00], [0xff, 0x9a, 0x00], [0xff, 0xe0, 0x40]]),
    ("Ocean", [[0x00, 0x10, 0x2a], [0x00, 0x6e, 0xc8], [0x00, 0xd0, 0xc8], [0xa0, 0xf0, 0xff]]),
    ("Synthwave", [[0x1a, 0x00, 0x2e], [0xff, 0x00, 0xc8], [0x00, 0xe0, 0xff], [0xff, 0x6a, 0x00]]),
    ("Aurora", [[0x00, 0x1a, 0x1a], [0x00, 0xff, 0x9c], [0x8a, 0x2b, 0xff], [0x00, 0xc8, 0xff]]),
    ("Forest", [[0x05, 0x1a, 0x05], [0x1e, 0x8c, 0x2e], [0xa8, 0xd8, 0x00], [0xff, 0xc8, 0x40]]),
    ("Gold", [[0x1a, 0x10, 0x00], [0xd4, 0x9a, 0x00], [0xff, 0xd7, 0x00], [0xff, 0xff, 0xff]]),
    ("Candy", [[0x2a, 0x00, 0x1a], [0xff, 0x5c, 0xc8], [0x5c, 0xd6, 0xff], [0xff, 0xf0, 0x5c]]),
    ("Police", [[0x00, 0x00, 0x1a], [0x00, 0x30, 0xff], [0xff, 0x00, 0x10], [0xff, 0xff, 0xff]]),
    ("Lava", [[0x1a, 0x00, 0x00], [0x8a, 0x00, 0x00], [0xff, 0x40, 0x00], [0xff, 0xb0, 0x00]]),
    ("Pure white", [[0x10, 0x10, 0x10], [0x80, 0x80, 0x80], [0xff, 0xff, 0xff], [0xff, 0xff, 0xff]]),
    ("Papaya", [[0x1a, 0x0a, 0x00], [0xff, 0x80, 0x00], [0x47, 0xc7, 0xfc], [0xff, 0xff, 0xff]]),
    ("Silver arrow", [[0x00, 0x14, 0x14], [0x00, 0xa1, 0x9b], [0xc8, 0xcc, 0xd0], [0xff, 0xff, 0xff]]),
    ("Night racer", [[0x00, 0x05, 0x1a], [0x1e, 0x2a, 0x8c], [0xff, 0x10, 0x30], [0xff, 0xcc, 0x00]]),
    ("Deep purple", [[0x10, 0x00, 0x1a], [0x6a, 0x00, 0xff], [0xc8, 0x6a, 0xff], [0xff, 0xff, 0xff]]),
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
    pub custom: Option<[[u8; 3]; 4]>, // the user's own colour set, instead of PALETTES[pal]
}

impl Default for Opts {
    fn default() -> Self {
        Opts { pal: 0, speed: 1, focus: 0, strobes: true, same: false, seed: 0, custom: None }
    }
}

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

fn clip(fx: &str, a: f64, b: f64, col: [u8; 3], col2: [u8; 3]) -> Clip {
    Clip { fx: fx.into(), start: a, len: (b - a).max(0.05), col, col2, bri: 100.0, speed: 1.0, rev: false, fin: 0.0, fout: 0.0, pal: vec![], ..Default::default() }
}
fn flash(at: f64, l: f64, col: [u8; 3]) -> Clip {
    Clip { fout: l, ..clip("Solid", at, at + l, col, BLACK) }
}

/// Tracks for each light `(name, light id, segments)`, for a song `len` seconds long.
pub fn build(an: &Analysis, len: f64, lights: &[(String, String, usize)], o: Opts) -> Vec<Track> {
    let [deep, main, alt, hot] = o.custom.unwrap_or(PALETTES[o.pal.min(PALETTES.len() - 1)].1);
    let (chill, wild) = (o.speed == 0, o.speed == 2);
    let (beat, vocals, melody) = (o.focus == 1, o.focus == 2, o.focus == 3);
    // Bars per pattern change: verses / choruses.
    let (verse_every, chorus_every) = if chill { (8, 4) } else if wild { (2, 1) } else { (4, 2) };
    let base_speed = if chill { 0.5 } else if wild { 2.0 } else { 1.0 };
    let verse_pool: [&'static str; 4] = if beat { ["Beat flash", "Heartbeat", "Stack", "Alternate"] } else if vocals { ["Wave", "Comet", "Aurora", "Plasma"] } else if melody { ["Comet", "Wave", "Scanner", "Meteor shower"] } else { ["Beat flash", "Comet", "Ping-pong", "Theatre chase"] };
    let chorus_pool: [&'static str; 4] = if beat { ["Stack", "Alternate", "Colour wipe", "Converge"] } else if vocals { ["Wave", "Alternate", "Twinkle", "Comet"] } else if melody { ["Comet", "DNA", "Meteor shower", "Converge"] } else { ["Alternate", "Converge", "Colour wipe", "Meteor shower"] };
    // The music-reactive layer: one of these per part.
    let verse_react: [&'static str; 4] = if beat { ["Kick", "Kick chase", "Clap", "Bouncing ball"] } else if vocals { ["Voice", "Vocal colour", "Voice", "Heat"] } else if melody { ["Pitch trail", "Kick", "Sparks", "Bass wobble"] } else { ["Kick", "Bass wobble", "Split bands", "Kick chase"] };
    let chorus_react: [&'static str; 4] = if beat { ["Ripple", "Kick chase", "Bouncing ball", "Clap"] } else if vocals { ["Voice", "Vocal colour", "Heat", "Voice"] } else if wild { ["Fireworks", "Lightning", "Tug of war", "Fireworks"] } else { ["Kick", "Energy", "Tug of war", "Sparks"] };
    let quiet_react: [&'static str; 4] = ["Voice", "Sparks", "Energy", "Voice"];
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
                let bars = bars_in(a, b, 1);
                if let Some(&(x, _)) = bars.iter().rev().nth(1).filter(|bar| bar.0 > a) {
                    acc.push(clip(if pi % 2 == 0 { "Riser" } else { "Drop countdown" }, x, b, alt, hot)); // fill up over the last 2 bars into the chorus
                }
                if let Some(&(x, _)) = bars.last().filter(|bar| bar.0 > a && (wild || !o.strobes)) {
                    acc.push(strobe(x, b, hot));
                }
            }
            let pick = |pool: &[&'static str; 4], i: usize| pool[(rnd(pi as u64 * 64 + i as u64) + lk) as usize % pool.len()];
            let swap = |i: usize| rnd(pi as u64 * 64 + i as u64 + 7 + lk) % 2 == 1;
            match part {
                Part::Intro | Part::Break => {
                    base.push(Clip { bri: 55.0, speed: 0.5 * base_speed, fin: if part == Part::Intro { (b - a).min(3.0) } else { 0.0 }, ..clip(if pi % 2 == 0 { "Wave" } else { "Lava lamp" }, a, b, main, deep) });
                    react.push(clip(pick(&quiet_react, 999), a, b, alt, main));
                    if dot && !beat {
                        acc.push(clip("Lead", a, b, WHITE, alt));
                    }
                }
                Part::Verse => {
                    acc.push(flash(a, if after_quiet { 0.8 } else { 0.4 }, if after_quiet { WHITE } else { alt }));
                    if dot && melody {
                        acc.push(clip("Lead", a + 0.4, b, WHITE, alt));
                    }
                    react.push(Clip { bri: if chill { 70.0 } else { 100.0 }, ..clip(pick(&verse_react, 999), a, b, if vocals { alt } else { hot }, main) });
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
                    react.push(clip(pick(&chorus_react, 999), a, b, if wild && !beat && !vocals { hot } else { WHITE }, if wild && !beat && !vocals { WHITE } else { alt }));
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
        // One compound clip per song part and layer ("Chorus 2"), not hundreds of loose clips.
        let wrap = |clips: Vec<Clip>| -> Vec<Clip> {
            let mut seen: Vec<Part> = vec![];
            parts.iter().filter_map(|&(a, b, part)| {
                seen.push(part);
                let k = seen.iter().filter(|p| **p == part).count();
                let inner: Vec<Clip> = clips.iter().filter(|c| c.start >= a - 1e-9 && c.start < b - 1e-9).cloned().collect();
                (!inner.is_empty()).then(|| crate::scene::group(inner, format!("{} {k}", part.name())))
            }).collect()
        };
        for (layer, clips) in [("accents", wrap(acc)), (react_name, wrap(react)), ("base", wrap(base))] {
            out.push(Track { name: format!("Show · {name} {layer}"), segs: *segs, from: 0, dev: Some(id.clone()), muted: false, locked: false, clips });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(t: &Track) -> Vec<Clip> {
        t.clips.iter().flat_map(|c| c.inner.clone()).collect()
    }

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
            assert!(tr.clips.iter().all(|c| c.fx == "Compound"), "wrapped per part");
            assert!(flat(tr).iter().all(|c| c.start >= 0.0 && c.len > 0.0 && c.start + c.len <= 121.0), "{}", tr.name);
        }
        assert!(flat(&t[0]).iter().any(|c| c.fx == "Strobe" && c.start == 48.0), "stop gets a strobe");
        assert!(flat(&t[0]).iter().any(|c| c.fx == "Lead"), "strip gets the lead dot");
        assert!(!flat(&t[3]).iter().any(|c| c.fx == "Lead"), "1-segment light doesn't");
        assert!(flat(&t[2]).windows(2).all(|w| w[0].start + w[0].len <= w[1].start + 1e-9), "base clips don't overlap");
    }

    #[test]
    fn options_change_the_show() {
        let lights = vec![("Strip".to_string(), "A".to_string(), 84)];
        let fxs = |o: Opts| build(&song(), 120.0, &lights, o).iter().flat_map(|t| flat(t).into_iter().map(|c| (c.fx.clone(), (c.start * 10.0) as i64))).collect::<Vec<_>>();
        let d = Opts::default();
        assert_ne!(fxs(d), fxs(Opts { seed: 99, ..d }), "a new seed gives a new variation");
        assert_ne!(fxs(d), fxs(Opts { speed: 2, ..d }), "wild changes more often");
        assert!(!fxs(Opts { strobes: false, ..d }).iter().any(|(f, _)| f == "Strobe"), "strobes off");
        assert!(fxs(Opts { focus: 2, ..d }).iter().any(|(f, _)| f == "Voice"), "vocal focus uses the vocal bar");
    }
}
