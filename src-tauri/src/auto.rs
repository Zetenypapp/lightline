//! "Auto show": fills a scene's tracks from the song analysis. Moving effects in quiet parts,
//! busy multi-colour beat effects in loud parts, a one-bar strobe on each drop. Effect and colours
//! change every 4 bars, and every track gets its own.

use crate::analysis::Analysis;
use crate::fx::hsl;
use crate::scene::{Clip, Track};

const CALM: [&str; 4] = ["Wave", "Comet", "Spectrum", "Sparkle"];
const MID: [&str; 5] = ["Chase", "Spectrum", "Comet", "Alternate", "Meter"];
const LOUD: [&str; 5] = ["Rainbow", "Alternate", "Sparkle", "Spectrum", "Chase"];
const PHRASE: usize = 4; // bars

/// Bar start times: the downbeats if found, else every 4th beat.
fn bars(an: &Analysis) -> Vec<f64> {
    if an.downbeats.len() >= 2 { an.downbeats.clone() } else { an.beats.iter().step_by(4).copied().collect() }
}

/// Mean bass+mid+high level 0..1 between times a and b.
fn energy(an: &Analysis, a: f64, b: f64) -> f32 {
    let n = an.bands.len() / 3;
    let f = |t: f64| (((t - an.lag) * an.fps).max(0.0) as usize).min(n);
    let (i, j) = (f(a), f(b).max(f(a) + 1).min(n));
    if j <= i {
        return 0.0;
    }
    an.bands[i * 3..j * 3].iter().map(|&v| v as f32).sum::<f32>() / ((j - i) * 3) as f32 / 255.0
}

/// Replaces every track's clips with a generated show. Each track gets its own effect and colours.
pub fn fill(an: &Analysis, len: f64, tracks: &mut [Track]) {
    let mut cuts: Vec<f64> = bars(an).into_iter().filter(|&t| t > 0.0 && t < len).collect();
    cuts.insert(0, 0.0);
    cuts.push(len);
    let e: Vec<f32> = cuts.windows(2).map(|w| energy(an, w[0], w[1])).collect();
    let mut sorted = e.clone();
    sorted.sort_by(f32::total_cmp);
    let pct = |p: f32| sorted[((sorted.len() - 1) as f32 * p) as usize];
    let (lo, hi) = (pct(0.4), pct(0.75));
    let mut lvl: Vec<usize> = e.iter().map(|&v| if v >= hi { 2 } else if v > lo { 1 } else { 0 }).collect();
    // A lone bar between two equal neighbours follows them, so sections don't flicker.
    for i in 1..lvl.len().saturating_sub(1) {
        if lvl[i - 1] == lvl[i + 1] {
            lvl[i] = lvl[i - 1];
        }
    }
    for tr in tracks.iter_mut() {
        tr.clips.clear();
    }
    let (mut i, mut ph) = (0, 0);
    while i < lvl.len() {
        let mut j = i + 1;
        while j < lvl.len() && lvl[j] == lvl[i] {
            j += 1;
        }
        let drop = lvl[i] == 2 && i > 0 && e[i] - e[i - 1] > 0.12;
        let (pool, bri, speed): (&[&str], f64, f64) = match lvl[i] {
            0 => (&CALM, 60.0, 0.5),
            1 => (&MID, 85.0, 1.0),
            _ => (&LOUD, 100.0, 1.0),
        };
        let mut b = i;
        while b < j {
            let strobe = drop && b == i && j - i > 1;
            let end = if strobe { b + 1 } else { (b + PHRASE).min(j) };
            for (k, tr) in tracks.iter_mut().enumerate() {
                let h = (ph * 53 + k * 137) as f32; // 137°: no two tracks land on the same colour
                let c = |h: f32| hsl(h, 95.0, 55.0).map(|v| v.round() as u8);
                let fx = if strobe { "Strobe" } else { pool[(ph + k) % pool.len()] };
                tr.clips.push(Clip { fx: fx.into(), start: cuts[b], len: cuts[end] - cuts[b], col: c(h), col2: c(h + 150.0), bri, speed, rev: (ph + k) % 2 == 1, fin: 0.0, fout: 0.0 });
            }
            (b, ph) = (end, ph + 1);
        }
        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_then_drop() {
        // 64 s at 120 BPM: quiet for 32 s, then loud.
        let fps = 50.0;
        let bands = (0..64 * 50).flat_map(|f| [if f < 32 * 50 { 30 } else { 230 }; 3]).collect();
        let beats: Vec<f64> = (0..128).map(|b| b as f64 * 0.5).collect();
        let an = Analysis { bpm: 120.0, beats, downbeats: vec![], fps, lag: 0.0, bands, spec: vec![] };
        let tr = |n: &str| Track { name: n.into(), segs: 15, from: 0, dev: None, clips: vec![] };
        let mut t = vec![tr("A"), tr("B"), tr("C"), tr("D")];
        fill(&an, 64.0, &mut t);
        for tr in &t {
            assert!(CALM.contains(&tr.clips[0].fx.as_str()));
            assert!(tr.clips.windows(2).all(|w| (w[0].start + w[0].len - w[1].start).abs() < 1e-9), "gapless");
            let at32 = tr.clips.iter().find(|c| c.start == 32.0).unwrap();
            assert_eq!(at32.fx, "Strobe");
            assert!(LOUD.contains(&tr.clips.last().unwrap().fx.as_str()));
        }
        assert_ne!(t[0].clips.last().unwrap().fx, t[1].clips.last().unwrap().fx, "tracks differ");
        assert_ne!(t[0].clips[0].col, t[3].clips[0].col, "tracks 1 and 4 differ in colour");
        assert!(t[0].clips.iter().all(|c| c.len <= 4.0 * 2.0 + 1e-9), "new pattern every 4 bars");

    }

    /// Prints the show for a real song: cargo test auto_on_song -- --ignored --nocapture (SONG=path)
    #[test]
    #[ignore]
    fn auto_on_song() {
        let path = std::env::var("SONG").unwrap();
        let (track, mono) = crate::audio::decode(std::path::Path::new(&path)).unwrap();
        let an = crate::analysis::analyze(&mono, track.rate);
        let len = mono.len() as f64 / track.rate as f64;
        let tr = |n: &str| Track { name: n.into(), segs: 84, from: 0, dev: None, clips: vec![] };
        let mut t = vec![tr("A"), tr("B"), tr("C"), tr("D")];
        fill(&an, len, &mut t);
        for (a, d) in t[0].clips.iter().zip(&t[3].clips) {
            println!("{:6.1} {:5.1}  {:<10} {:<10} {:?} {:?}", a.start, a.len, a.fx, d.fx, a.col, d.col);
        }
    }
}
