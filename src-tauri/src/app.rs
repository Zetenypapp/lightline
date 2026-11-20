//! The native UI (egui): Home, Editor, Live.

use crate::analysis::Analysis;
use crate::audio::{self, Player};
use crate::fx::{self, Ctx, Rgb, Smoother, EFFECTS};
use crate::govee;
use crate::lights::{self, Lights, Output, Show};
use crate::scene::{self, Audio, Clip, Model, Scene, Settings, Track, COMPOUND};
use crate::show;
use crate::analysis::Part;
use eframe::egui::{self, Align, Color32, CornerRadius, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BG: Color32 = Color32::from_rgb(0x14, 0x15, 0x1b);
const PANEL: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x28);
const PANEL2: Color32 = Color32::from_rgb(0x26, 0x29, 0x35);
const PANEL3: Color32 = Color32::from_rgb(0x2f, 0x33, 0x41);
const TEXT: Color32 = Color32::from_rgb(0xee, 0xf0, 0xf6);
const MUTED: Color32 = Color32::from_rgb(0x9e, 0xa3, 0xb5);
const ACCENT: Color32 = Color32::from_rgb(0x2f, 0xd8, 0xb4);
const ACCENT2: Color32 = Color32::from_rgb(0x7c, 0x6c, 0xff);
const ON_ACCENT: Color32 = Color32::from_rgb(0x05, 0x22, 0x1b);
const WARN: Color32 = Color32::from_rgb(0xff, 0xc2, 0x4b);
const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0x4d, 0x5e);
const STAGE_BG: Color32 = Color32::from_rgb(0x12, 0x13, 0x19);
// Clip colours on the timeline: patterns, music effects, compound clips.
const PAT_C: Color32 = Color32::from_rgb(0x6a, 0x4f, 0xd8);
const REACT_C: Color32 = Color32::from_rgb(0x1f, 0x9e, 0x86);
const CMP_C: Color32 = Color32::from_rgb(0x3d, 0xa0, 0x5a);
const AUDIO_EXT: [&str; 6] = ["mp3", "wav", "flac", "ogg", "m4a", "aac"];
const LABEL_W: f32 = 180.0;
const ROW_H: f32 = 46.0;

/// Most colours per frame a light accepts (more and it rejects the frame and goes dark), measured on the
/// user's lights. Other models start at their Govee-app segment count.
const SEG_MEASURED: [(&str, usize); 4] = [("H61F6", 84), ("H619C", 40), ("H6609", 10), ("H60A1", 1)];
/// Lights without real-time mode: driven with one colour by the plain colour command (tested: H60A1 ceiling light).
const NO_STREAM: [&str; 1] = ["H60A1"];
/// A track whose light is this runs across all lights in the chain order.
const CHAIN: &str = "chain";
/// Lights seen keeping up with 120 frames/s (H61F6 can't; unknown models get 30).
const FAST_MEASURED: [&str; 1] = ["H619C"];

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Home,
    Editor,
    Live,
    React,
}

#[derive(Clone)]
enum Drop {
    Fx(&'static str),
    Light(String),
    Preset(Clip),
}

/// The library's tabs (top left of the editor).
#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Song,
    Patterns,
    Reactive,
    Show,
    Colours,
    Presets,
    Lights,
}
const TABS: [(Tab, &str, &str); 7] = [
    (Tab::Song, "🎵", "Song"),
    (Tab::Patterns, "🌈", "Patterns"),
    (Tab::Reactive, "⚡", "Reactive"),
    (Tab::Show, "🎬", "Show"),
    (Tab::Colours, "🎨", "Colours"),
    (Tab::Presets, "⭐", "Presets"),
    (Tab::Lights, "💡", "Lights"),
];

pub struct Device {
    ip: String,
    id: String,
    sku: String,
}

struct Drag {
    ti: usize,
    ci: usize,
    resize: bool,
    s0: f64,
    l0: f64,
    target: usize,
}

type Loaded = (String, Result<(Analysis, f64), String>);

/// Following a media player (Riff, Spotify, ...): its song's scene in sync, or reacting live.
struct Follow {
    rx: Receiver<Option<crate::media::NowPlaying>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    np: Option<crate::media::NowPlaying>,
    at: Instant,                   // when np was read
    scene: Option<usize>,          // the scene being played along
    synced: Option<(Instant, f64)>, // when the lights were last started, at which song time
}

/// Reacting to whatever the PC is playing.
struct LiveIn {
    _stream: cpal::Stream,
    buf: Arc<std::sync::Mutex<Vec<f32>>>,
    live: crate::live::Live,
    an: Arc<Analysis>,
    t0: Instant,
}

const LIVE_LOOKS: [&str; 10] = ["Visualizer", "Spectrum", "Ripple", "Kick", "Bass VU", "Meter", "Hi-hats", "Glitch", "Snare", "Voice"];

