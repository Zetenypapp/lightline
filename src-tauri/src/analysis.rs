//! Music analysis: onset strength (spectral flux), tempo (autocorrelation with a
//! 120 BPM prior), beat tracking (Ellis dynamic programming, the method librosa
//! uses), downbeats (bass onsets per 4/4 phase) and bass/mid/high energy.

use rustfft::{num_complex::Complex, FftPlanner};

const N: usize = 2048;
const HOP: usize = 512;
/// Log-spaced spectrum bands for the Spectrum effect.
pub const SPEC: usize = 24;

/// Spectrum band of FFT bin `k` (bins 2..kmax, log scale so every band gets at least one bin).
fn spec_band(k: usize, kmax: usize) -> Option<usize> {
    (k >= 2 && k < kmax).then(|| (((k as f64 / 2.0).ln() / (kmax as f64 / 2.0).ln()) * SPEC as f64) as usize).map(|b| b.min(SPEC - 1))
}

pub struct Analysis {
    pub bpm: f64,
    pub beats: Vec<f64>,
    pub downbeats: Vec<f64>,
    pub fps: f64,
    /// Seconds from a frame's start to its window centre: band frame = (t - lag) * fps.
    pub lag: f64,
    /// Per frame: bass, mid, high, each 0..255.
    pub bands: Vec<u8>,
    /// Per frame: SPEC log-spaced bands, each 0..255.
    pub spec: Vec<u8>,
}

