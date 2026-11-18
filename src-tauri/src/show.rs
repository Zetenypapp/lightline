//! "Build show": a layered light show generated from the song's parts. Per light three tracks:
//! accents (flashes, strobes, the lead dot), kick bursts / vocal bar, and a moving base that changes
//! every few bars. Higher tracks win, so dark parts of the top layers show the base.

use crate::analysis::{Analysis, Part};
use crate::scene::{Clip, Track};

/// Name, then deep (dark base), main, alt (second colour), hot (kick / accent).
pub const PALETTES: [(&str, [[u8; 3]; 4]); 5] = [
    ("Aston Martin", [[0x00, 0x3a, 0x30], [0x00, 0x66, 0x5e], [0xce, 0xdc, 0x00], [0xce, 0xdc, 0x00]]),
    ("Linkin Park red", [[0x2a, 0x00, 0x00], [0xc8, 0x10, 0x2e], [0xff, 0x4a, 0x00], [0xff, 0x4a, 0x00]]),
    ("Electric", [[0x00, 0x0a, 0x3a], [0x00, 0x50, 0xff], [0x00, 0xe5, 0xff], [0xff, 0x00, 0x66]]),
    ("Sunset", [[0x2a, 0x0a, 0x3a], [0xff, 0x2a, 0x6d], [0xff, 0xb0, 0x00], [0x7c, 0x6c, 0xff]]),
    ("Ice", [[0x00, 0x14, 0x2a], [0x2f, 0xd8, 0xb4], [0xff, 0xff, 0xff], [0x7c, 0x6c, 0xff]]),
];
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

fn clip(fx: &str, a: f64, b: f64, col: [u8; 3], col2: [u8; 3]) -> Clip {
    Clip { fx: fx.into(), start: a, len: (b - a).max(0.05), col, col2, bri: 100.0, speed: 1.0, rev: false, fin: 0.0, fout: 0.0, pal: vec![] }
}
fn flash(at: f64, l: f64, col: [u8; 3]) -> Clip {
    Clip { fout: l, ..clip("Solid", at, at + l, col, BLACK) }
}
fn strobe(a: f64, b: f64, col: [u8; 3]) -> Clip {
    Clip { speed: 0.5, ..clip("Strobe", a, b, col, BLACK) } // 2 flashes a beat: clean on slow lights too
}

/// Tracks for each light `(name, light id, segments)`, for a song `len` seconds long.
pub fn build(an: &Analysis, len: f64, lights: &[(String, String, usize)], pal: [[u8; 3]; 4]) -> Vec<Track> {
    let [deep, main, alt, hot] = pal;
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
        let (mut acc, mut kick, mut base) = (vec![], vec![], vec![]);
        for (pi, &(a, b, part)) in parts.iter().enumerate() {
            let after_quiet = pi > 0 && matches!(parts[pi - 1].2, Part::Intro | Part::Gap | Part::Break);
            let next = parts.get(pi + 1).map(|p| p.2);
            // The last bar before a chorus that doesn't follow a stop: strobe into it.
            if next == Some(Part::Chorus) && !matches!(part, Part::Gap | Part::Break) {
                if let Some(&(x, _)) = bars_in(a, b, 1).last().filter(|bar| bar.0 > a) {
                    acc.push(strobe(x, b, hot));
                }
            }
            match part {
                Part::Intro | Part::Break => {
                    base.push(Clip { bri: 55.0, speed: 0.5, fin: if part == Part::Intro { (b - a).min(3.0) } else { 0.0 }, ..clip("Wave", a, b, main, deep) });
                    kick.push(clip("Voice", a, b, alt, main));
                    if dot {
                        acc.push(clip("Lead", a, b, WHITE, alt));
                    }
                }
                Part::Verse => {
                    acc.push(flash(a, if after_quiet { 0.8 } else { 0.4 }, if after_quiet { WHITE } else { alt }));
                    kick.push(clip("Kick", a, b, hot, main));
                    let pats = [("Beat flash", main, deep), ("Comet", alt, main), ("Alternate", main, deep), ("Chase", alt, deep)];
                    for (i, (x, y)) in bars_in(a, b, 4).into_iter().enumerate() {
                        let (fx, c1, c2) = pats[(i + li) % pats.len()];
                        base.push(if one { clip("Solid", x, y, deep, BLACK) } else { Clip { bri: 75.0, rev: i % 2 == 1, ..clip(fx, x, y, c1, c2) } });
                    }
                }
                Part::Chorus => {
                    acc.push(flash(a, if after_quiet { 0.8 } else { 0.5 }, WHITE));
                    for (x, _) in bars_in(a, b, 4).into_iter().skip(1) {
                        acc.push(flash(x, 0.35, WHITE));
                    }
                    if dot {
                        acc.push(clip("Lead", a + 0.5, b, WHITE, hot));
                    }
                    kick.push(clip("Kick", a, b, WHITE, alt));
                    let pats = [("Alternate", hot, main), ("Chase", alt, hot), ("Beat flash", hot, alt), ("Sparkle", main, WHITE)];
                    for (i, (x, y)) in bars_in(a, b, 2).into_iter().enumerate() {
                        let (fx, c1, c2) = pats[(i + li) % pats.len()];
                        base.push(if one { clip("Alternate", x, y, hot, alt) } else { Clip { rev: i % 2 == 1, ..clip(fx, x, y, c1, c2) } });
                    }
                }
                Part::Gap => acc.push(strobe(a, b, WHITE)),
                Part::Outro => {
                    acc.push(flash(a, 0.6, WHITE));
                    base.push(Clip { fout: (b - a).min(4.0), ..clip("Solid", a, b, main, BLACK) });
                }
            }
        }
        for (layer, clips) in [("accents", acc), ("kick", kick), ("base", base)] {
            out.push(Track { name: format!("Show · {name} {layer}"), segs: *segs, from: 0, dev: Some(id.clone()), clips });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_layers_inside_the_song() {
        let beats: Vec<f64> = (0..240).map(|b| b as f64 * 0.5).collect();
        let downbeats: Vec<f64> = beats.iter().step_by(4).copied().collect();
        let sections = vec![(0.0, Part::Intro), (16.0, Part::Verse), (48.0, Part::Gap), (50.0, Part::Chorus), (90.0, Part::Outro)];
        let an = Analysis { bpm: 120.0, beats, downbeats, fps: 50.0, lag: 0.0, bands: vec![], spec: vec![], lead: vec![], sections };
        let lights = vec![("Strip".to_string(), "A".to_string(), 84), ("Neon".to_string(), "B".to_string(), 1)];
        let t = build(&an, 120.0, &lights, PALETTES[0].1);
        assert_eq!(t.len(), 6);
        for tr in &t {
            assert!(tr.clips.iter().all(|c| c.start >= 0.0 && c.len > 0.0 && c.start + c.len <= 120.0 + 1.0), "{}", tr.name);
        }
        assert!(t[0].clips.iter().any(|c| c.fx == "Strobe" && c.start == 48.0), "stop gets a strobe");
        assert!(t[0].clips.iter().any(|c| c.fx == "Lead"), "strip gets the lead dot");
        assert!(!t[3].clips.iter().any(|c| c.fx == "Lead"), "1-segment light doesn't");
        let base = &t[2].clips;
        assert!(base.windows(2).all(|w| w[0].start + w[0].len <= w[1].start + 1e-9), "base clips don't overlap");
    }
}