pub struct App {
    player: Arc<Player>,
    lights: Arc<Lights>,
    scenes: Vec<Scene>,
    cur: usize,
    page: Page,
    settings: Settings,
    models: HashMap<&'static str, Model>,
    devices: Vec<Device>,
    scan_rx: Option<Receiver<Result<Vec<Device>, String>>>,
    status: String,
    sel: Option<(usize, usize)>,
    ti: usize,
    t: f64,
    playing: bool,
    clock: (Instant, f64),
    snap: bool,
    zoom: f32,
    scroll_to: Option<f32>,
    an: Option<Arc<Analysis>>,
    an_path: String,
    an_rx: Option<Receiver<Loaded>>,
    loading: String,
    importing: Option<Option<Audio>>, // previous audio of a scene whose new song is loading
    autoplay: bool,                   // play as soon as the song has loaded
    bri: f64,
    black: bool,
    blink: HashSet<String>,
    pv: Smoother,
    epoch: Instant,
    dirty: bool,
    settings_dirty: bool,
    last_save: Instant,
    no_save: bool,
    drag: Option<Drag>,
    undo: Vec<Vec<Scene>>,
    redo: Vec<Vec<Scene>>,
    committed: Vec<Scene>, // scenes as of the last finished edit
    clip_board: Option<Clip>,
    flash: bool,
    tab: Tab,
    inside: Option<(usize, usize)>, // the compound clip open in the timeline: (track, clip)
    multi: Vec<usize>,              // more selected clips, on the selected clip's row (Ctrl+click)
    fit_to: Option<(f64, f64)>,     // zoom the timeline to this (start, length) next frame
    confirm_del: Option<usize>, // scene waiting for a second click on Delete
    live_in: Option<LiveIn>,
    ended: bool, // the scene just played to its end
    follow: Option<Follow>,
    update: Option<(String, String)>, // newer release: (version, page)
    update_rx: Option<Receiver<(String, String)>>,
    live_fx: usize,
    live_pal: usize, // 0 = rainbow, else PALETTES[live_pal - 1]
    build_opts: show::Opts,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, player: Arc<Player>, lights: Arc<Lights>) -> Self {
        style(&cc.egui_ctx);
        let (scenes, no_save, status) = match scene::load_scenes() {
            Ok(Some(s)) if !s.is_empty() => (s, false, String::new()),
            Ok(_) => (vec![scene::demo()], false, String::new()),
            // Never overwrite a scenes file we failed to read.
            Err(e) => (vec![scene::demo()], true, format!("Saving is off: {e}")),
        };
        let mut app = App {
            player,
            lights,
            scenes,
            cur: 0,
            page: Page::Home,
            settings: scene::load_settings(),
            models: scene::models(),
            devices: vec![],
            scan_rx: None,
            status,
            sel: None,
            ti: 0,
            t: 0.0,
            playing: false,
            clock: (Instant::now(), 0.0),
            snap: true,
            zoom: 1.0,
            scroll_to: None,
            an: None,
            an_path: String::new(),
            an_rx: None,
            loading: String::new(),
            importing: None,
            autoplay: false,
            bri: 100.0,
            black: false,
            blink: HashSet::new(),
            pv: Smoother::default(),
            epoch: Instant::now(),
            dirty: false,
            settings_dirty: false,
            last_save: Instant::now(),
            no_save,
            drag: None,
            undo: vec![],
            redo: vec![],
            committed: vec![],
            clip_board: None,
            flash: false,
            tab: Tab::Patterns,
            inside: None,
            multi: vec![],
            fit_to: None,
            confirm_del: None,
            live_in: None,
            ended: false,
            follow: None,
            update: None,
            update_rx: None,
            live_fx: 0,
            live_pal: 0,
            build_opts: show::Opts::default(),
        };
        app.committed = app.scenes.clone();
        app.check_update(&cc.egui_ctx);
        app.scan(&cc.egui_ctx);
        app.ensure_audio(&cc.egui_ctx);
        app
    }

    fn sc(&self) -> &Scene {
        &self.scenes[self.cur]
    }

    fn sc_mut(&mut self) -> &mut Scene {
        self.dirty = true;
        &mut self.scenes[self.cur]
    }

    fn cx(&self) -> Ctx<'_> {
        Ctx { an: self.an.as_deref(), bpm: self.sc().bpm }
    }

    fn dev_name(&self, d: &Device) -> String {
        if let Some(n) = self.settings.names.get(&d.id) {
            return n.clone();
        }
        self.models.get(d.sku.as_str()).map_or(d.sku.clone(), |m| format!("{} {}", m.name, m.len))
    }

    fn light_segs(&self, d: &Device) -> usize {
        self.settings.lsegs.get(&d.id).copied()
            .or_else(|| SEG_MEASURED.iter().find(|(s, _)| *s == d.sku).map(|(_, n)| *n))
            .or_else(|| self.models.get(d.sku.as_str()).map(|m| m.segs))
            .unwrap_or(10)
    }

    /// This light's smoothing, else the overall one.
    fn light_smooth(&self, id: &str) -> f64 {
        self.settings.lsmooth.get(id).copied().unwrap_or(self.settings.smooth)
    }

    /// All the segments of a light, for a track that just got it.
    fn full_segs(&self, id: &str) -> usize {
        if id == CHAIN {
            return self.devices.iter().map(|d| self.light_segs(d)).sum::<usize>().max(1);
        }
        self.devices.iter().find(|d| d.id == id).map_or(15, |d| self.light_segs(d))
    }

    fn track_light(&self, tr: &Track) -> String {
        match &tr.dev {
            None => "No light".into(),
            Some(id) if id == CHAIN => "All lights, chained".into(),
            Some(id) => self.devices.iter().find(|d| &d.id == id).map_or("Light not found".into(), |d| self.dev_name(d)),
        }
    }

    // ---------- background work ----------

    fn scan(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = govee::scan().map(|v| {
                v.iter()
                    .filter_map(|d| Some(Device { ip: d["ip"].as_str()?.into(), id: d["device"].as_str()?.into(), sku: d["sku"].as_str().unwrap_or("?").into() }))
                    .collect()
            });
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.scan_rx = Some(rx);
    }

    /// Loads (decodes + analyses) the current scene's song unless it is already loaded.
    fn ensure_audio(&mut self, ctx: &egui::Context) {
        let Some(a) = self.sc().audio.clone() else {
            self.an = None;
            return;
        };
        if self.an.is_some() && self.an_path == a.path {
            return;
        }
        self.an = None;
        self.loading = "Analysing music…".into();
        let (tx, rx) = channel();
        let (player, ctx) = (self.player.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = audio::decode(std::path::Path::new(&a.path)).map(|(track, mono)| {
                let an = crate::analysis::analyze(&mono, track.rate);
                let dur = track.pcm.len() as f64 / 2.0 / track.rate as f64;
                player.set(track);
                (an, dur)
            });
            let _ = tx.send((a.path, r));
            ctx.request_repaint();
        });
        self.an_rx = Some(rx);
    }

    fn import_audio(&mut self, ctx: &egui::Context, path: String) {
        self.set_playing(false);
        let name = std::path::Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned());
        let old = self.sc_mut().audio.replace(Audio { path, name });
        self.importing = Some(old);
        self.an = None;
        self.ensure_audio(ctx);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(Ok(r)) = self.scan_rx.as_ref().map(|rx| rx.try_recv()) {
            self.scan_rx = None;
            match r {
                Ok(d) => {
                    self.devices = d;
                    if !self.playing {
                        // Free lights left in real-time mode by a crash or a lost packet.
                        let ips: Vec<String> = self.devices.iter().map(|d| d.ip.clone()).collect();
                        std::thread::spawn(move || lights::set_mode(&ips, false));
                    }
                }
                Err(e) => self.status = e,
            }
        }
        if let Some(Ok((path, r))) = self.an_rx.as_ref().map(|rx| rx.try_recv()) {
            self.an_rx = None;
            if self.sc().audio.as_ref().map(|a| &a.path) != Some(&path) {
                // Another scene was opened meanwhile, and the player holds this song: load the right one.
                self.ensure_audio(ctx);
                return;
            }
            match r {
                Ok((an, dur)) => {
                    let bpm = an.bpm;
                    (self.an, self.an_path, self.loading) = (Some(Arc::new(an)), path, String::new());
                    if self.importing.take().is_some() {
                        let s = self.sc_mut();
                        (s.len, s.bpm) = (dur.ceil(), bpm.round());
                    }
                    if std::mem::take(&mut self.autoplay) {
                        self.set_playing(true);
                    }
                }
                Err(e) => {
                    self.autoplay = false;
                    self.loading = format!("Couldn't load music: {e}");
                    if let Some(old) = self.importing.take() {
                        self.sc_mut().audio = old;
                    }
                }
            }
        }
    }

    /// Saves changes at most every 0.4 s (or right away with `now`, on exit).
    fn save(&mut self, now: bool) {
        if self.no_save || (!now && self.last_save.elapsed() < Duration::from_millis(400)) {
            return;
        }
        if self.dirty {
            self.dirty = false;
            if let Err(e) = scene::save_scenes(&self.scenes) {
                self.status = format!("Save failed: {e}");
            }
        }
        if self.settings_dirty {
            self.settings_dirty = false;
            if let Err(e) = scene::save_settings(&self.settings) {
                self.status = format!("Save failed: {e}");
            }
        }
        self.last_save = Instant::now();
    }

    // ---------- playback ----------

    /// One track per light, playing the chosen live look.
    fn live_tracks(&self) -> Vec<Track> {
        let (col, col2, pal) = match self.live_pal {
            0 => ([0x2f, 0xd8, 0xb4], [0x7c, 0x6c, 0xff], vec![]),
            p => {
                let [d, m, a, h] = show::PALETTES[p - 1].1;
                (m, a, vec![d, m, a, h])
            }
        };
        self.devices.iter().enumerate().map(|(i, d)| Track {
            name: self.dev_name(d),
            segs: self.light_segs(d),
            from: 0,
            dev: Some(d.id.clone()),
            muted: false,
            locked: false,
            clips: vec![Clip { fx: LIVE_LOOKS[self.live_fx].into(), start: 0.0, len: 1e9, col, col2, pal: pal.clone(), rev: i % 2 == 1, ..Default::default() }],
        }).collect()
    }

    fn start_live(&mut self) {
        self.stop_follow();
        self.start_live_keep_follow();
    }

    fn start_live_keep_follow(&mut self) {
        self.set_playing(false);
        let buf = Arc::new(std::sync::Mutex::new(vec![]));
        match audio::capture(buf.clone()) {
            Ok((stream, rate)) => {
                let live = crate::live::Live::new(rate);
                let an = Arc::new(live.an.clone());
                self.live_in = Some(LiveIn { _stream: stream, buf, live, an, t0: Instant::now() });
                self.lights.set_show(self.show());
                self.lights.play(0.0, false);
            }
            Err(e) => self.status = format!("Can't listen to the PC audio: {e}"),
        }
    }

    fn start_follow(&mut self, ctx: &egui::Context) {
        self.set_playing(false);
        let (tx, rx) = channel();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c = ctx.clone();
        crate::media::watch(tx, stop.clone(), move || c.request_repaint());
        self.follow = Some(Follow { rx, stop, np: None, at: Instant::now(), scene: None, synced: None });
    }

    fn stop_follow(&mut self) {
        if let Some(f) = self.follow.take() {
            f.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if f.synced.is_some() {
                let l = self.lights.clone();
                std::thread::spawn(move || l.stop());
            }
        }
        self.stop_live();
    }

    /// Follow mode, every frame: play the matching scene in sync with the player, else react live.
    fn tick_follow(&mut self, ctx: &egui::Context) {
        let Some(f) = self.follow.as_mut() else { return };
        while let Ok(np) = f.rx.try_recv() {
            (f.np, f.at) = (np, Instant::now());
        }
        let np = f.np.clone().filter(|n| n.playing);
        let songs: Vec<(Option<&str>, usize)> = self.scenes.iter().map(|s| (s.audio.as_ref().map(|a| a.name.as_str()), s.tracks.iter().map(|t| t.clips.len()).sum())).collect();
        let want = np.as_ref().and_then(|n| crate::media::match_scene(&n.title, &songs));
        let f = self.follow.as_mut().unwrap();
        let had = f.synced.is_some();
        if want != f.scene || np.is_none() {
            f.scene = want;
            f.synced = None;
            if had {
                let l = self.lights.clone();
                std::thread::spawn(move || l.stop());
            }
        }
        match (np, want) {
            (Some(n), Some(i)) => {
                self.stop_live();
                if self.cur != i {
                    let page = self.page;
                    self.open(ctx, i, page);
                }
                let f = self.follow.as_ref().unwrap();
                let t = n.pos + f.at.elapsed().as_secs_f64();
                if self.an.is_none() {
                    return; // the scene's song is still being analysed
                }
                self.t = t.min(self.sc().len);
                // (Re)start the lights' clock when it starts or drifts more than 0.15 s from the player.
                let lights_t = f.synced.map(|(at, t0)| t0 + at.elapsed().as_secs_f64());
                if lights_t.is_none_or(|lt| (lt - t).abs() > 0.15) {
                    self.lights.set_show(self.show());
                    self.lights.play(t, false);
                    self.follow.as_mut().unwrap().synced = Some((Instant::now(), t));
                }
                self.lights.set_show(self.show());
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            (Some(_), None) => {
                if self.live_in.is_none() {
                    self.start_live_keep_follow();
                }
            }
            (None, _) => self.stop_live(),
        }
    }

    fn stop_live(&mut self) {
        if self.live_in.take().is_some() {
            let l = self.lights.clone();
            std::thread::spawn(move || l.stop());
        }
    }

    /// Feeds new PC audio into the live analysis and the lights.
    fn tick_live(&mut self, ctx: &egui::Context) {
        let Some(l) = self.live_in.as_mut() else { return };
        let samples = std::mem::take(&mut *l.buf.lock().unwrap());
        if !samples.is_empty() {
            l.live.push(&samples);
            l.an = Arc::new(l.live.an.clone());
        }
        self.lights.set_show(self.show());
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    fn show(&self) -> Show {
        if let Some(l) = &self.live_in {
            let mut s = self.show_of(&self.live_tracks());
            s.an = Some(l.an.clone());
            return s;
        }
        self.show_of(&self.sc().tracks)
    }

    /// Asks GitHub (with Windows' own curl) whether a newer release exists. Silent if offline.
    fn check_update(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        self.update_rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            use std::os::windows::process::CommandExt;
            let out = std::process::Command::new("curl.exe")
                .args(["-s", "-m", "6", "-H", "User-Agent: lightline", "https://api.github.com/repos/Zetenypapp/lightline/releases/latest"])
                .creation_flags(0x0800_0000) // no console window
                .output();
            let Ok(out) = out else { return };
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return };
            let (Some(tag), Some(url)) = (v["tag_name"].as_str(), v["html_url"].as_str()) else { return };
            if newer(tag.trim_start_matches('v'), env!("CARGO_PKG_VERSION")) {
                let _ = tx.send((tag.to_string(), url.to_string()));
                ctx.request_repaint();
            }
        });
    }

    /// Lights in chain order: the saved order first, then any others.
    fn chain_devices(&self) -> Vec<&Device> {
        let mut v: Vec<&Device> = self.settings.chain.iter().filter_map(|id| self.devices.iter().find(|d| &d.id == id)).collect();
        v.extend(self.devices.iter().filter(|d| !self.settings.chain.contains(&d.id)));
        v
    }

    fn show_of(&self, tracks: &[Track]) -> Show {
        let mut outputs: Vec<Output> = vec![];
        let mut add = |d: &Device, tr: &Track, win: (usize, usize)| match outputs.iter_mut().find(|o| o.ip == d.ip) {
            Some(o) => {
                o.tracks.push(tr.clone());
                o.win.push(win);
            }
            None => outputs.push(Output { ip: d.ip.clone(), max: self.light_segs(d), lead: -self.settings.loff_ms.get(&d.id).copied().unwrap_or(0.0) / 1000.0, fast: FAST_MEASURED.contains(&d.sku.as_str()), smooth: self.light_smooth(&d.id), simple: NO_STREAM.contains(&d.sku.as_str()), tracks: vec![tr.clone()], win: vec![win] }),
        };
        for tr in tracks {
            if tr.dev.as_deref() == Some(CHAIN) {
                let mut at = 0;
                for d in self.chain_devices() {
                    let n = self.light_segs(d);
                    add(d, tr, (at, n));
                    at += n;
                }
                continue;
            }
            let Some(d) = tr.dev.as_ref().and_then(|id| self.devices.iter().find(|d| &d.id == id)) else { continue };
            add(d, tr, (0, usize::MAX));
        }
        for o in &mut outputs {
            let w = o.tracks.iter().zip(&o.win).map(|(t, &(skip, take))| t.from + t.segs.saturating_sub(skip).min(take)).max().unwrap_or(1).min(84);
            o.max = o.max.clamp(1, w.max(1));
        }
        Show {
            outputs,
            an: self.an.clone(),
            bpm: self.sc().bpm,
            bri: self.bri / 100.0,
            off: self.settings.off_ms / 1000.0,
            black: self.black,
            flash: self.flash,
        }
    }

    fn set_playing(&mut self, on: bool) {
        if on {
            self.stop_follow();
        }
        if on == self.playing {
            return;
        }
        if on {
            if self.t >= self.sc().len {
                self.t = 0.0;
            }
            if self.an.is_some() {
                if let Err(e) = self.player.play(self.t) {
                    self.status = e;
                    return;
                }
            }
            self.playing = true;
            self.clock = (Instant::now(), self.t);
            self.pv.reset();
            self.lights.set_show(self.show());
            self.lights.play(self.t, self.an.is_some());
        } else {
            self.playing = false;
            self.player.pause();
            let l = self.lights.clone();
            std::thread::spawn(move || l.stop());
        }
    }

    fn seek(&mut self, t: f64) {
        self.t = t.clamp(0.0, self.sc().len);
        if self.an.is_some() {
            self.player.seek(self.t);
        } else if self.playing {
            self.clock = (Instant::now(), self.t);
            self.lights.play(self.t, false); // without music the lights keep their own clock
        }
    }

    fn advance(&mut self) {
        if !self.playing {
            return;
        }
        if self.an.is_some() {
            let (t, playing) = self.player.state();
            self.t = t.min(self.sc().len);
            if !playing {
                self.set_playing(false);
                self.ended = true;
                return;
            }
        } else {
            self.t = self.clock.1 + self.clock.0.elapsed().as_secs_f64();
        }
        if self.t >= self.sc().len {
            self.t = self.sc().len;
            self.set_playing(false);
            self.ended = true;
        }
    }

    /// Playlist: when a scene ends on Live, start the next (or a random other) one.
    fn next_in_playlist(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.ended) || !self.settings.autonext || self.page != Page::Live || self.scenes.len() < 2 {
            return;
        }
        let n = self.scenes.len();
        let next = if self.settings.shuffle {
            let r = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() as usize);
            (self.cur + 1 + r % (n - 1)) % n // any other scene
        } else {
            (self.cur + 1) % n
        };
        self.open(ctx, next, Page::Live);
        self.play_when_loaded();
    }

    fn open(&mut self, ctx: &egui::Context, i: usize, page: Page) {
        self.set_playing(false);
        (self.cur, self.t, self.sel, self.ti, self.page, self.inside) = (i, 0.0, None, 0, page, None);
        self.multi.clear();
        if self.sc().audio.as_ref().map(|a| &a.path) != Some(&self.an_path) {
            self.an = None; // never show or play another scene's song
        }
        self.ensure_audio(ctx);
    }

    fn add_clip(&mut self, ti: usize, fx: &'static str, at: f64) {
        let beat = 60.0 / self.an.as_ref().map(|a| a.bpm).filter(|b| *b > 0.0).unwrap_or(self.sc().bpm).max(1.0);
        let len = (4.0 * beat).min(self.sc().len);
        let start = (if self.snap { self.cx().snap(at) } else { at }).clamp(0.0, (self.sc().len - len).max(0.0));
        let h = (self.epoch.elapsed().as_nanos() % 360) as f32;
        let (col, col2) = fx::default_colors(fx).unwrap_or_else(|| (to_u8(fx::hsl(h, 85.0, 58.0)), to_u8(fx::hsl(h + 120.0, 85.0, 58.0))));
        let Some(lane) = self.lane_mut(ti) else { return };
        lane.push(Clip { fx: fx.into(), start, len, col, col2, ..Default::default() });
        let ci = lane.len() - 1;
        (self.sel, self.ti) = (Some((ti, ci)), ti);
        self.multi.clear();
    }

    /// A saved preset (or any clip) placed at `at` on row ti.
    fn add_preset(&mut self, ti: usize, mut c: Clip, at: f64) {
        let start = (if self.snap { self.cx().snap(at) } else { at }).clamp(0.0, (self.sc().len - c.len).max(0.0));
        c.shift(start - c.start);
        let Some(lane) = self.lane_mut(ti) else { return };
        lane.push(c);
        let ci = lane.len() - 1;
        (self.sel, self.ti) = (Some((ti, ci)), ti);
        self.multi.clear();
    }

    /// Rows the timeline shows: the tracks, or the one open compound clip.
    fn rows(&self) -> usize {
        if self.inside.is_some() { 1 } else { self.sc().tracks.len() }
    }

    /// The track a timeline row belongs to.
    fn real_track(&self, ti: usize) -> usize {
        self.inside.map_or(ti, |(t, _)| t)
    }

    /// The clips on a timeline row (inside the open compound clip, if one is open).
    fn lane(&self, ti: usize) -> &[Clip] {
        let tr = self.sc().tracks.get(self.real_track(ti));
        match self.inside {
            Some((_, c)) => tr.and_then(|t| t.clips.get(c)).map_or(&[][..], |c| &c.inner[..]),
            None => tr.map_or(&[][..], |t| &t.clips[..]),
        }
    }

    fn lane_mut(&mut self, ti: usize) -> Option<&mut Vec<Clip>> {
        self.dirty = true;
        let inside = self.inside;
        let tr = self.scenes[self.cur].tracks.get_mut(inside.map_or(ti, |(t, _)| t))?;
        match inside {
            Some((_, c)) => tr.clips.get_mut(c).map(|c| &mut c.inner),
            None => Some(&mut tr.clips),
        }
    }

    fn leave_compound(&mut self) {
        if let Some((t, _)) = self.inside.take() {
            (self.ti, self.sel) = (t, None);
            (self.zoom, self.scroll_to, self.fit_to) = (1.0, Some(0.0), None);
            self.multi.clear();
        }
    }

    /// Opens a compound clip in the timeline, zoomed to it.
    fn enter_compound(&mut self, ti: usize, ci: usize) {
        let Some(c) = self.lane(ti).get(ci) else { return };
        self.fit_to = Some((c.start, c.len));
        (self.inside, self.sel) = (Some((ti, ci)), None);
        self.multi.clear();
    }

    /// The selected clips (Ctrl+click picks several) become one compound clip.
    fn group(&mut self) {
        let Some((ti, ci)) = self.sel else { return };
        let mut idx: Vec<usize> = self.multi.iter().copied().chain([ci]).collect();
        idx.sort_unstable();
        idx.dedup();
        let n = self.sc().tracks.iter().flat_map(|t| &t.clips).filter(|c| c.fx == COMPOUND).count() + 1;
        let Some(lane) = self.lane_mut(ti) else { return };
        if idx.iter().any(|&i| i >= lane.len()) {
            return;
        }
        let mut inner: Vec<Clip> = idx.iter().rev().map(|&i| lane.remove(i)).collect();
        inner.sort_by(|a, b| a.start.total_cmp(&b.start));
        lane.push(scene::group(inner, format!("Compound {n}")));
        self.sel = Some((ti, lane.len() - 1));
        self.multi.clear();
    }

    fn ungroup(&mut self) {
        let Some((ti, ci)) = self.sel else { return };
        let Some(lane) = self.lane_mut(ti) else { return };
        if lane.get(ci).is_some_and(|c| c.fx == COMPOUND) {
            let g = lane.remove(ci);
            lane.extend(g.inner);
            self.sel = None;
        }
    }

    /// Cuts the selected clip in two at the playhead.
    fn split(&mut self) {
        let t = self.t;
        let Some((ti, _)) = self.sel else { return };
        let Some(c) = self.sel_clip() else { return };
        if t <= c.start + 0.05 || t >= c.start + c.len - 0.05 {
            return;
        }
        let mut b = c.clone();
        (c.len, c.fout) = (t - c.start, 0.0);
        (b.len, b.start, b.fin, b.trans) = (b.start + b.len - t, t, 0.0, 0);
        if let Some(lane) = self.lane_mut(ti) {
            lane.push(b);
            self.sel = Some((ti, lane.len() - 1));
        }
    }

    /// Ctrl+Z (back = true) / Ctrl+Y.
    fn history(&mut self, back: bool) {
        let (from, to) = if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(s) = from.pop() else { return };
        to.push(std::mem::replace(&mut self.scenes, s));
        self.committed = self.scenes.clone();
        self.cur = self.cur.min(self.scenes.len().saturating_sub(1));
        self.ti = self.ti.min(self.sc().tracks.len().saturating_sub(1));
        (self.sel, self.dirty, self.inside) = (None, true, None);
        self.multi.clear();
    }

    /// The Show tab: colours, speed, focus, strobes; each Build makes a new variation as a new scene.
    fn show_tab(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let go = ui.horizontal(|ui| {
            ui.label(RichText::new("Build a light show").size(18.0).strong());
            ui.add_space(12.0);
            self.an.is_some() && accent_button(ui, "  Build  ").on_hover_text("Each build is a new variation").clicked()
        }).inner;
        ui.label(RichText::new("Made from the song's beats and parts, one compound clip per part. Each build is a new variation, as a new scene.").color(MUTED).small());
        let Some(an) = self.an.clone() else {
            ui.add_space(8.0);
            ui.label(RichText::new("Load a song first (Song tab).").color(WARN));
            return;
        };
        let mut o = self.build_opts;
        let sets: Vec<(String, Vec<[u8; 3]>)> = show::PALETTES.iter().map(|(n, c)| (n.to_string(), c.to_vec())).chain(self.settings.colours.iter().cloned()).collect();
        if o.pal >= sets.len() {
            o.pal = 0;
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Colours").color(MUTED));
        ui.horizontal_wrapped(|ui| {
            for (i, (name, cols)) in sets.iter().enumerate() {
                if swatch_button(ui, o.pal == i, name, cols).clicked() {
                    o.pal = i;
                }
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Speed").color(MUTED));
        ui.horizontal(|ui| {
            for (i, n) in show::SPEEDS.iter().enumerate() {
                ui.selectable_value(&mut o.speed, i, *n);
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Focus on").color(MUTED));
        ui.horizontal_wrapped(|ui| {
            for (i, n) in show::FOCUS.iter().enumerate() {
                ui.selectable_value(&mut o.focus, i, *n);
            }
        });
        ui.add_space(8.0);
        ui.checkbox(&mut o.strobes, "Strobes");
        ui.checkbox(&mut o.same, "Same pattern on every light");
        self.build_opts = o;
        if !go {
            return;
        }
        let mut ids: Vec<String> = vec![];
        for id in self.sc().tracks.iter().filter_map(|t| t.dev.clone()) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            ids = self.devices.iter().map(|d| d.id.clone()).collect();
        }
        let lights: Vec<(String, String, usize)> = ids.iter().map(|id| {
            let name = self.devices.iter().find(|d| &d.id == id).map_or(id.clone(), |d| self.dev_name(d));
            (name, id.clone(), self.full_segs(id))
        }).collect();
        o.seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        let cols = &sets[o.pal].1;
        o.custom = (o.pal >= show::PALETTES.len()).then(|| std::array::from_fn(|k| cols.get(k % cols.len().max(1)).copied().unwrap_or([255; 3])));
        let mut s = self.sc().clone();
        // Building from a built scene: replace its "(colours · speed · focus)", don't stack another.
        let base = match s.name.find(" (") {
            Some(i) if s.name.ends_with(')') && s.name[i..].contains(" · ") => s.name[..i].to_string(),
            _ => s.name.clone(),
        };
        s.name = format!("{} ({} · {} · {})", base, sets[o.pal].0, show::SPEEDS[o.speed], show::FOCUS[o.focus]);
        s.tracks = show::build(&an, s.len, &lights, o);
        self.scenes.push(s);
        self.dirty = true;
        self.open(&ctx, self.scenes.len() - 1, Page::Editor);
    }

    fn sel_clip(&mut self) -> Option<&mut Clip> {
        let (ti, ci) = self.sel?;
        self.lane_mut(ti)?.get_mut(ci)
    }

    fn delete_clip(&mut self) {
        let Some((ti, ci)) = self.sel.take() else { return };
        let mut idx: Vec<usize> = std::mem::take(&mut self.multi).into_iter().chain([ci]).collect();
        idx.sort_unstable();
        idx.dedup();
        if let Some(lane) = self.lane_mut(ti) {
            for i in idx.into_iter().rev() {
                if i < lane.len() {
                    lane.remove(i);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.advance();
        self.next_in_playlist(&ctx);
        self.tick_follow(&ctx);
        self.tick_live(&ctx);
        if self.playing {
            self.lights.set_show(self.show()); // edits reach the lights straight away
            ctx.request_repaint_after(Duration::from_millis(16)); // ~60 fps preview, not the monitor's 200 Hz
        } else if self.scan_rx.is_some() || self.an_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Keyboard: space plays, Delete removes the selected clip.
        if !ctx.egui_wants_keyboard_input() && self.page != Page::Home {
            if ui.input(|i| i.key_pressed(egui::Key::Space)) {
                if self.page == Page::React {
                    if self.live_in.is_some() { self.stop_live() } else { self.start_live() }
                } else {
                    self.set_playing(!self.playing);
                }
            }
            if self.page == Page::Editor && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete_clip();
            }
            if ui.input(|i| i.key_pressed(egui::Key::B)) {
                self.black = !self.black;
            }
            if self.page == Page::Editor {
                use egui::{Key, Modifiers};
                let shift_z = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z));
                if shift_z || ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Y)) {
                    self.history(false);
                } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
                    self.history(true);
                }
                let (copy, paste) = ui.input(|i| (
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                    i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
                ));
                if copy {
                    self.clip_board = self.sel_clip().map(|c| c.clone());
                }
                if let Some(c) = self.clip_board.clone().filter(|_| paste && self.ti < self.rows()) {
                    let (t, ti) = (self.t, self.ti);
                    self.add_preset(ti, c, t);
                }
                let group = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::G));
                if group {
                    self.ungroup();
                } else if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::G)) {
                    self.group();
                }
                if ui.input(|i| i.key_pressed(Key::S) && i.modifiers.is_none()) {
                    self.split();
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.leave_compound();
                }
            }
        }
        self.flash = self.page != Page::Home && !ctx.egui_wants_keyboard_input() && ui.input(|i| i.key_down(egui::Key::F));
        // A music file dropped on the window.
        if self.page == Page::Editor {
            let dropped = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).next());
            if let Some(p) = dropped.filter(|p| p.extension().is_some_and(|e| AUDIO_EXT.contains(&e.to_string_lossy().to_lowercase().as_str()))) {
                self.import_audio(&ctx, p.to_string_lossy().into_owned());
            }
        }

        self.top_bar(ui);
        match self.page {
            Page::Home => self.home(ui),
            Page::Editor => self.editor(ui),
            Page::Live => self.live(ui),
            Page::React => self.react(ui),
        }
        // Undo history: one step per finished edit (not per frame of a drag).
        if !ctx.input(|i| i.pointer.any_down()) && self.scenes != self.committed {
            self.undo.push(std::mem::replace(&mut self.committed, self.scenes.clone()));
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.save(false);
    }


    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save(true);
        self.lights.stop();
        for ip in &self.blink {
            let _ = govee::identify(ip, false); // never leave a light switched off
        }
    }
}