pub fn analyze(mono: &[f32], sr: u32) -> Analysis {
    let srf = sr as f64;
    let fps = srf / HOP as f64;
    let lag = (N / 2) as f64 / srf;
    let frames = if mono.len() > N { (mono.len() - N) / HOP + 1 } else { 0 };
    // Under 2 s there is too little to find a tempo (and the lag search would underflow).
    if frames < (fps * 2.0) as usize {
        return Analysis { bpm: 120.0, beats: vec![], downbeats: vec![], fps, lag, bands: vec![], spec: vec![] };
    }

    let win: Vec<f32> = (0..N)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos())
        .collect();
    let fft = FftPlanner::new().plan_fft_forward(N);
    let bin_hz = srf / N as f64;
    let lo_end = (150.0 / bin_hz).ceil() as usize;
    let mid_end = (2000.0 / bin_hz).ceil() as usize;

    let mut buf = vec![Complex::new(0.0f32, 0.0); N];
    let mut prev = vec![0f32; N / 2];
    let mut onset = Vec::with_capacity(frames);
    let mut bass_onset = Vec::with_capacity(frames);
    let mut energy = Vec::with_capacity(frames * 3);
    let kmax = (N / 2).min((16000.0 / bin_hz) as usize);
    let band_of: Vec<Option<usize>> = (0..N / 2).map(|k| spec_band(k, kmax)).collect();
    let mut spec = Vec::with_capacity(frames * SPEC);
    for f in 0..frames {
        let s = &mono[f * HOP..f * HOP + N];
        for i in 0..N {
            buf[i] = Complex::new(s[i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let (mut flux, mut bflux, mut e, mut sp) = (0f32, 0f32, [0f32; 3], [0f32; SPEC]);
        for k in 1..N / 2 {
            let m = buf[k].norm();
            let lm = (1.0 + 10.0 * m).ln();
            let d = (lm - prev[k]).max(0.0);
            prev[k] = lm;
            flux += d;
            if k < lo_end {
                bflux += d;
            }
            e[if k < lo_end { 0 } else if k < mid_end { 1 } else { 2 }] += m * m;
            if let Some(b) = band_of[k] {
                sp[b] += m * m;
            }
        }
        onset.push(flux);
        bass_onset.push(bflux);
        energy.extend(e.map(f32::sqrt));
        spec.extend(sp.map(f32::sqrt));
    }
    let onset = normalize(&onset);
    let bass_onset = normalize(&bass_onset);

    let period = tempo(&onset, fps);
    let beat_frames = track(&onset, period);
    // Sub-frame beat time: parabola through the onset peak and its neighbours.
    let to_sec = |f: usize| {
        let (a, b, c) = (onset[f.saturating_sub(1)], onset[f], onset[(f + 1).min(frames - 1)]);
        let den = a - 2.0 * b + c;
        let off = if den < 0.0 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 } as f64;
        (((f as f64 + off) * HOP as f64 + (N / 2) as f64) / srf * 1000.0).round() / 1000.0
    };
    let beats: Vec<f64> = beat_frames.iter().map(|&f| to_sec(f)).collect();

    let phase = (0..4)
        .max_by(|&a, &b| phase_score(&bass_onset, &beat_frames, a).total_cmp(&phase_score(&bass_onset, &beat_frames, b)))
        .unwrap_or(0);
    let downbeats = beats.iter().skip(phase).step_by(4).copied().collect();

    // Tempo = least-squares slope of beat time vs beat index (averages out frame rounding).
    let bpm = if beats.len() >= 4 {
        let n = beats.len() as f64;
        let mx = (n - 1.0) / 2.0;
        let my = beats.iter().sum::<f64>() / n;
        let (mut sxy, mut sxx) = (0.0, 0.0);
        for (i, y) in beats.iter().enumerate() {
            sxy += (i as f64 - mx) * (y - my);
            sxx += (i as f64 - mx).powi(2);
        }
        60.0 / (sxy / sxx)
    } else {
        60.0 * fps / period
    };

    let (bands, spec) = (to_u8(&energy, 3), to_u8(&spec, SPEC));
    Analysis { bpm: (bpm * 10.0).round() / 10.0, beats, downbeats, fps, lag, bands, spec }
}

/// Scales each column of frame-major `rows` (`width` per frame) by its 98th percentile to 0..255.
fn to_u8(rows: &[f32], width: usize) -> Vec<u8> {
    let frames = rows.len() / width;
    let mut out = vec![0u8; rows.len()];
    for b in 0..width {
        let mut v: Vec<f32> = (0..frames).map(|f| rows[f * width + b]).collect();
        v.sort_by(f32::total_cmp);
        let p98 = v[v.len() * 98 / 100].max(1e-9);
        for f in 0..frames {
            out[f * width + b] = ((rows[f * width + b] / p98).min(1.0) * 255.0) as u8;
        }
    }
    out
}

/// Removes the slow trend (moving average over ~0.5 s), keeps rises, scales to unit std.
fn normalize(x: &[f32]) -> Vec<f32> {
    let w = 20;
    let mut pre = vec![0f64; x.len() + 1];
    for (i, v) in x.iter().enumerate() {
        pre[i + 1] = pre[i] + *v as f64;
    }
    let out: Vec<f32> = (0..x.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(w), (i + w + 1).min(x.len()));
            (x[i] - ((pre[b] - pre[a]) / (b - a) as f64) as f32).max(0.0)
        })
        .collect();
    let mean = out.iter().sum::<f32>() / out.len() as f32;
    let sd = (out.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / out.len() as f32).sqrt().max(1e-9);
    out.iter().map(|v| v / sd).collect()
}

