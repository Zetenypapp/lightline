//! Decoding (symphonia) and playback (cpal). Playback position is the master clock.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

/// Stereo i16 interleaved (half the memory of f32).
pub struct Track {
    pub pcm: Vec<i16>,
    pub rate: u32,
}

/// Returns the stereo track for playback and a mono f32 copy for analysis.
pub fn decode(path: &Path) -> Result<(Track, Vec<f32>), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, MediaSourceStream::new(Box::new(file), Default::default()), &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Unsupported file: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("No audio track in this file")?;
    let (id, rate) = (track.id, track.codec_params.sample_rate.ok_or("Unknown sample rate")?);
    let mut dec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;

    let (mut pcm, mut mono) = (Vec::new(), Vec::new());
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue, // one bad frame: skip it, keep the song
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count();
        let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sb.copy_interleaved_ref(decoded);
        for f in sb.samples().chunks(ch) {
            let (l, r) = (f[0], if ch > 1 { f[1] } else { f[0] });
            pcm.push((l.clamp(-1.0, 1.0) * 32767.0) as i16);
            pcm.push((r.clamp(-1.0, 1.0) * 32767.0) as i16);
            mono.push((l + r) * 0.5);
        }
    }
    if mono.is_empty() {
        return Err("The file has no audio".into());
    }
    Ok((Track { pcm, rate }, mono))
}

#[derive(Default)]
pub struct Player {
    track: Mutex<Option<Arc<Track>>>,
    rate: AtomicU32,
    pos: AtomicU64, // f64 bits, in source frames
    playing: AtomicBool,
    ok: AtomicBool,
}

impl Player {
    /// The output stream lives on its own thread (cpal streams can't move between threads).
    pub fn start() -> Arc<Self> {
        let p = Arc::new(Self::default());
        let q = p.clone();
        std::thread::spawn(move || match open_stream(q.clone()) {
            Ok(_stream) => {
                q.ok.store(true, Relaxed);
                loop {
                    std::thread::park();
                }
            }
            Err(e) => eprintln!("audio output: {e}"),
        });
        p
    }

    pub fn set(&self, t: Track) {
        self.playing.store(false, Relaxed);
        self.rate.store(t.rate, Relaxed);
        self.pos.store(0f64.to_bits(), Relaxed);
        *self.track.lock().unwrap() = Some(Arc::new(t));
    }

    pub fn seek(&self, secs: f64) {
        let f = secs.max(0.0) * self.rate.load(Relaxed) as f64;
        self.pos.store(f.to_bits(), Relaxed);
    }

    pub fn play(&self, at: f64) -> Result<(), String> {
        if !self.ok.load(Relaxed) {
            return Err("No audio output device".into());
        }
        self.seek(at);
        self.playing.store(true, Relaxed);
        Ok(())
    }

    pub fn pause(&self) {
        self.playing.store(false, Relaxed);
    }

    pub fn state(&self) -> (f64, bool) {
        let rate = self.rate.load(Relaxed).max(1) as f64;
        (f64::from_bits(self.pos.load(Relaxed)) / rate, self.playing.load(Relaxed))
    }
}

fn open_stream(p: Arc<Player>) -> Result<cpal::Stream, String> {
    let dev = cpal::default_host().default_output_device().ok_or("no output device")?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("unsupported output format {:?}", cfg.sample_format()));
    }
    let ch = cfg.channels() as usize;
    let dev_rate = cfg.sample_rate().0 as f64;
    let stream = dev
        .build_output_stream(&cfg.into(), move |out: &mut [f32], _| fill(&p, out, ch, dev_rate), |e| eprintln!("audio: {e}"), None)
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn fill(p: &Player, out: &mut [f32], ch: usize, dev_rate: f64) {
    out.fill(0.0);
    if !p.playing.load(Relaxed) {
        return;
    }
    let Ok(g) = p.track.try_lock() else { return };
    let Some(t) = g.as_ref() else { return };
    let frames = t.pcm.len() / 2;
    let step = t.rate as f64 / dev_rate;
    let start = p.pos.load(Relaxed);
    let mut pos = f64::from_bits(start);
    let s = |j: usize| t.pcm[j] as f32 / 32768.0;
    for f in out.chunks_mut(ch) {
        let i = pos as usize;
        if i + 1 >= frames {
            p.playing.store(false, Relaxed);
            break;
        }
        let k = (pos - i as f64) as f32; // linear resample to the device rate
        f[0] = s(2 * i) + (s(2 * i + 2) - s(2 * i)) * k;
        if ch > 1 {
            f[1] = s(2 * i + 1) + (s(2 * i + 3) - s(2 * i + 1)) * k;
        }
        pos += step;
    }
    // A seek during this buffer wins over our advance.
    let _ = p.pos.compare_exchange(start, pos.to_bits(), Relaxed, Relaxed);
}