// ---------- pages ----------

impl App {
    fn top_bar(&mut self, ui: &mut Ui) {
        egui::Panel::top("top").frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(16, 10))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::hover());
                ui.painter().rect_filled(r, 8, PANEL2);
                for (i, c) in [[255, 107, 139], [255, 194, 75], [47, 216, 180], [124, 108, 255]].iter().enumerate() {
                    let x = r.left() + 4.0 + i as f32 * 4.6;
                    ui.painter().rect_filled(Rect::from_min_size(Pos2::new(x, r.top() + 9.0), Vec2::new(3.4, 8.0)), 2, Color32::from_rgb(c[0], c[1], c[2]));
                }
                ui.label(RichText::new("Lightline").strong().size(17.0));
                ui.add_space(16.0);
                Frame::new().fill(PANEL).corner_radius(255).inner_margin(4).show(ui, |ui| {
                    for (p, name) in [(Page::Home, "Home"), (Page::Editor, "Editor"), (Page::Live, "Live"), (Page::React, "React")] {
                        if pill(ui, name, self.page == p).clicked() {
                            self.page = p;
                        }
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(u) = self.update_rx.as_ref().and_then(|r| r.try_recv().ok()) {
                        self.update = Some(u);
                    }
                    if let Some((v, url)) = self.update.clone() {
                        if accent_button(ui, &format!("Update {v} available")).on_hover_text("Opens the download page").clicked() {
                            use std::os::windows::process::CommandExt;
                            let _ = std::process::Command::new("explorer.exe").arg(url).creation_flags(0x0800_0000).spawn();
                        }
                    }
                    let n = self.devices.len();
                    let text = if self.status.is_empty() { format!("{n} light{} found", if n == 1 { "" } else { "s" }) } else { self.status.clone() };
                    ui.label(RichText::new(text).color(if self.status.is_empty() { MUTED } else { WARN }));
                });
            });
        });
    }

    fn home(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(32, 16))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                Frame::new().fill(PANEL).corner_radius(20).inner_margin(28).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Your lights").size(28.0).strong());
                            ui.label(RichText::new("Build scenes on a timeline, sync them to music, play them live.").color(MUTED));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let busy = self.scan_rx.is_some();
                            if ui.add_enabled(!busy, egui::Button::new(if busy { "Scanning…" } else { "Scan network" })).clicked() {
                                self.scan(&ctx);
                            }
                        });
                    });
                });
                ui.add_space(16.0);
                ui.label(RichText::new("Devices").color(MUTED).strong());
                if self.devices.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Chain order").color(MUTED)).on_hover_text("The order a track set to \"All lights, chained\" runs across your lights");
                        let order: Vec<String> = self.chain_devices().iter().map(|d| d.id.clone()).collect();
                        for (k, id) in order.iter().enumerate() {
                            let name = self.devices.iter().find(|d| &d.id == id).map_or(id.clone(), |d| self.dev_name(d));
                            let mut moved = None;
                            if k > 0 && ui.small_button("←").clicked() {
                                moved = Some(k - 1);
                            }
                            ui.label(RichText::new(name).strong());
                            if k + 1 < order.len() && ui.small_button("→").clicked() {
                                moved = Some(k + 1);
                            }
                            if let Some(j) = moved {
                                let mut o = order.clone();
                                o.swap(k, j);
                                self.settings.chain = o;
                                self.settings_dirty = true;
                            }
                            if k + 1 < order.len() {
                                ui.label(RichText::new("·").color(MUTED));
                            }
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    if self.devices.is_empty() {
                        card(ui, |ui| {
                            ui.label(RichText::new("No lights found. Turn on LAN Control for each light in the Govee Home app, then scan again.").color(MUTED));
                        });
                    }
                    for i in 0..self.devices.len() {
                        card(ui, |ui| self.device_card(ui, i));
                    }
                });
                ui.add_space(16.0);
                ui.label(RichText::new("Scenes").color(MUTED).strong());
                // Cards in rows that fit the window (egui doesn't wrap frames on its own).
                let gap = ui.spacing().item_spacing.x;
                let per_row = (((ui.available_width() + gap) / (290.0 + gap)) as usize).max(1); // card + Copy/Delete buttons
                let total = self.scenes.len() + 1;
                for row in 0..total.div_ceil(per_row) {
                ui.horizontal(|ui| {
                for item in row * per_row..((row + 1) * per_row).min(total) {
                    if item == 0 {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(250.0, 180.0), Sense::click());
                    let hov = resp.hovered();
                    ui.painter().rect_stroke(r.shrink(1.0), 14, Stroke::new(2.0, if hov { ACCENT } else { PANEL3 }), StrokeKind::Inside);
                    ui.painter().circle_filled(r.center() - Vec2::new(0.0, 14.0), 22.0, ACCENT);
                    ui.painter().text(r.center() - Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(26.0), ON_ACCENT);
                    ui.painter().text(r.center() + Vec2::new(0.0, 28.0), egui::Align2::CENTER_CENTER, "New scene", egui::FontId::proportional(15.0), if hov { TEXT } else { MUTED });
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        let tracks: Vec<Track> = if self.devices.is_empty() {
                            vec![Track { name: "Track 1".into(), segs: 15, from: 0, dev: None, muted: false, locked: false, clips: vec![] }]
                        } else {
                            self.devices.iter().map(|d| Track { name: self.dev_name(d), segs: self.light_segs(d), from: 0, dev: Some(d.id.clone()), muted: false, locked: false, clips: vec![] }).collect()
                        };
                        let n = self.scenes.len() + 1;
                        self.scenes.push(Scene { name: format!("Scene {n}"), len: 60.0, bpm: 120.0, audio: None, tracks });
                        self.dirty = true;
                        self.open(&ctx, n - 1, Page::Editor);
                    }
                    } else {
                        let i = item - 1;
                        card(ui, |ui| {
                            let s = &self.scenes[i];
                            let cols = s.tracks.first().map(|tr| fx::track_colors(tr, s.len / 3.0, &Ctx { an: None, bpm: s.bpm })).unwrap_or_default();
                            let (r, _) = ui.allocate_exact_size(Vec2::new(226.0, 70.0), Sense::hover());
                            ui.painter().rect_filled(r, 10, BG);
                            segments(ui, r.shrink(10.0), &cols, 1.0, 4.0);
                            let info = format!("{} tracks · {}", s.tracks.len(), fmt(s.len));
                            let mut name = s.name.clone();
                            if ui.add(egui::TextEdit::singleline(&mut name).font(egui::TextStyle::Body).frame(Frame::NONE).desired_width(220.0)).changed() {
                                self.scenes[i].name = name;
                                self.dirty = true;
                            }
                            ui.label(RichText::new(info).color(MUTED).small());
                            ui.horizontal(|ui| {
                                if ui.button("Edit").clicked() {
                                    self.open(&ctx, i, Page::Editor);
                                }
                                if icon_button(ui, Icon::Play, 34.0, true).on_hover_text("Play").clicked() {
                                    self.open(&ctx, i, Page::Live);
                                    self.play_when_loaded();
                                }
                                if ui.button("Copy").on_hover_text("Duplicate this scene").clicked() {
                                    let mut c = self.scenes[i].clone();
                                    c.name = format!("{} copy", c.name);
                                    self.scenes.insert(i + 1, c);
                                    self.dirty = true;
                                }
                                let sure = self.confirm_del == Some(i);
                                let del = ui.add(egui::Button::new(RichText::new(if sure { "Sure?" } else { "Delete" }).color(if sure { WARN } else { MUTED })));
                                if del.clicked() {
                                    if sure {
                                        self.delete_scene(&ctx, i);
                                    } else {
                                        self.confirm_del = Some(i);
                                    }
                                }
                            });
                        });
                    }
                }
                });
                }
            });
        });
    }

    fn delete_scene(&mut self, ctx: &egui::Context, i: usize) {
        self.confirm_del = None;
        self.set_playing(false);
        self.scenes.remove(i);
        if self.scenes.is_empty() {
            self.scenes.push(scene::demo());
        }
        let cur = if self.cur > i { self.cur - 1 } else { self.cur.min(self.scenes.len() - 1) };
        self.dirty = true;
        self.open(ctx, cur, Page::Home);
    }

    /// Start playing now, or once the scene's song has loaded.
    fn play_when_loaded(&mut self) {
        if self.an_rx.is_none() {
            self.set_playing(true);
        } else {
            self.autoplay = true;
        }
    }

    fn device_card(&mut self, ui: &mut Ui, i: usize) {
        let (id, ip, sku) = (self.devices[i].id.clone(), self.devices[i].ip.clone(), self.devices[i].sku.clone());
        let mut name = self.dev_name(&self.devices[i]);
        if ui.add(egui::TextEdit::singleline(&mut name).font(egui::TextStyle::Heading).frame(Frame::NONE).desired_width(220.0)).changed() {
            if name.trim().is_empty() {
                self.settings.names.remove(&id);
            } else {
                self.settings.names.insert(id.clone(), name.trim().into());
            }
            self.settings_dirty = true;
        }
        ui.horizontal(|ui| {
            ui.painter().circle_filled(ui.cursor().min + Vec2::new(4.0, 9.0), 4.0, ACCENT);
            ui.add_space(12.0);
            ui.label(RichText::new(format!("{sku} · {ip}")).color(MUTED).small());
        });
        ui.horizontal(|ui| {
            let mut n = self.light_segs(&self.devices[i]);
            ui.label(RichText::new("Max segments").color(MUTED));
            if ui.add(egui::DragValue::new(&mut n).range(1..=84)).on_hover_text("The most colours this light accepts per frame. Too high and it goes dark during scenes.").changed() {
                self.settings.lsegs.insert(id.clone(), n);
                self.settings_dirty = true;
            }
        });
        ui.horizontal(|ui| {
            let mut ms = self.settings.loff_ms.get(&id).copied().unwrap_or(0.0);
            ui.label(RichText::new("Timing").color(MUTED));
            if ui.add(egui::DragValue::new(&mut ms).range(-500.0..=500.0).speed(1.0).suffix(" ms")).on_hover_text("Negative = this light gets its colours earlier. Use it to line a slow light up with the others.").changed() {
                self.settings.loff_ms.insert(id.clone(), ms);
                self.settings_dirty = true;
            }
        });
        ui.horizontal(|ui| {
            let mut sm = self.light_smooth(&id);
            ui.label(RichText::new("Smoothing").color(MUTED));
            if ui.add(egui::DragValue::new(&mut sm).range(0.0..=100.0).speed(0.5)).on_hover_text("How softly colours fade out on this light. Lower = snappier. Until you change it, it follows the overall smoothing.").changed() {
                self.settings.lsmooth.insert(id.clone(), sm);
                self.settings_dirty = true;
            }
        });
        let on = self.blink.contains(&ip);
        let b = if on { accent_button(ui, "Stop blinking") } else { ui.button("Identify (blink)") };
        if b.clicked() {
            if on { self.blink.remove(&ip); } else { self.blink.insert(ip.clone()); }
            if let Err(e) = govee::identify(&ip, !on) {
                self.status = e;
            }
        }
    }

    fn editor(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let bar = Frame::new().fill(PANEL).inner_margin(Margin::symmetric(14, 6));
        egui::Panel::top("tools").frame(bar).show(ui, |ui| ui.horizontal(|ui| self.toolbar(ui, &ctx)));
        let h = ui.available_height();
        egui::Panel::bottom("timeline").resizable(true).default_size(h * 0.46).min_size(200.0)
            .frame(Frame::new().fill(PANEL).inner_margin(Margin::symmetric(0, 6)))
            .show(ui, |ui| self.timeline(ui));
        // Top: library | preview | clip settings, as separate cards (fixed rects, so tiles wrap).
        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(8)).show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            let (gap, insp_w) = (8.0, 330.0);
            let lib_w = ((full.width() - insp_w - 2.0 * gap) * 0.5).max(300.0);
            let lib = Rect::from_min_size(full.min, Vec2::new(lib_w, full.height()));
            let insp = Rect::from_min_max(Pos2::new(full.right() - insp_w, full.top()), full.max);
            let prev = Rect::from_min_max(Pos2::new(lib.right() + gap, full.top()), Pos2::new(insp.left() - gap, full.bottom()));
            card_at(ui, lib, |ui| {
                self.tab_strip(ui);
                ui.separator();
                egui::ScrollArea::vertical().id_salt("libs").auto_shrink(false).show(ui, |ui| match self.tab {
                    Tab::Song => self.song_tab(ui),
                    Tab::Patterns => self.fx_tiles(ui, false),
                    Tab::Reactive => self.fx_tiles(ui, true),
                    Tab::Show => self.show_tab(ui),
                    Tab::Colours => self.colours_tab(ui),
                    Tab::Presets => self.presets_tab(ui),
                    Tab::Lights => self.lights_tab(ui),
                });
            });
            card_at(ui, prev, |ui| {
                let h = (ui.available_height() - 50.0).max(60.0);
                Frame::new().fill(STAGE_BG).corner_radius(8).inner_margin(18).show(ui, |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), h - 36.0));
                    let big = self.sc().tracks.len() <= 4;
                    self.stage(ui, big, h - 36.0);
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| self.transport(ui));
            });
            card_at(ui, insp, |ui| {
                egui::ScrollArea::vertical().id_salt("insp").auto_shrink(false).show(ui, |ui| {
                    self.inspector(ui);
                    ui.separator();
                    self.track_panel(ui);
                    ui.separator();
                    self.output_panel(ui);
                });
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, _ctx: &egui::Context) {
        let mut name = self.sc().name.clone();
        if ui.add(egui::TextEdit::singleline(&mut name).font(egui::TextStyle::Heading).desired_width(260.0)).changed() {
            self.sc_mut().name = name;
        }
        ui.label(RichText::new("Length").color(MUTED));
        let mut len = self.sc().len;
        if ui.add(egui::DragValue::new(&mut len).range(5.0..=7200.0).suffix(" s")).changed() {
            self.sc_mut().len = len;
        }
        if self.an.is_none() {
            ui.label(RichText::new("BPM").color(MUTED));
            let mut bpm = self.sc().bpm;
            if ui.add(egui::DragValue::new(&mut bpm).range(30.0..=300.0)).changed() {
                self.sc_mut().bpm = bpm;
            }
        }
        if !self.loading.is_empty() {
            ui.label(RichText::new(&self.loading).color(MUTED));
        } else if let Some(an) = &self.an {
            let pulse = 1.0 - self.cx().beat_pos(self.t).rem_euclid(1.0) as f32;
            ui.label(RichText::new("●").color(ACCENT.gamma_multiply(0.25 + 0.75 * pulse)));
            ui.label(RichText::new(format!("{} BPM", an.bpm)).color(ACCENT));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Go live").clicked() {
                self.page = Page::Live;
                self.set_playing(true);
            }
        });
    }

    /// Under the preview: back to start, play, time, scrub bar.
    fn transport(&mut self, ui: &mut Ui) {
        if icon_button(ui, Icon::Prev, 34.0, false).on_hover_text("Back to start").clicked() {
            self.seek(0.0);
        }
        if icon_button(ui, if self.playing { Icon::Pause } else { Icon::Play }, 38.0, true).on_hover_text("Play / pause (Space)").clicked() {
            self.set_playing(!self.playing);
        }
        let len = self.sc().len;
        ui.label(RichText::new(format!("{} / {}", fmt(self.t), fmt(len))).monospace());
        let mut t = self.t;
        ui.spacing_mut().slider_width = (ui.available_width() - 16.0).max(60.0);
        if ui.add(egui::Slider::new(&mut t, 0.0..=len.max(0.1)).show_value(false)).changed() {
            self.seek(t);
        }
    }

    /// Icon tabs along the top of the library.
    fn tab_strip(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, icon, name) in TABS {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(62.0, 50.0), Sense::click());
                let on = self.tab == tab;
                let col = if on { ACCENT } else if resp.hovered() { TEXT } else { MUTED };
                let p = ui.painter();
                if on {
                    p.rect_filled(Rect::from_min_size(Pos2::new(r.left() + 12.0, r.bottom() - 3.0), Vec2::new(r.width() - 24.0, 3.0)), 2, ACCENT);
                }
                p.text(Pos2::new(r.center().x, r.top() + 16.0), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(20.0), col);
                p.text(Pos2::new(r.center().x, r.bottom() - 13.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(12.5), col);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.tab = tab;
                }
            }
        });
    }

    /// Patterns or music effects, as tiles with a moving preview. Drag onto a track or click.
    fn fx_tiles(&mut self, ui: &mut Ui, reactive: bool) {
        ui.label(RichText::new(if reactive { "Follow the music: kicks, vocals, bass, drops..." } else { "Patterns timed to the beat" }).color(MUTED).small());
        ui.add_space(4.0);
        let cx = Ctx { an: None, bpm: 120.0 };
        let t = self.epoch.elapsed().as_secs_f64();
        ui.ctx().request_repaint_after(Duration::from_millis(50));
        let mut add = None;
        let list: Vec<&'static str> = EFFECTS.iter().copied().filter(|f| fx::REACTIVE.contains(f) == reactive).collect();
        for row in list.chunks(per_row(ui)) {
            ui.horizontal(|ui| {
                for &fxn in row {
                    let (col, col2) = fx::default_colors(fxn).unwrap_or(([0x2f, 0xd8, 0xb4], [0x7c, 0x6c, 0xff]));
                    let c = Clip { fx: fxn.to_string(), start: 0.0, len: 1e9, col, col2, ..Default::default() };
                    if tile(ui, ("fx", fxn), Drop::Fx(fxn), fxn, &c, t, &cx).clicked() {
                        add = Some(fxn);
                    }
                }
            });
        }
        if let Some(f) = add {
            self.add_clip(self.ti, f, self.t);
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Drag onto a track, or click to add at the playhead on the selected track.").color(MUTED).small());
    }

    fn presets_tab(&mut self, ui: &mut Ui) {
        let can = self.sel.is_some();
        if ui.add_enabled(can, egui::Button::new("⭐ Save selected clip as preset")).clicked() {
            if let Some(mut c) = self.sel_clip().cloned() {
                c.shift(-c.start);
                self.settings.presets.push(c);
                self.settings_dirty = true;
            }
        }
        if self.settings.presets.is_empty() {
            ui.label(RichText::new("No presets yet. Set up a clip how you like it (effect, colours, speed...) and save it here. Compound clips work too.").color(MUTED).small());
            return;
        }
        ui.add_space(4.0);
        let cx = Ctx { an: None, bpm: 120.0 };
        let t = self.epoch.elapsed().as_secs_f64();
        ui.ctx().request_repaint_after(Duration::from_millis(50));
        let (mut add, mut del) = (None, None);
        let per = per_row(ui);
        for (r0, chunk) in self.settings.presets.chunks(per).enumerate() {
          ui.horizontal(|ui| {
            for (k, c) in chunk.iter().enumerate() {
                let i = r0 * per + k;
                let name = if c.label.is_empty() { c.fx.clone() } else { c.label.clone() };
                let demo = Clip { start: 0.0, len: 1e9, ..c.clone() };
                let r = tile(ui, ("preset", i), Drop::Preset(c.clone()), &name, &demo, t % c.len.max(0.5), &cx);
                if r.clicked() {
                    add = Some(c.clone());
                }
                r.context_menu(|ui| {
                    if ui.button("Delete preset").clicked() {
                        del = Some(i);
                    }
                });
            }
          });
        }
        if let Some(c) = add {
            self.add_preset(self.ti, c, self.t);
        }
        if let Some(i) = del {
            self.settings.presets.remove(i);
            self.settings_dirty = true;
        }
        ui.label(RichText::new("Drag onto a track or click. Right-click to delete.").color(MUTED).small());
    }

    fn colours_tab(&mut self, ui: &mut Ui) {
        let mut apply: Option<Vec<[u8; 3]>> = None;
        ui.label(RichText::new("Your colour sets").strong());
        ui.label(RichText::new("Use them in Show, or Apply one to the selected clip.").color(MUTED).small());
        let (mut del, mut changed) = (None, false);
        for (i, (name, cols)) in self.settings.colours.iter_mut().enumerate() {
            Frame::new().fill(PANEL2).corner_radius(10).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::TextEdit::singleline(name).desired_width(150.0)).changed();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Delete this set").clicked() {
                            del = Some(i);
                        }
                        if ui.small_button("Apply").clicked() {
                            apply = Some(cols.clone());
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    for c in cols.iter_mut() {
                        changed |= ui.color_edit_button_srgb(c).changed();
                    }
                    if cols.len() < 8 && ui.small_button("+").on_hover_text("Add a colour").clicked() {
                        cols.push(*cols.last().unwrap_or(&[255, 255, 255]));
                        changed = true;
                    }
                    if cols.len() > 1 && ui.small_button("−").on_hover_text("Remove the last colour").clicked() {
                        cols.pop();
                        changed = true;
                    }
                });
            });
        }
        if let Some(i) = del {
            self.settings.colours.remove(i);
            changed = true;
        }
        if ui.button("+ New colour set").clicked() {
            let n = self.settings.colours.len() + 1;
            self.settings.colours.push((format!("My colours {n}"), vec![[0xff, 0x2a, 0x6d], [0xff, 0x9f, 0x1c], [0x2e, 0xc4, 0xb6], [0x7c, 0x6c, 0xff]]));
            changed = true;
        }
        self.settings_dirty |= changed;
        ui.add_space(12.0);
        ui.label(RichText::new("Built in").strong());
        for (name, cols) in show::PALETTES.iter() {
            ui.horizontal(|ui| {
                swatches(ui, cols);
                ui.label(*name);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Apply").clicked() {
                        apply = Some(cols.to_vec());
                    }
                });
            });
        }
        if let Some(cols) = apply {
            match self.sel_clip() {
                Some(c) => {
                    (c.col, c.col2) = (cols[0], cols[1 % cols.len()]);
                    c.pal = cols;
                }
                None => self.status = "Select a clip first, then Apply.".into(),
            }
        }
    }

    fn lights_tab(&mut self, ui: &mut Ui) {
        ui.dnd_drag_source(ui.scope_id().with("chainsrc"), Drop::Light(CHAIN.to_string()), |ui| {
            Frame::new().fill(PANEL2).corner_radius(10).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").color(ACCENT2));
                    ui.label(RichText::new("All lights, chained").strong());
                });
            });
        }).response.on_hover_text("One long strip: the effect runs across all lights in the chain order set on Home");
        if self.devices.is_empty() {
            ui.label(RichText::new("No lights found yet.").color(MUTED).small());
        }
        for i in 0..self.devices.len() {
            let (id, ip, name) = (self.devices[i].id.clone(), self.devices[i].ip.clone(), self.dev_name(&self.devices[i]));
            ui.dnd_drag_source(ui.scope_id().with(("light", &id)), Drop::Light(id.clone()), |ui| {
                Frame::new().fill(PANEL2).corner_radius(10).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(ACCENT));
                        ui.label(RichText::new(name).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(ip).color(MUTED).small()));
                    });
                });
            });
        }
        ui.label(RichText::new("Drag a light onto a track to give that track the light.").color(MUTED).small());
    }

    /// The song: load it, what's in it, and quick actions built on that.
    fn song_tab(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let name = self.sc().audio.as_ref().map_or("No song yet".to_string(), |a| a.name.clone());
        ui.label(RichText::new(name).size(17.0).strong());
        if ui.button(if self.sc().audio.is_some() { "Replace song…" } else { "+ Import music…" }).clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("Music", &AUDIO_EXT).pick_file() {
                self.import_audio(&ctx, p.to_string_lossy().into_owned());
            }
        }
        if !self.loading.is_empty() {
            ui.label(RichText::new(&self.loading).color(MUTED));
        }
        let Some(an) = self.an.clone() else {
            ui.label(RichText::new("Or drop a music file on the window.").color(MUTED).small());
            return;
        };
        let len = self.sc().len;
        let (lines, drops) = insights(&an, len);
        ui.add_space(8.0);
        ui.label(RichText::new("What's in it").color(MUTED).strong());
        for l in &lines {
            ui.label(format!("• {l}"));
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Parts (click to jump)").color(MUTED).strong());
        let mut count: Vec<&str> = vec![];
        let mut jump = None;
        for (k, &(t0, part)) in an.sections.iter().enumerate() {
            let t1 = an.sections.get(k + 1).map_or(len, |s| s.0);
            count.push(part.name());
            let n = count.iter().filter(|x| **x == part.name()).count();
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(6.0, 18.0), Sense::hover());
                ui.painter().rect_filled(r, 2, part_colour(part));
                if ui.add(egui::Button::new(format!("{} {n}", part.name())).frame(false)).clicked() {
                    jump = Some(t0);
                }
                ui.label(RichText::new(format!("{} – {}", fmt(t0), fmt(t1))).color(MUTED).small());
            });
        }
        if let Some(t) = jump {
            self.seek(t);
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Quick actions").color(MUTED).strong());
        let ti = self.ti;
        if ui.add_enabled(!drops.is_empty(), egui::Button::new(format!("Flash on every drop ({})", drops.len()))).on_hover_text("White flashes on the selected track").clicked() {
            if let Some(lane) = self.lane_mut(ti) {
                for &d in &drops {
                    lane.push(Clip { fx: "Solid".into(), start: d, len: 0.6, col: [255; 3], col2: [0; 3], fout: 0.6, ..Default::default() });
                }
            }
        }
        let sel = self.sel.and_then(|(t, c)| self.lane(t).get(c).cloned());
        if ui.add_enabled(sel.is_some(), egui::Button::new("Copy selected clip to every chorus")).clicked() {
            let ch: Vec<(f64, f64)> = an.sections.iter().enumerate().filter(|(_, s)| s.1 == Part::Chorus).map(|(k, s)| (s.0, an.sections.get(k + 1).map_or(len, |x| x.0))).collect();
            if let (Some(c), Some(lane)) = (sel, self.lane_mut(ti)) {
                for (a, b) in ch {
                    let mut k = c.clone();
                    k.shift(a - k.start);
                    if k.fx != COMPOUND {
                        k.len = b - a;
                    }
                    lane.push(k);
                }
            }
        }
    }

    fn inspector(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Clip").color(MUTED).strong());
        let Some((ti, ci)) = self.sel else {
            ui.label(RichText::new("Select a clip on the timeline. Ctrl+click picks several.").color(MUTED));
            return;
        };
        let track_name = self.sc().tracks.get(self.real_track(ti)).map(|t| t.name.clone()).unwrap_or_default();
        let len_total = self.sc().len;
        let inside = self.inside.is_some();
        let n_sel = self.multi.len() + 1;
        let Some(c) = self.sel_clip() else { return };
        let mut changed = false;
        if n_sel > 1 {
            ui.label(RichText::new(format!("{n_sel} clips selected. Ctrl+G makes them one compound clip.")).color(ACCENT));
        }
        if c.fx == COMPOUND {
            ui.label(RichText::new("Compound clip").size(18.0).strong());
            ui.label(RichText::new(format!("{} clips inside · {track_name}", c.inner.len())).color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut c.label).hint_text("Name"));
            ui.add(egui::Slider::new(&mut c.bri, 0.0..=100.0).text("Brightness").suffix("%"));
            let mut st = c.start;
            ui.horizontal(|ui| {
                ui.label("Start");
                if ui.add(egui::DragValue::new(&mut st).range(0.0..=(len_total - c.len).max(0.0)).speed(0.05).suffix(" s")).changed() {
                    c.shift(st - c.start);
                }
            });
            let (open, ung, del) = ui.horizontal(|ui| (
                ui.add_enabled(!inside, egui::Button::new("Open")).on_hover_text("Edit what's inside (or double-click it)").clicked(),
                ui.button("Ungroup").clicked(),
                ui.button("Delete").clicked(),
            )).inner;
            if open {
                self.enter_compound(ti, ci);
                self.multi.clear();
            }
            if ung {
                self.ungroup();
            }
            if del {
                self.delete_clip();
            }
            return;
        }
        ui.label(RichText::new(&c.fx).size(18.0).strong());
        ui.label(RichText::new(track_name).color(MUTED));
        egui::ComboBox::from_label("Effect").selected_text(c.fx.clone()).show_ui(ui, |ui| {
            for f in EFFECTS {
                changed |= ui.selectable_value(&mut c.fx, f.to_string(), f).changed();
            }
        });
        if fx::PALETTE_FX.contains(&c.fx.as_str()) {
            ui.horizontal_wrapped(|ui| {
                ui.label("Colours");
                if c.pal.is_empty() {
                    ui.label(RichText::new("Rainbow").color(ACCENT));
                    if ui.button("Custom").on_hover_text("Pick your own colours, blended along the strip").clicked() {
                        (c.pal, changed) = (vec![c.col, c.col2], true);
                    }
                } else {
                    for col in c.pal.iter_mut() {
                        changed |= ui.color_edit_button_srgb(col).changed();
                    }
                    if c.pal.len() < 12 && ui.button("+").on_hover_text("Add a colour").clicked() {
                        c.pal.push(*c.pal.last().unwrap());
                        changed = true;
                    }
                    if ui.button("−").on_hover_text("Remove the last colour").clicked() {
                        c.pal.pop();
                        changed = true;
                    }
                    if ui.button("Rainbow").clicked() {
                        (c.pal, changed) = (vec![], true);
                    }
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label("Colour 1");
                changed |= ui.color_edit_button_srgb(&mut c.col).changed();
                ui.label("Colour 2");
                changed |= ui.color_edit_button_srgb(&mut c.col2).changed();
            });
        }
        changed |= ui.add(egui::Slider::new(&mut c.bri, 0.0..=100.0).text("Brightness").suffix("%")).changed();
        egui::ComboBox::from_label("Speed").selected_text(format!("{}×", c.speed)).show_ui(ui, |ui| {
            for s in [0.25, 0.5, 1.0, 2.0, 4.0] {
                changed |= ui.selectable_value(&mut c.speed, s, format!("{s}×")).changed();
            }
        });
        changed |= ui.checkbox(&mut c.rev, "Reverse direction").changed();
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut c.mirror, "Mirror").on_hover_text("Mirror the effect out from the middle").changed();
            ui.label("Repeat");
            for r in [1u8, 2, 4] {
                changed |= ui.selectable_value(&mut c.repeat, r, format!("{r}×")).changed();
            }
        });
        egui::ComboBox::from_label("Reacts to").selected_text(scene::REACT[c.react as usize % 5]).show_ui(ui, |ui| {
            for (i, n) in scene::REACT.iter().enumerate() {
                changed |= ui.selectable_value(&mut c.react, i as u8, *n).changed();
            }
        }).response.on_hover_text("Brightness follows this part of the music");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("trans").selected_text(scene::TRANS[c.trans as usize % 3]).show_ui(ui, |ui| {
                for (i, n) in scene::TRANS.iter().enumerate() {
                    changed |= ui.selectable_value(&mut c.trans, i as u8, *n).changed();
                }
            });
            if c.trans > 0 {
                changed |= ui.add(egui::DragValue::new(&mut c.tlen).range(0.05..=4.0).speed(0.02).suffix(" s")).changed();
            }
            ui.label("In from the clip before");
        });
        egui::Grid::new("clipnums").num_columns(2).show(ui, |ui| {
            let max_len = (len_total - c.start).max(0.1);
            ui.label("Start");
            changed |= ui.add(egui::DragValue::new(&mut c.start).range(0.0..=(len_total - c.len).max(0.0)).speed(0.05).suffix(" s")).changed();
            ui.end_row();
            ui.label("Length");
            changed |= ui.add(egui::DragValue::new(&mut c.len).range(0.1..=max_len).speed(0.05).suffix(" s")).changed();
            ui.end_row();
            ui.label("Fade in");
            changed |= ui.add(egui::DragValue::new(&mut c.fin).range(0.0..=c.len).speed(0.05).suffix(" s")).changed();
            ui.end_row();
            ui.label("Fade out");
            changed |= ui.add(egui::DragValue::new(&mut c.fout).range(0.0..=c.len).speed(0.05).suffix(" s")).changed();
            ui.end_row();
        });
        let dup = c.clone();
        ui.horizontal(|ui| {
            if ui.button("Duplicate").clicked() {
                let at = dup.start + dup.len;
                self.add_preset(ti, dup.clone(), at);
                changed = true;
            }
            if ui.button("Delete").clicked() {
                self.delete_clip();
            }
        });
        self.dirty |= changed;
    }

    fn track_panel(&mut self, ui: &mut Ui) {
        let ti = self.real_track(self.ti);
        let Some(tr) = self.sc().tracks.get(ti).cloned() else { return };
        ui.label(RichText::new(format!("Track · {}", tr.name)).color(MUTED).strong());
        let mut dev = tr.dev.clone();
        let current = self.track_light(&tr);
        egui::ComboBox::from_label("Light").selected_text(current).show_ui(ui, |ui| {
            ui.selectable_value(&mut dev, None, "None (preview only)");
            ui.selectable_value(&mut dev, Some(CHAIN.to_string()), "All lights, chained");
            for d in &self.devices {
                ui.selectable_value(&mut dev, Some(d.id.clone()), format!("{} · {}", self.dev_name(d), d.ip));
            }
        });
        let (mut from, mut segs) = (tr.from + 1, tr.segs);
        ui.horizontal(|ui| {
            ui.label("Starts at segment");
            ui.add(egui::DragValue::new(&mut from).range(1..=84));
            ui.label("Segments");
            ui.add(egui::DragValue::new(&mut segs).range(1..=84));
        });
        if let Some(id) = dev.as_ref().filter(|_| dev != tr.dev) {
            (from, segs) = (1, self.full_segs(id)); // a new light: use all of it
        }
        if dev != tr.dev || from != tr.from + 1 || segs != tr.segs {
            let t = &mut self.sc_mut().tracks[ti];
            (t.dev, t.from, t.segs) = (dev, from - 1, segs);
        }
        let tr = &self.sc().tracks[ti];
        if let Some(id) = &tr.dev {
            let end = |x: &Track| x.from + x.segs;
            let others: Vec<&Track> = self.sc().tracks.iter().filter(|x| !std::ptr::eq(*x, tr) && x.dev.as_ref() == Some(id)).collect();
            let shared: Vec<&str> = others.iter().filter(|x| x.from < end(tr) && tr.from < end(x)).map(|x| x.name.as_str()).collect();
            if !shared.is_empty() {
                ui.label(RichText::new(format!("Layered on the same segments with: {}. The higher track wins where both are lit. Change \"Starts at segment\" to split them into zones.", shared.join(", "))).color(WARN).small());
            }
            let total = others.iter().map(|x| end(x)).chain([end(tr)]).max().unwrap_or(0);
            if total > 84 {
                ui.label(RichText::new(format!("Tracks on this light reach segment {total}, but it takes at most 84. The rest is cut off.")).color(WARN).small());
            }
        }
        ui.label(RichText::new("Tracks on the same light layer on top of each other. Give them different start segments to make zones.").color(MUTED).small());
        if ui.add_enabled(self.inside.is_none(), egui::Button::new("Delete track")).clicked() {
            self.sc_mut().tracks.remove(ti);
            (self.ti, self.sel) = (ti.saturating_sub(1), None);
        }
    }

    fn output_panel(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Light output").color(MUTED).strong());
        let s = &mut self.settings;
        let a = ui.add(egui::Slider::new(&mut s.off_ms, -300.0..=300.0).step_by(5.0).text("Light timing").suffix(" ms")).on_hover_text("− makes the lights earlier, + later");
        let fade = format!("Smoothing ({} ms fade)", (s.smooth * 3.0).round());
        let b = ui.add(egui::Slider::new(&mut s.smooth, 0.0..=100.0).text(fade).show_value(false));
        ui.add(egui::Slider::new(&mut self.bri, 0.0..=100.0).text("Master brightness").suffix("%"));
        self.settings_dirty |= a.changed() || b.changed();
    }

    /// The preview: every track's segments at the playhead, smoothed like the lights.
    fn stage(&mut self, ui: &mut Ui, big: bool, max_h: f32) {
        let ms = self.epoch.elapsed().as_secs_f64() * 1000.0;
        let k = if self.black { 0.0 } else { (self.bri / 100.0) as f32 };
        let following = self.follow.as_ref().is_some_and(|f| f.synced.is_some());
        let rows: Vec<(String, Vec<Option<Rgb>>)> = if !following && (self.live_in.is_some() || self.page == Page::React) {
            let cx = Ctx { an: self.live_in.as_ref().map(|l| &*l.an), bpm: 120.0 };
            let t = self.live_in.as_ref().map_or(self.epoch.elapsed().as_secs_f64(), |l| l.t0.elapsed().as_secs_f64());
            self.live_tracks().iter().map(|tr| (tr.name.clone(), fx::track_colors(tr, t, &cx))).collect()
        } else {
            let cx = self.cx();
            self.sc().tracks.iter().map(|tr| (tr.name.clone(), fx::track_colors(tr, self.t, &cx))).collect()
        };
        if rows.is_empty() {
            ui.label(RichText::new("No tracks yet.").color(MUTED));
        }
        // Rows shrink so every track fits in the space given.
        let per = max_h / rows.len().max(1) as f32;
        let (h, gap) = if big { ((per - 44.0).clamp(8.0, 40.0), 18.0) } else { ((per - 16.0).clamp(8.0, 22.0), 14.0) };
        let used = rows.len() as f32 * (h + gap + if big { 26.0 } else { 0.0 });
        ui.add_space(((max_h - used) / 2.0).max(0.0));
        for (i, (name, cols)) in rows.iter().enumerate() {
            let cols = if self.playing { self.pv.step(&i.to_string(), cols, ms, self.settings.smooth) } else { cols.clone() };
            if big {
                ui.label(RichText::new(name).color(MUTED));
                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
                segments(ui, r, &cols, k, 5.0);
            } else {
                ui.horizontal(|ui| {
                    ui.add_sized([150.0, h], egui::Label::new(RichText::new(name).color(MUTED)).truncate());
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
                    segments(ui, r, &cols, k, 4.0);
                });
            }
            ui.add_space(gap);
        }
    }

    fn live(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::right("livectl").default_size(340.0).frame(Frame::new().fill(BG).inner_margin(16)).show(ui, |ui| {
            card_wide(ui, |ui| {
                ui.label(RichText::new("Scenes").color(MUTED).strong());
                ui.horizontal(|ui| {
                    self.settings_dirty |= ui.checkbox(&mut self.settings.autonext, "Play next automatically").changed();
                    self.settings_dirty |= ui.checkbox(&mut self.settings.shuffle, "Shuffle").changed();
                });
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for i in 0..self.scenes.len() {
                        let label = format!("{}   {}", self.scenes[i].name, fmt(self.scenes[i].len));
                        if ui.add_sized([ui.available_width(), 36.0], egui::Button::selectable(i == self.cur, label)).clicked() {
                            self.open(&ctx, i, Page::Live);
                            self.play_when_loaded();
                        }
                    }
                });
            });
            ui.add_space(12.0);
            card_wide(ui, |ui| {
                ui.label(RichText::new("Controls").color(MUTED).strong());
                self.output_panel(ui);
                let fl = ui.add_sized([ui.available_width(), 40.0], egui::Button::new("Flash  (hold F)"));
                self.flash |= fl.is_pointer_button_down_on();
                let label = if self.black { "Lights back on  (B)" } else { "Blackout  (B)" };
                if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(label).selected(self.black)).clicked() {
                    self.black = !self.black;
                }
            });
        });
        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin { left: 24, right: 8, top: 12, bottom: 16 })).show(ui, |ui| {
            let pill_text = if self.playing { "● Live" } else { "● Paused" };
            ui.label(RichText::new(pill_text).color(ACCENT).strong());
            ui.label(RichText::new(&self.sc().name).size(40.0).strong());
            let audio = self.sc().audio.as_ref().map_or(String::new(), |a| format!(" · {}", a.name));
            let bpm = self.an.as_ref().map_or(self.sc().bpm, |a| a.bpm);
            ui.label(RichText::new(format!("{} tracks · {bpm} BPM{audio} {}", self.sc().tracks.len(), self.loading)).color(MUTED));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(fmt(self.t)).monospace().color(MUTED));
                let w = ui.available_width() - 60.0;
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 10.0), Sense::click_and_drag());
                ui.painter().rect_filled(r, 5, PANEL2);
                let f = (self.t / self.sc().len) as f32;
                ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * f, r.height())), 5, ACCENT);
                if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.clicked() || resp.dragged()) {
                    self.seek(((p.x - r.left()) / r.width()) as f64 * self.sc().len);
                }
                ui.label(RichText::new(fmt(self.sc().len)).monospace().color(MUTED));
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 200.0) / 2.0);
                let n = self.scenes.len();
                if icon_button(ui, Icon::Prev, 52.0, false).on_hover_text("Previous scene").clicked() {
                    self.open(&ctx, (self.cur + n - 1) % n, Page::Live);
                    self.play_when_loaded();
                }
                if icon_button(ui, if self.playing { Icon::Pause } else { Icon::Play }, 72.0, true).on_hover_text("Play / pause (Space)").clicked() {
                    self.set_playing(!self.playing);
                }
                if icon_button(ui, Icon::Next, 52.0, false).on_hover_text("Next scene").clicked() {
                    self.open(&ctx, (self.cur + 1) % n, Page::Live);
                    self.play_when_loaded();
                }
            });
            ui.add_space(12.0);
            let stage_h = (ui.available_height() - 16.0).max(120.0);
            Frame::new().fill(PANEL).corner_radius(14).inner_margin(28).show(ui, |ui| {
                ui.set_min_size(Vec2::new(ui.available_width(), stage_h));
                self.stage(ui, true, stage_h - 56.0);
            });
        });
    }
}