/// Frames per beat, from onset autocorrelation weighted toward 120 BPM.
fn tempo(on: &[f32], fps: f64) -> f64 {
    let ac = |lag: usize| -> f64 {
        let n = on.len() - lag;
        (0..n).map(|i| (on[i] * on[i + lag]) as f64).sum::<f64>() / n as f64
    };
    let min_lag = (fps * 60.0 / 200.0).floor().max(2.0) as usize;
    let max_lag = ((fps * 60.0 / 60.0).ceil() as usize).min(on.len() / 2);
    let (mut best, mut lag) = (f64::MIN, min_lag);
    for l in min_lag..=max_lag {
        let bpm = 60.0 * fps / l as f64;
        let prior = (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
        let s = ac(l) * prior;
        if s > best {
            best = s;
            lag = l;
        }
    }
    let (a, b, c) = (ac(lag - 1), ac(lag), ac(lag + 1));
    let den = a - 2.0 * b + c;
    lag as f64 + if den < 0.0 { 0.5 * (a - c) / den } else { 0.0 }
}

/// Ellis 2007: best chain of onsets spaced about one period apart.
fn track(on: &[f32], period: f64) -> Vec<usize> {
    let n = on.len();
    let (lo, hi) = (((period / 2.0).round() as usize).max(1), (period * 2.0).round() as usize);
    let mut score = vec![0f64; n];
    let mut back = vec![usize::MAX; n];
    for t in 0..n {
        let (mut best, mut arg) = (0.0, usize::MAX);
        if t >= lo {
            for p in t.saturating_sub(hi)..=t - lo {
                let d = ((t - p) as f64 / period).ln();
                let s = score[p] - 100.0 * d * d;
                if s > best {
                    best = s;
                    arg = p;
                }
            }
        }
        score[t] = on[t] as f64 + best;
        back[t] = arg;
    }
    let mut t = (n.saturating_sub(hi)..n).max_by(|&a, &b| score[a].total_cmp(&score[b])).unwrap_or(0);
    let mut beats = vec![t];
    while back[t] != usize::MAX {
        t = back[t];
        beats.push(t);
    }
    beats.reverse();
    beats
}

fn phase_score(bass: &[f32], beats: &[usize], k: usize) -> f64 {
    let v: Vec<f64> = beats.iter().skip(k).step_by(4).map(|&f| bass[f] as f64).collect();
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

#[cfg(test)]
mod tests {
    #[test]
    fn spectrum_puts_a_tone_in_its_band_and_covers_every_band() {
        let sr = 22050u32;
        let x: Vec<f32> = (0..sr as usize * 3).map(|i| (i as f32 * 1000.0 * 6.2832 / sr as f32).sin() * 0.5).collect();
        let a = super::analyze(&x, sr);
        let s = super::SPEC;
        let mean = |b: usize| a.spec.iter().skip(b).step_by(s).map(|&v| v as u32).sum::<u32>();
        let loud = (0..s).max_by_key(|&b| mean(b)).unwrap();
        let bin_hz = sr as f64 / super::N as f64;
        let kmax = (super::N / 2).min((16000.0 / bin_hz) as usize);
        assert_eq!(Some(loud), super::spec_band((1000.0 / bin_hz).round() as usize, kmax));
        // every band owns at least one FFT bin, so no segment is always dark
        for b in 0..s {
            assert!((0..super::N / 2).any(|k| super::spec_band(k, kmax) == Some(b)), "band {b} empty");
        }
    }

    #[test]
    fn short_audio_does_not_crash() {
        for secs in [0.1, 0.4, 0.6, 1.9] {
            let a = super::analyze(&vec![0.1; (22050.0 * secs) as usize], 22050);
            assert!(a.beats.is_empty());
        }
    }

    #[test]
    fn finds_tempo_beats_and_downbeats_of_a_click_track() {
        let sr = 22050u32;
        let mut x = vec![0f32; sr as usize * 12];
        let mut seed = 1u32;
        for k in 0..24 {
            let start = ((0.25 + k as f64 * 0.5) * sr as f64) as usize;
            let accent = k % 4 == 1; // downbeat = 2nd click, with a low thump
            for i in 0..600 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
                let decay = (-(i as f32) / 120.0).exp();
                let thump = if accent { (i as f32 * 60.0 * 6.283 / sr as f32).sin() * 2.0 } else { 0.0 };
                x[start + i] += (noise * 0.5 + thump) * decay;
            }
        }
        let a = super::analyze(&x, sr);
        assert!((a.bpm - 120.0).abs() < 1.5, "bpm {}", a.bpm);
        let mut errs: Vec<f64> = a.beats.iter().map(|b| ((b - 0.25) / 0.5 - ((b - 0.25) / 0.5).round()).abs() * 0.5).collect();
        errs.sort_by(f64::total_cmp);
        assert!(errs[errs.len() / 2] < 0.025, "median beat error {}s", errs[errs.len() / 2]);
        let d = a.downbeats[1];
        assert!(((d - 0.75) / 2.0 - ((d - 0.75) / 2.0).round()).abs() < 0.05, "downbeat {d}");
    }
}
