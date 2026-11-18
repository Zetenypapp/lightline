//! Live input: analyses whatever the PC is playing, frame by frame, into a rolling `Analysis` that the
//! effects read exactly like an imported song's. Levels scale themselves to the music (slow auto gain),
//! and the beat grid is re-estimated twice a second from the last 8 s.

use crate::analysis::{self, Analysis, SPEC};
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

const N: usize = 2048;
const HOP: usize = 512;
const KEEP: f64 = 12.0; // seconds of history kept

pub struct Live {
    fft: Arc<dyn Fft<f32>>,
    win: Vec<f32>,
    buf: Vec<Complex<f32>>,
    pending: Vec<f32>,
    prev: Vec<f32>,
    band_of: Vec<Option<usize>>,
    lo_end: usize,
    mid_end: usize,
    lead_lo: usize,
    lead_hi: usize,
    peak: Vec<f32>, // running maximum per column (3 bands, SPEC bands, lead), for 0..255 scaling
    decay: f32,
    onset: Vec<f32>,
    trimmed: usize, // frames dropped from the front
    pub an: Analysis,
}

impl Live {
    pub fn new(sr: u32) -> Self {
        let srf = sr as f64;
        let fps = srf / HOP as f64;
        let bin_hz = srf / N as f64;
        let kmax = (N / 2).min((16000.0 / bin_hz) as usize);
        Live {
            fft: FftPlanner::new().plan_fft_forward(N),
            win: (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect(),
            buf: vec![Complex::new(0.0, 0.0); N],
            pending: vec![],
            prev: vec![0.0; N / 2],
            band_of: (0..N / 2).map(|k| analysis::spec_band(k, kmax)).collect(),
            lo_end: (150.0 / bin_hz).ceil() as usize,
            mid_end: (2000.0 / bin_hz).ceil() as usize,
            lead_lo: (500.0 / bin_hz) as usize,
            lead_hi: (5000.0 / bin_hz) as usize,
            peak: vec![1e-3; 3 + SPEC + 1],
            decay: (-1.0 / (8.0 * fps as f32)).exp(), // forget a loud moment over ~8 s
            onset: vec![],
            trimmed: 0,
            an: Analysis { bpm: 120.0, beats: vec![], downbeats: vec![], fps, lag: (N / 2) as f64 / srf, bands: vec![], spec: vec![], lead: vec![], sections: vec![] },
        }
    }

    /// Feeds mono samples; analyses every full window.
    pub fn push(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
        let before = self.onset.len();
        while self.pending.len() >= N {
            self.frame();
            self.pending.drain(..HOP);
        }
        let fps = self.an.fps;
        if self.onset.len() / (fps as usize / 2).max(1) != before / (fps as usize / 2).max(1) {
            self.beats();
        }
        // Keep KEEP seconds: drop the oldest half when there is 1.5x that.
        let keep = (KEEP * fps) as usize;
        if self.onset.len() > keep * 3 / 2 {
            let k = self.onset.len() - keep;
            self.onset.drain(..k);
            self.an.bands.drain(..k * 3);
            self.an.spec.drain(..k * SPEC);
            self.an.lead.drain(..k * 2);
            self.trimmed += k;
            self.an.lag += k as f64 / fps;
        }
    }

    fn scale(&mut self, col: usize, v: f32) -> u8 {
        let p = &mut self.peak[col];
        *p = (*p * self.decay).max(v).max(1e-3);
        (v / *p * 255.0).round().clamp(0.0, 255.0) as u8
    }

    fn frame(&mut self) {
        for i in 0..N {
            self.buf[i] = Complex::new(self.pending[i] * self.win[i], 0.0);
        }
        self.fft.process(&mut self.buf);
        let (mut flux, mut e, mut sp) = (0f32, [0f32; 3], [0f32; SPEC]);
        for k in 1..N / 2 {
            let m = self.buf[k].norm();
            let lm = (1.0 + 10.0 * m).ln();
            flux += (lm - self.prev[k]).max(0.0);
            self.prev[k] = lm;
            e[if k < self.lo_end { 0 } else if k < self.mid_end { 1 } else { 2 }] += m * m;
            if let Some(b) = self.band_of[k] {
                sp[b] += m * m;
            }
        }
        self.onset.push(flux);
        for (c, v) in e.into_iter().enumerate() {
            let u = self.scale(c, v.sqrt());
            self.an.bands.push(u);
        }
        for (c, v) in sp.into_iter().enumerate() {
            let u = self.scale(3 + c, v.sqrt());
            self.an.spec.push(u);
        }
        // Lead note: the biggest clear peak between 500 Hz and 5 kHz, placed on a log scale.
        let m = |k: usize| self.buf[k].norm();
        let kp = (self.lead_lo..self.lead_hi).max_by(|&a, &b| m(a).total_cmp(&m(b))).unwrap_or(self.lead_lo);
        let around = (kp.saturating_sub(24)..kp + 25).filter(|k| k.abs_diff(kp) > 2).map(m).sum::<f32>() / 44.0;
        let pos = ((kp as f32 / self.lead_lo as f32).ln() / (self.lead_hi as f32 / self.lead_lo as f32).ln()).clamp(0.0, 1.0);
        let amp = if m(kp) > 6.0 * around { m(kp) } else { 0.0 };
        let a = self.scale(3 + SPEC, amp);
        self.an.lead.extend([(pos * 255.0) as u8, a]);
    }

    /// Beat grid from the last 8 s, carried 3 s into the future.
    fn beats(&mut self) {
        let fps = self.an.fps;
        let w = (8.0 * fps) as usize;
        if self.onset.len() < (4.0 * fps) as usize {
            return;
        }
        let start = self.onset.len().saturating_sub(w);
        let on = analysis::normalize(&self.onset[start..]);
        let period = analysis::tempo(&on, fps);
        let frames = analysis::track(&on, period);
        let t_of = |f: usize| ((self.trimmed + start + f) * HOP + N / 2) as f64 / (fps * HOP as f64);
        let mut beats: Vec<f64> = frames.into_iter().map(t_of).collect();
        if let Some(&last) = beats.last() {
            let step = period / fps;
            let end = t_of(on.len()) + 3.0;
            let mut b = last + step;
            while b < end {
                beats.push(b);
                b += step;
            }
            self.an.bpm = (60.0 / step * 10.0).round() / 10.0;
        }
        self.an.beats = beats;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn follows_a_click_track() {
        // 120 BPM clicks at 48 kHz for 10 s, fed in small chunks like an audio callback.
        let sr = 48000;
        let mut s = vec![0f32; sr as usize * 10];
        for b in 0..20 {
            let at = b * sr as usize / 2;
            for i in 0..400 {
                s[at + i] = ((i as f32 * 0.3).sin()) * (1.0 - i as f32 / 400.0);
            }
        }
        let mut l = super::Live::new(sr);
        for c in s.chunks(480) {
            l.push(c);
        }
        assert!((l.an.bpm - 120.0).abs() < 3.0, "bpm {}", l.an.bpm);
        assert!(!l.an.bands.is_empty() && l.an.bands.len() / 3 == l.an.spec.len() / super::SPEC);
    }
}