impl App {
    /// The React tab: the lights follow whatever this PC is playing.
    fn react(&mut self, ui: &mut Ui) {
        egui::Panel::right("reactctl").default_size(340.0).frame(Frame::new().fill(BG).inner_margin(16)).show(ui, |ui| {
            card_wide(ui, |ui| {
                ui.label(RichText::new("Media player").color(MUTED).strong());
                let on = self.follow.is_some();
                let b = ui.add_sized([ui.available_width(), 40.0], egui::Button::new(if on { "Stop following" } else { "Follow media player" }).selected(on));
                if b.on_hover_text("Spotify, a browser...: songs you made a scene for play that scene in sync, everything else gets the live look below").clicked() {
                    if on { self.stop_follow() } else { self.start_follow(ui.ctx()) }
                }
                if let Some(f) = &self.follow {
                    let text = match (&f.np, f.scene) {
                        (Some(n), Some(i)) if n.playing => format!("Now: {}\n→ playing scene \"{}\" in sync", n.title, self.scenes[i].name),
                        (Some(n), None) if n.playing => format!("Now: {}\n→ no scene for this song: reacting live", n.title),
                        (Some(n), _) => format!("Paused: {}", n.title),
                        (None, _) => "Nothing playing (or the player doesn't show in Windows' media controls)".into(),
                    };
                    ui.label(RichText::new(text).color(ACCENT).small());
                }
                if let Some(dir) = crate::media::riff_plugins_dir() {
                    let path = dir.join("lightline.js");
                    let installed = std::fs::read_to_string(&path).is_ok_and(|s| s == crate::media::RIFF_PLUGIN);
                    if installed {
                        ui.label(RichText::new("Riff plugin installed. Turn \"Lightline\" on in Riff's plugin settings.").color(MUTED).small());
                    } else if ui.button("Install Riff plugin").on_hover_text("Riff tells Windows the song but not where it is; this plugin tells Lightline directly").clicked() {
                        self.status = match std::fs::write(&path, crate::media::RIFF_PLUGIN) {
                            Ok(()) => String::new(),
                            Err(e) => format!("Couldn't install the Riff plugin: {e}"),
                        };
                    }
                }
                ui.add_space(10.0);
                ui.label(RichText::new("PC audio").color(MUTED).strong());
                let on = self.live_in.is_some();
                let b = ui.add_sized([ui.available_width(), 40.0], egui::Button::new(if on { "Stop listening to PC audio" } else { "React to PC audio" }).selected(on));
                if b.on_hover_text("The lights react to whatever this PC is playing: Spotify, YouTube, games").clicked() {
                    if on { self.stop_live() } else { self.start_live() }
                }
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("livefx").selected_text(LIVE_LOOKS[self.live_fx]).show_ui(ui, |ui| {
                        for (i, n) in LIVE_LOOKS.iter().enumerate() {
                            ui.selectable_value(&mut self.live_fx, i, *n);
                        }
                    });
                    let name = if self.live_pal == 0 { "Rainbow" } else { show::PALETTES[self.live_pal - 1].0 };
                    egui::ComboBox::from_id_salt("livepal").selected_text(name).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.live_pal, 0, "Rainbow");
                        for (i, (n, _)) in show::PALETTES.iter().enumerate() {
                            ui.selectable_value(&mut self.live_pal, i + 1, *n);
                        }
                    });
                });
                if let Some(l) = &self.live_in {
                    ui.label(RichText::new(format!("Listening · {} BPM", l.an.bpm)).color(ACCENT).small());
                }
            });
            ui.add_space(12.0);
            card_wide(ui, |ui| {
                ui.label(RichText::new("Controls").color(MUTED).strong());
                self.output_panel(ui);
                let fl = ui.add_sized([ui.available_width(), 40.0], egui::Button::new("Flash  (hold F)"));
                self.flash |= fl.is_pointer_button_down_on();
                let label = if self.black { "Lights back on  (B)" } else { "Blackout  (B)" };
                if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(label).selected(self.black)).clicked() {
                    self.black = !self.black;
                }
            });
        });
        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin { left: 24, right: 8, top: 12, bottom: 16 })).show(ui, |ui| {
            let on = self.live_in.is_some();
            let head = match &self.follow {
                Some(f) => match (&f.np, f.scene) {
                    (Some(n), Some(i)) if n.playing && f.synced.is_some() => format!("● Following {} · scene \"{}\"", n.app, self.scenes[i].name),
                    (Some(n), _) if n.playing => format!("● Following {} · reacting live", n.app),
                    _ => "● Following · waiting for a song".into(),
                },
                None if on => "● Listening".into(),
                None => "● Off".into(),
            };
            ui.label(RichText::new(head).color(if on || self.follow.is_some() { ACCENT } else { MUTED }).strong());
            ui.label(RichText::new("React to PC audio").size(40.0).strong());
            ui.label(RichText::new("The lights follow whatever this PC is playing: Spotify, YouTube, games. No scene needed.").color(MUTED));
            ui.add_space(14.0);
            let stage_h = (ui.available_height() - 16.0).max(120.0);
            Frame::new().fill(PANEL).corner_radius(14).inner_margin(28).show(ui, |ui| {
                ui.set_min_size(Vec2::new(ui.available_width(), stage_h));
                self.stage(ui, true, stage_h - 56.0);
            });
        });
    }
}

// ---------- timeline ----------

impl App {
    fn timeline(&mut self, ui: &mut Ui) {
        let sel_compound = self.sel.and_then(|(ti, ci)| self.lane(ti).get(ci)).is_some_and(|c| c.fx == COMPOUND);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            let has_sel = self.sel.is_some();
            if tool(ui, "↩", "Undo (Ctrl+Z)", !self.undo.is_empty()) {
                self.history(true);
            }
            if tool(ui, "↪", "Redo (Ctrl+Y)", !self.redo.is_empty()) {
                self.history(false);
            }
            if tool(ui, "🗑", "Delete (Del)", has_sel) {
                self.delete_clip();
            }
            if tool(ui, "✂", "Split at the playhead (S)", has_sel) {
                self.split();
            }
            ui.separator();
            if tool(ui, "▣ Group", "Make one compound clip from the selected clips (Ctrl+G). Ctrl+click clips to pick several.", has_sel) {
                self.group();
            }
            if tool(ui, "Ungroup", "Break the compound clip apart (Ctrl+Shift+G)", sel_compound) {
                self.ungroup();
            }
            ui.separator();
            if self.inside.is_none() && tool(ui, "+ Track", "Add a track", true) {
                let n = self.sc().tracks.len() + 1;
                self.sc_mut().tracks.push(Track { name: format!("Track {n}"), segs: 15, from: 0, dev: None, muted: false, locked: false, clips: vec![] });
            }
            ui.checkbox(&mut self.snap, "Snap to beat");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if ui.button("Fit").clicked() {
                    (self.zoom, self.scroll_to) = (1.0, Some(0.0));
                }
                if tool(ui, "⊕", "Zoom in (Ctrl + scroll)", true) {
                    self.zoom = (self.zoom * 1.5).min(64.0);
                }
                let mut lz = self.zoom.log2();
                if ui.add(egui::Slider::new(&mut lz, 0.0..=6.0).show_value(false)).changed() {
                    self.zoom = 2f32.powf(lz);
                }
                if tool(ui, "⊖", "Zoom out", true) {
                    self.zoom = (self.zoom / 1.5).max(1.0);
                }
            });
        });
        // Tabs: the main timeline, and the compound clip open inside it.
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            if ui.add(egui::Button::selectable(self.inside.is_none(), "Main timeline")).clicked() {
                self.leave_compound();
            }
            let open = self.inside.and_then(|(t, c)| self.sc().tracks.get(t)?.clips.get(c)).map(|c| c.label.clone());
            if let Some(label) = open {
                let _ = ui.add(egui::Button::selectable(true, format!("▣ {label}")));
                if ui.small_button("✕").on_hover_text("Back to the main timeline (Esc)").clicked() {
                    self.leave_compound();
                }
            } else {
                ui.label(RichText::new("Double-click a compound clip to open it").color(MUTED).small());
            }
        });
        ui.add_space(4.0);

        let len = self.sc().len.max(1.0);
        let base_w = (ui.available_width() - LABEL_W - 12.0).max(200.0);
        if let Some((s0, l)) = self.fit_to.take() {
            self.zoom = (len / (l * 1.15).max(0.5)).clamp(1.0, 64.0) as f32;
            self.scroll_to = Some(((s0 - l * 0.05) / len) as f32 * base_w * self.zoom);
        }
        let lane_w = base_w * self.zoom;
        let audio_h = if self.an.is_some() { 64.0 } else { 0.0 };
        let n_tracks = self.rows();
        let total_h = 28.0 + audio_h + n_tracks as f32 * ROW_H + 8.0;
        let mut sa = egui::ScrollArea::both().id_salt("tl").auto_shrink(false);
        if let Some(x) = self.scroll_to.take() {
            sa = sa.horizontal_scroll_offset(x.max(0.0));
        }
        let mut enter = None; // opened after drawing: the rows change once a compound clip is open
        let out = sa.show_viewport(ui, |ui, vp| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(LABEL_W + lane_w + 12.0, total_h), Sense::hover());
            let o = rect.min;
            let vis_x0 = o.x + vp.min.x;
            let lane_x = o.x + LABEL_W;
            let x_of = |t: f64| lane_x + (t / len) as f32 * lane_w;
            let t_of = |x: f32| ((x - lane_x) / lane_w) as f64 * len;
            let p = ui.painter().clone();
            let vis_t0 = t_of(vis_x0 + LABEL_W).max(0.0);
            let vis_t1 = t_of(vis_x0 + vp.width()).min(len);

            // Ruler: labelled ticks and small ones between; spacing adapts to the zoom.
            let ruler = Rect::from_min_size(Pos2::new(lane_x, o.y), Vec2::new(lane_w, 26.0));
            let step = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0].into_iter().find(|s| *s as f32 * lane_w / len as f32 >= 80.0).unwrap_or(60.0);
            let minor = step / 5.0;
            let mut k = (vis_t0 / minor).floor() as i64;
            while k as f64 * minor <= vis_t1 {
                let tk = k as f64 * minor;
                let x = x_of(tk);
                if k % 5 == 0 {
                    p.line_segment([Pos2::new(x, o.y + 14.0), Pos2::new(x, o.y + 26.0)], Stroke::new(1.0, MUTED));
                    let label = if step < 1.0 { fmt(tk) } else { fmt(tk).trim_end_matches(".0").to_string() };
                    p.text(Pos2::new(x + 4.0, o.y + 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(11.0), MUTED);
                } else {
                    p.line_segment([Pos2::new(x, o.y + 21.0), Pos2::new(x, o.y + 26.0)], Stroke::new(1.0, PANEL3));
                }
                k += 1;
            }
            let rr = ui.interact(ruler, ui.scope_id().with("ruler"), Sense::click_and_drag());
            if let Some(pp) = rr.interact_pointer_pos().filter(|_| rr.clicked() || rr.dragged()) {
                self.seek(t_of(pp.x));
            }

            let mut y = o.y + 28.0;
            // Song row: bass (purple), mids (teal), highs (white); bar starts as lines, beats as ticks.
            if let Some(an) = self.an.clone() {
                let r = Rect::from_min_size(Pos2::new(lane_x, y), Vec2::new(lane_w, audio_h - 6.0));
                p.rect_filled(r, 6, PANEL2);
                for (k, &(t0, part)) in an.sections.iter().enumerate() {
                    let t1 = an.sections.get(k + 1).map_or(len, |s| s.0);
                    if t1 < vis_t0 || t0 > vis_t1 {
                        continue;
                    }
                    let band = Rect::from_min_max(Pos2::new(x_of(t0) + 1.0, r.top() + 2.0), Pos2::new(x_of(t1) - 1.0, r.top() + 16.0));
                    p.rect_filled(band, 4, part_colour(part).gamma_multiply(0.35));
                    if band.width() > 44.0 {
                        p.with_clip_rect(band).text(band.left_center() + Vec2::new(5.0, 0.0), egui::Align2::LEFT_CENTER, part.name(), egui::FontId::proportional(10.5), TEXT);
                    }
                }
                let n = an.bands.len() / 3;
                let (cols, scale) = ([ACCENT2, ACCENT, TEXT], [0.48, 0.34, 0.2]);
                let mut x = x_of(vis_t0).max(lane_x);
                let x_end = x_of(vis_t1);
                while x < x_end {
                    let f0 = (t_of(x) * an.fps).max(0.0) as usize;
                    let f1 = ((t_of(x + 2.0) * an.fps) as usize + 1).min(n);
                    for k in 0..3 {
                        let m = (f0..f1).map(|f| an.bands[f * 3 + k]).max().unwrap_or(0) as f32 / 255.0;
                        let a = m * r.height() * scale[k];
                        p.rect_filled(Rect::from_center_size(Pos2::new(x, r.center().y + 6.0), Vec2::new(1.5, a * 1.6)), 0, cols[k]);
                    }
                    x += 2.0;
                }
                for &b in an.beats.iter().filter(|b| **b >= vis_t0 && **b <= vis_t1) {
                    let tick = if self.zoom > 3.0 { 12.0 } else { 5.0 };
                    p.line_segment([Pos2::new(x_of(b), r.bottom() - tick), Pos2::new(x_of(b), r.bottom())], Stroke::new(1.0, Color32::from_white_alpha(140)));
                }
                for &b in an.downbeats.iter().filter(|b| **b >= vis_t0 && **b <= vis_t1) {
                    p.line_segment([Pos2::new(x_of(b), r.top() + 17.0), Pos2::new(x_of(b), r.bottom())], Stroke::new(1.0, WARN.gamma_multiply(0.5)));
                }
                let resp = ui.interact(r, ui.scope_id().with("song"), Sense::click());
                if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
                    self.seek(t_of(pp.x));
                }
                y += audio_h;
            }

            // Track lanes: click to seek, drop effects, presets and lights.
            let rows_y = y;
            let row_at = |py: f32| (((py - rows_y) / ROW_H).floor() as isize).clamp(0, n_tracks.saturating_sub(1) as isize) as usize;
            for ti in 0..n_tracks {
                let r = Rect::from_min_size(Pos2::new(lane_x, rows_y + ti as f32 * ROW_H + 2.0), Vec2::new(lane_w, ROW_H - 4.0));
                let resp = ui.interact(r, ui.scope_id().with(("lane", ti)), Sense::click());
                let drop_hover = resp.dnd_hover_payload::<Drop>().is_some();
                p.rect_filled(r, 4, if ti == self.ti { PANEL2 } else { Color32::from_rgb(0x21, 0x23, 0x2d) });
                if drop_hover {
                    p.rect_stroke(r, 4, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
                }
                if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
                    self.seek(t_of(pp.x));
                    (self.ti, self.sel) = (ti, None);
                    self.multi.clear();
                }
                if let Some(d) = resp.dnd_release_payload::<Drop>() {
                    let at = t_of(ui.input(|i| i.pointer.hover_pos()).map_or(lane_x, |p| p.x));
                    self.drop_on_track(ti, &d, at);
                }
            }

            // Clips: click to select (Ctrl+click: several), drag to move (also to another track),
            // drag the right edge to resize, double-click a compound clip to open it.
            let press_x = ui.input(|i| i.pointer.press_origin()).map(|p| p.x);
            let mods = ui.input(|i| i.modifiers);
            for ti in 0..n_tracks {
                let locked = self.sc().tracks.get(self.real_track(ti)).is_some_and(|t| t.locked);
                for ci in 0..self.lane(ti).len() {
                    let Some(c) = self.lane(ti).get(ci).cloned() else { continue };
                    let row = match &self.drag { Some(d) if d.ti == ti && d.ci == ci => d.target, _ => ti };
                    let r = Rect::from_min_max(
                        Pos2::new(x_of(c.start), rows_y + row as f32 * ROW_H + 4.0),
                        Pos2::new(x_of(c.start + c.len).max(x_of(c.start) + 6.0), rows_y + row as f32 * ROW_H + ROW_H - 4.0),
                    );
                    let resp = ui.interact(r, ui.scope_id().with(("clip", ti, ci)), if locked { Sense::click() } else { Sense::click_and_drag() });
                    let edge = !locked && resp.hover_pos().is_some_and(|hp| hp.x > r.right() - 8.0);
                    let resp = if locked { resp.on_hover_text("Track is locked") } else if edge { resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal) } else { resp.on_hover_cursor(egui::CursorIcon::Grab) };
                    if resp.drag_started() {
                        let resize = press_x.is_some_and(|px| px > r.right() - 8.0);
                        self.drag = Some(Drag { ti, ci, resize, s0: c.start, l0: c.len, target: ti });
                        (self.sel, self.ti) = (Some((ti, ci)), ti);
                        self.multi.clear();
                    }
                    if resp.dragged() {
                        if let (Some((resize, s0, l0)), Some(pp), Some(px)) = (self.drag.as_ref().map(|d| (d.resize, d.s0, d.l0)), resp.interact_pointer_pos(), press_x) {
                            let dt = ((pp.x - px) / lane_w) as f64 * len;
                            let v = if resize { s0 + l0 + dt } else { s0 + dt };
                            let v = if self.snap { self.cx().snap(v) } else { v };
                            if resize {
                                let new_len = (v.min(len) - c.start).max(0.1);
                                if let Some(l) = self.lane_mut(ti) {
                                    l[ci].len = new_len;
                                }
                            } else {
                                let start = v.clamp(0.0, (len - l0).max(0.0));
                                if let Some(l) = self.lane_mut(ti) {
                                    let d = start - l[ci].start;
                                    l[ci].shift(d);
                                }
                                let target = if self.inside.is_some() { ti } else { row_at(pp.y) };
                                if let Some(d) = self.drag.as_mut() {
                                    d.target = target;
                                }
                            }
                        }
                    }
                    if resp.drag_stopped() {
                        if let Some(d) = self.drag.take() {
                            if d.target != d.ti && self.inside.is_none() {
                                let clip = self.sc_mut().tracks[d.ti].clips.remove(d.ci);
                                let dst = &mut self.scenes[self.cur].tracks[d.target].clips;
                                dst.push(clip);
                                (self.sel, self.ti) = (Some((d.target, dst.len() - 1)), d.target);
                            }
                        }
                    }
                    if resp.double_clicked() && c.fx == COMPOUND && self.inside.is_none() {
                        enter = Some((ti, ci));
                        self.multi.clear();
                    } else if resp.clicked() {
                        match self.sel {
                            Some((st, sc)) if st == ti && (mods.command || mods.shift) && sc != ci => {
                                if let Some(k) = self.multi.iter().position(|&m| m == ci) {
                                    self.multi.remove(k);
                                } else {
                                    self.multi.push(ci);
                                }
                            }
                            _ => {
                                (self.sel, self.ti) = (Some((ti, ci)), ti);
                                self.multi.clear();
                            }
                        }
                    }
                    let c = self.lane(ti).get(ci).cloned().unwrap_or(c);
                    let picked = self.sel == Some((ti, ci)) || (self.sel.is_some_and(|(t, _)| t == ti) && self.multi.contains(&ci));
                    draw_clip(&p, r, &c, picked, |t| x_of(t));
                }
            }

            // Playhead: grab it anywhere along its line (registered last, so it wins over lanes and clips).
            let px = x_of(self.t);
            let grip = Rect::from_min_max(Pos2::new(px - 6.0, o.y), Pos2::new(px + 6.0, rect.bottom()));
            let ph = ui.interact(grip, ui.scope_id().with("playhead"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if let Some(pp) = ph.interact_pointer_pos().filter(|_| ph.dragged()) {
                self.seek(t_of(pp.x));
            }
            let px = x_of(self.t);
            p.line_segment([Pos2::new(px, o.y), Pos2::new(px, rect.bottom())], Stroke::new(1.5, PLAYHEAD));
            p.rect_filled(Rect::from_center_size(Pos2::new(px, o.y + 8.0), Vec2::new(12.0, 16.0)), 4, PLAYHEAD);

            // Track headers stay on the left while scrolling sideways.
            let head = Rect::from_min_size(Pos2::new(vis_x0, o.y), Vec2::new(LABEL_W, total_h));
            p.rect_filled(head, 0, PANEL);
            p.line_segment([head.right_top(), head.right_bottom()], Stroke::new(1.0, PANEL3));
            if self.an.is_some() {
                let name = self.sc().audio.as_ref().map_or(String::new(), |a| a.name.clone());
                let cy = o.y + 28.0 + audio_h / 2.0 - 3.0;
                p.text(Pos2::new(vis_x0 + 14.0, cy), egui::Align2::LEFT_CENTER, "🎵", egui::FontId::proportional(14.0), ACCENT2);
                p.with_clip_rect(head).text(Pos2::new(vis_x0 + 36.0, cy), egui::Align2::LEFT_CENTER, short(&name, 22), egui::FontId::proportional(12.5), TEXT);
            }
            for ti in 0..n_tracks {
                let r = Rect::from_min_size(Pos2::new(vis_x0, rows_y + ti as f32 * ROW_H), Vec2::new(LABEL_W, ROW_H));
                let rt = self.real_track(ti);
                let Some(tr) = self.sc().tracks.get(rt).cloned() else { continue };
                if ti == self.ti {
                    p.rect_filled(r.shrink(2.0), 6, PANEL2);
                }
                if let Some((_, c)) = self.inside {
                    let label = tr.clips.get(c).map_or(String::new(), |c| c.label.clone());
                    p.text(Pos2::new(r.left() + 12.0, r.top() + 15.0), egui::Align2::LEFT_CENTER, format!("▣ {}", short(&label, 18)), egui::FontId::proportional(13.0), CMP_C);
                    p.with_clip_rect(r).text(Pos2::new(r.left() + 14.0, r.top() + 32.0), egui::Align2::LEFT_CENTER, format!("inside {}", tr.name), egui::FontId::proportional(11.5), MUTED);
                    continue;
                }
                let hr = ui.interact(r, ui.scope_id().with(("head", ti)), Sense::click());
                if hr.dnd_hover_payload::<Drop>().is_some() {
                    p.rect_stroke(r.shrink(2.0), 6, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
                }
                if let Some(d) = hr.dnd_release_payload::<Drop>() {
                    self.drop_on_track(ti, &d, self.t);
                }
                if hr.clicked() {
                    (self.ti, self.sel) = (ti, None);
                    self.multi.clear();
                }
                let [dr, dg, db] = to_u8(fx::hsl(160.0 + ti as f32 * 70.0, 70.0, 60.0));
                p.circle_filled(Pos2::new(r.left() + 14.0, r.top() + 15.0), 5.0, Color32::from_rgb(dr, dg, db));
                let mut name = tr.name.clone();
                let te = ui.put(
                    Rect::from_min_size(Pos2::new(r.left() + 24.0, r.top() + 4.0), Vec2::new(LABEL_W - 30.0, 22.0)),
                    egui::TextEdit::singleline(&mut name).frame(Frame::NONE).text_color(if ti == self.ti { ACCENT } else { TEXT }),
                );
                if te.changed() {
                    self.sc_mut().tracks[ti].name = name;
                }
                p.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 52.0, r.bottom()))).text(Pos2::new(r.left() + 26.0, r.top() + 32.0), egui::Align2::LEFT_CENTER, self.track_light(&tr), egui::FontId::proportional(11.5), MUTED);
                for (k, glyph, tip) in [(0, if tr.locked { "🔒" } else { "🔓" }, "Lock: its clips can't be moved"), (1, if tr.muted { "🔇" } else { "🔊" }, "Mute: this track sends nothing")] {
                    let b = Rect::from_center_size(Pos2::new(r.right() - 40.0 + k as f32 * 22.0, r.top() + 32.0), Vec2::splat(20.0));
                    let br = ui.interact(b, ui.scope_id().with(("tbtn", ti, k)), Sense::click()).on_hover_text(tip);
                    let on = if k == 0 { tr.locked } else { tr.muted };
                    p.text(b.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(13.0), if on { WARN } else if br.hovered() { TEXT } else { MUTED });
                    if br.clicked() {
                        let t = &mut self.sc_mut().tracks[ti];
                        if k == 0 { t.locked = !t.locked } else { t.muted = !t.muted }
                    }
                }
            }
            (lane_x, lane_w)
        });

        if let Some((ti, ci)) = enter {
            self.enter_compound(ti, ci);
        }
        // Ctrl + scroll zooms around the mouse; while playing, keep the playhead in view.
        let (inner, offset) = (out.inner_rect, out.state.clamped_offset().x);
        let zd = ui.input(|i| i.zoom_delta());
        if let Some(hp) = ui.input(|i| i.pointer.hover_pos()).filter(|hp| inner.contains(*hp) && zd != 1.0) {
            let (lane_x, lane_w) = out.inner;
            let t_p = ((hp.x - lane_x) / lane_w) as f64 * len;
            self.zoom = (self.zoom * zd).clamp(1.0, 64.0);
            let new_w = base_w * self.zoom;
            self.scroll_to = Some(LABEL_W + (t_p / len) as f32 * new_w - (hp.x - inner.min.x));
        } else if self.playing && self.zoom > 1.0 {
            let x = LABEL_W + (self.t / len) as f32 * lane_w;
            if x < offset + LABEL_W || x > offset + inner.width() - 60.0 {
                self.scroll_to = Some(x - LABEL_W - 40.0);
            }
        }
    }

    fn drop_on_track(&mut self, ti: usize, d: &Drop, at: f64) {
        match d {
            Drop::Fx(fxn) => self.add_clip(ti, fxn, at),
            Drop::Preset(c) => self.add_preset(ti, c.clone(), at),
            Drop::Light(id) => {
                let ti = self.real_track(ti);
                let segs = self.full_segs(id);
                let t = &mut self.sc_mut().tracks[ti];
                (t.dev, t.from, t.segs) = (Some(id.clone()), 0, segs);
                self.ti = ti;
            }
        }
    }
}

// ---------- helpers ----------

/// Is version a (like "0.2.0") newer than b?
fn newer(a: &str, b: &str) -> bool {
    let p = |s: &str| s.split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    p(a) > p(b)
}

#[test]
fn version_compare() {
    assert!(newer("0.2.0", "0.1.0") && newer("0.10.0", "0.9.9") && !newer("0.1.0", "0.1.0") && !newer("0.0.9", "0.1.0"));
}

/// A flat toolbar button.
fn tool(ui: &mut Ui, text: &str, tip: &str, on: bool) -> bool {
    ui.add_enabled(on, egui::Button::new(RichText::new(text).size(15.0)).frame(false).min_size(Vec2::new(30.0, 28.0))).on_hover_text(tip).on_disabled_hover_text(tip).clicked()
}

fn part_colour(p: Part) -> Color32 {
    match p {
        Part::Chorus => ACCENT,
        Part::Verse => ACCENT2,
        Part::Gap | Part::Break => WARN,
        Part::Intro | Part::Outro => MUTED,
    }
}

fn rgb32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Small colour bars, one per colour.
fn swatches(ui: &mut Ui, cols: &[[u8; 3]]) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(cols.len().min(8) as f32 * 7.0, 14.0), Sense::hover());
    for (k, c) in cols.iter().take(8).enumerate() {
        ui.painter().rect_filled(Rect::from_min_size(r.min + Vec2::new(k as f32 * 7.0, 0.0), Vec2::new(6.0, 14.0)), 2, rgb32(*c));
    }
}

/// A selectable button with the set's colours in front of its name.
fn swatch_button(ui: &mut Ui, on: bool, name: &str, cols: &[[u8; 3]]) -> egui::Response {
    let n = cols.len().min(8);
    let r = ui.add(egui::Button::selectable(on, format!("{}{name}", " ".repeat(2 + n * 2))));
    for (k, c) in cols.iter().take(n).enumerate() {
        let sw = Rect::from_min_size(r.rect.left_center() + Vec2::new(8.0 + k as f32 * 7.0, -6.0), Vec2::new(6.0, 12.0));
        ui.painter().rect_filled(sw, 2, rgb32(*c));
    }
    r
}

/// How many library tiles fit side by side.
fn per_row(ui: &Ui) -> usize {
    ((ui.available_width() + 8.0) / 120.0).floor().max(1.0) as usize
}

/// A library tile: a moving preview of the clip and its name. Drag it out or click it.
fn tile(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, payload: Drop, name: &str, c: &Clip, t: f64, cx: &Ctx) -> egui::Response {
    let demo = Track { name: String::new(), segs: 10, from: 0, dev: None, muted: false, locked: false, clips: vec![c.clone()] };
    ui.dnd_drag_source(ui.scope_id().with(id), payload, |ui| {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(112.0, 82.0), Sense::click());
        let p = ui.painter().clone();
        p.rect_filled(r, 10, if resp.hovered() { PANEL3 } else { PANEL2 });
        let pr = Rect::from_min_size(r.min + Vec2::splat(6.0), Vec2::new(r.width() - 12.0, 46.0));
        p.rect_filled(pr, 8, STAGE_BG);
        segments(ui, pr.shrink2(Vec2::new(7.0, 15.0)), &fx::track_colors(&demo, t, cx), 1.0, 2.0);
        p.with_clip_rect(r).text(Pos2::new(r.left() + 8.0, r.bottom() - 15.0), egui::Align2::LEFT_CENTER, short(name, 15), egui::FontId::proportional(12.5), TEXT);
        resp
    }).inner
}

/// A clip on the timeline: coloured by kind, name in its header, its colours (or the clips inside) below.
fn draw_clip(p: &egui::Painter, r: Rect, c: &Clip, picked: bool, x_of: impl Fn(f64) -> f32) {
    let compound = c.fx == COMPOUND;
    let kind = if compound { CMP_C } else if fx::REACTIVE.contains(&c.fx.as_str()) { REACT_C } else { PAT_C };
    p.rect_filled(r, 5, kind.gamma_multiply(0.35));
    let head = Rect::from_min_max(r.min, Pos2::new(r.right(), (r.top() + 15.0).min(r.bottom())));
    p.rect_filled(head, CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, kind);
    let body = Rect::from_min_max(Pos2::new(r.left() + 2.0, head.bottom() + 2.0), Pos2::new(r.right() - 2.0, r.bottom() - 2.0));
    if body.width() > 2.0 && body.height() > 2.0 {
        let body_v = body.intersect(p.clip_rect());
        if compound {
            for ic in &c.inner {
                let b = Rect::from_min_max(Pos2::new(x_of(ic.start).max(body.left()), body.top()), Pos2::new(x_of(ic.start + ic.len).min(body.right()), body.bottom()));
                if b.width() > 0.5 && b.intersects(body_v) {
                    p.rect_filled(b.shrink2(Vec2::new(0.5, 0.0)), 2, rgb32(ic.col).gamma_multiply(0.5));
                }
            }
        } else {
            let cols: Vec<[u8; 3]> = if !c.pal.is_empty() { c.pal.clone() } else if c.fx == "Solid" { vec![c.col] } else { vec![c.col, c.col2] };
            let n = 12;
            let w = body.width() / n as f32;
            for s in 0..n {
                let f = s as f32 / (n - 1) as f32 * (cols.len() - 1) as f32;
                let (a, b) = (cols[f.floor() as usize], cols[(f.ceil() as usize).min(cols.len() - 1)]);
                let k = f.fract();
                let m = [0, 1, 2].map(|j| (a[j] as f32 + (b[j] as f32 - a[j] as f32) * k) as u8);
                let sr = Rect::from_min_size(Pos2::new(body.left() + s as f32 * w, body.top()), Vec2::new(w + 0.5, body.height()));
                if sr.intersects(body_v) {
                    p.rect_filled(sr, 0, rgb32(m).gamma_multiply(0.6));
                }
            }
        }
    }
    if picked {
        p.rect_stroke(r.expand(1.0), 6, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
    }
    let name = if compound { format!("▣ {}", c.label) } else { c.fx.clone() };
    p.with_clip_rect(head.intersect(p.clip_rect())).text(Pos2::new(r.left() + 6.0, head.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(11.5), Color32::WHITE);
}

/// Plain-words facts about the song, and where the big drops are.
fn insights(an: &Analysis, len: f64) -> (Vec<String>, Vec<f64>) {
    let nf = an.bands.len() / 3;
    if nf == 0 || an.downbeats.len() < 3 {
        return (vec![], vec![]);
    }
    let fr = |t: f64| (((t - an.lag) * an.fps).max(0.0) as usize).min(nf);
    let avg = |a: f64, b: f64, f: &dyn Fn(usize) -> f64| {
        let (f0, f1) = (fr(a), fr(b));
        if f1 <= f0 { 0.0 } else { (f0..f1).map(f).sum::<f64>() / (f1 - f0) as f64 }
    };
    let loud = |g: usize| an.bands[g * 3..g * 3 + 3].iter().map(|&v| v as f64).sum::<f64>() / 765.0;
    let sp = crate::analysis::SPEC;
    let voice = |g: usize| if an.spec.len() >= (g + 1) * sp { (9..15).map(|k| an.spec[g * sp + k] as f64).sum::<f64>() / 6.0 / 255.0 } else { 0.0 };
    let mut out = vec![format!("{:.0} BPM · {} bars · {}", an.bpm, an.downbeats.len(), fmt(len))];

    let mut parts: Vec<(f64, f64, String)> = vec![];
    let mut seen: Vec<&str> = vec![];
    for (k, &(t0, p)) in an.sections.iter().enumerate() {
        seen.push(p.name());
        let n = seen.iter().filter(|x| **x == p.name()).count();
        parts.push((t0, an.sections.get(k + 1).map_or(len, |s| s.0), format!("{} {n}", p.name())));
    }
    let count = |n: &str| seen.iter().filter(|x| **x == n).count();
    out.push(format!("{} choruses, {} verses, {} breaks", count("Chorus"), count("Verse"), count("Break") + count("Stop")));
    if let Some((t0, _, n)) = parts.iter().max_by(|a, b| avg(a.0, a.1, &loud).total_cmp(&avg(b.0, b.1, &loud))) {
        out.push(format!("Loudest part: {n} at {}", fmt(*t0)));
    }

    // Drops: a bar much louder than the two before it.
    let bars = &an.downbeats;
    let e: Vec<f64> = bars.windows(2).map(|w| avg(w[0], w[1], &loud)).collect();
    let mean = e.iter().sum::<f64>() / e.len() as f64;
    let mut drops: Vec<f64> = vec![];
    for k in 2..e.len() {
        if e[k] > mean && e[k] > (e[k - 1] + e[k - 2]) / 2.0 * 1.3 && drops.last().is_none_or(|&d| bars[k] - d > 8.0) {
            drops.push(bars[k]);
        }
    }
    out.push(if drops.is_empty() { "No big drops".into() } else { format!("Big drops at {}", drops.iter().map(|d| fmt(*d)).collect::<Vec<_>>().join(", ")) });

    // Vocals: bars clearly above the song's average in the vocal range.
    let v: Vec<f64> = bars.windows(2).map(|w| avg(w[0], w[1], &voice)).collect();
    let vm = v.iter().sum::<f64>() / v.len() as f64;
    let sung: Vec<usize> = (0..v.len()).filter(|&k| vm > 0.0 && v[k] > vm * 1.15).collect();
    if let (Some(&a), Some(&b)) = (sung.first(), sung.last()) {
        out.push(format!("Vocals strongest from {} to {}", fmt(bars[a]), fmt(bars[b + 1])));
    }
    let third = e.len() / 3;
    if third > 0 {
        let (a, b) = (e[..third].iter().sum::<f64>(), e[e.len() - third..].iter().sum::<f64>());
        out.push(if b > a * 1.2 { "Builds up: louder towards the end".into() } else if a > b * 1.2 { "Winds down: quieter towards the end".into() } else { "About as loud all the way through".into() });
    }
    (out, drops)
}

fn to_u8(c: Rgb) -> [u8; 3] {
    c.map(|v| v.round().clamp(0.0, 255.0) as u8)
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.into() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

fn fmt(s: f64) -> String {
    format!("{}:{:04.1}", (s / 60.0).floor() as u64, s.rem_euclid(60.0))
}

/// A row of segments filling `r`; empty segments are dim.
fn segments(ui: &Ui, r: Rect, cols: &[Option<Rgb>], k: f32, gap: f32) {
    let n = cols.len().max(1) as f32;
    let gap = gap.min(r.width() / n * 0.3);
    let w = ((r.width() - gap * (n - 1.0)) / n).max(0.5);
    for (i, c) in cols.iter().enumerate() {
        let s = Rect::from_min_size(Pos2::new(r.left() + i as f32 * (w + gap), r.top()), Vec2::new(w, r.height()));
        let fill = c.map_or(Color32::from_rgb(0x1b, 0x1d, 0x26), |c| {
            let [r, g, b] = to_u8(c.map(|v| v * k));
            Color32::from_rgb(r, g, b)
        });
        ui.painter().rect_filled(s, (w.min(r.height()) / 3.0).min(6.0) as u8, fill);
    }
}

/// A rounded panel filling exactly `r`.
fn card_at(ui: &mut Ui, r: Rect, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
        ui.set_clip_rect(r.intersect(ui.clip_rect()));
        Frame::new().fill(PANEL).corner_radius(10).inner_margin(10).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.set_max_size(ui.available_size());
            add(ui);
        });
    });
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new().fill(PANEL).corner_radius(14).inner_margin(16).show(ui, |ui| {
        ui.set_width(226.0);
        ui.vertical(add);
    });
}

fn card_wide(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new().fill(PANEL).corner_radius(14).inner_margin(16).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(add);
    });
}

fn pill(ui: &mut Ui, text: &str, on: bool) -> egui::Response {
    let b = egui::Button::new(RichText::new(text).strong().color(if on { ON_ACCENT } else { MUTED })).corner_radius(255).min_size(Vec2::new(78.0, 30.0));
    ui.add(if on { b.fill(ACCENT) } else { b.fill(Color32::TRANSPARENT) })
}

#[derive(Clone, Copy)]
enum Icon {
    Play,
    Pause,
    Prev,
    Next,
}

/// Round button with a drawn icon (Segoe UI has no media symbols).
fn icon_button(ui: &mut Ui, icon: Icon, size: f32, accent: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let bg = match (accent, resp.hovered()) {
        (true, false) => ACCENT,
        (true, true) => Color32::from_rgb(0x4e, 0xe6, 0xc4),
        (false, false) => PANEL3,
        (false, true) => Color32::from_rgb(0x3a, 0x3f, 0x50),
    };
    let p = ui.painter();
    p.circle_filled(r.center(), size / 2.0, bg);
    let (c, s, fg) = (r.center(), size * 0.18, if accent { ON_ACCENT } else { TEXT });
    let tri = |dir: f32, cx: f32| vec![Pos2::new(cx - s * 0.8 * dir, c.y - s), Pos2::new(cx - s * 0.8 * dir, c.y + s), Pos2::new(cx + s * dir, c.y)];
    match icon {
        Icon::Play => { p.add(egui::Shape::convex_polygon(tri(1.0, c.x + s * 0.15), fg, Stroke::NONE)); }
        Icon::Pause => {
            for dx in [-0.5, 0.5] {
                p.rect_filled(Rect::from_center_size(Pos2::new(c.x + dx * s * 1.1, c.y), Vec2::new(s * 0.55, s * 2.0)), 1, fg);
            }
        }
        Icon::Prev | Icon::Next => {
            let dir = if matches!(icon, Icon::Next) { 1.0 } else { -1.0 };
            p.add(egui::Shape::convex_polygon(tri(dir, c.x - dir * s * 0.2), fg, Stroke::NONE));
            p.rect_filled(Rect::from_center_size(Pos2::new(c.x + dir * s * 1.05, c.y), Vec2::new(s * 0.35, s * 2.0)), 1, fg);
        }
    }
    resp
}

fn accent_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).strong().color(ON_ACCENT)).fill(ACCENT))
}

fn style(ctx: &egui::Context) {
    // Segoe UI (ships with Windows) instead of a bundled font; egui's own fonts stay as fallback for symbols.
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(b) = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(egui::FontData::from_owned(b)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    // Symbols and icons (music note, lock, speaker...) that Segoe UI itself doesn't have.
    if let Ok(b) = std::fs::read("C:\\Windows\\Fonts\\seguisym.ttf") {
        fonts.font_data.insert("symbols".into(), Arc::new(egui::FontData::from_owned(b)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().push("symbols".into());
    }
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|s| {
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        (v.panel_fill, v.window_fill, v.extreme_bg_color, v.faint_bg_color) = (BG, PANEL, BG, PANEL2);
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.hyperlink_color = ACCENT;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
            w.corner_radius = CornerRadius::same(10);
        }
        (v.widgets.inactive.bg_fill, v.widgets.inactive.weak_bg_fill) = (PANEL3, PANEL3);
        (v.widgets.hovered.bg_fill, v.widgets.hovered.weak_bg_fill) = (Color32::from_rgb(0x3a, 0x3f, 0x50), Color32::from_rgb(0x3a, 0x3f, 0x50));
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, PANEL3);
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        s.text_styles = [
            (TextStyle::Small, FontId::new(12.5, Proportional)),
            (TextStyle::Body, FontId::new(15.0, Proportional)),
            (TextStyle::Button, FontId::new(15.0, Proportional)),
            (TextStyle::Heading, FontId::new(20.0, Proportional)),
            (TextStyle::Monospace, FontId::new(14.0, egui::FontFamily::Monospace)),
        ].into();
        s.spacing.item_spacing = Vec2::new(10.0, 8.0);
        s.spacing.button_padding = Vec2::new(14.0, 7.0);
        s.spacing.interact_size.y = 32.0;
    });
}
