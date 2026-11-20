//! Scenes, settings and the Govee model table, saved as JSON in %APPDATA%\com.lightline.app
//! (the same scenes.json the earlier version wrote, so nothing is lost).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub fx: String,
    pub start: f64,
    pub len: f64,
    #[serde(with = "hex", default = "teal")]
    pub col: [u8; 3],
    #[serde(with = "hex", default = "purple")]
    pub col2: [u8; 3],
    #[serde(default = "hundred")]
    pub bri: f64,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default)]
    pub rev: bool,
    #[serde(default)]
    pub fin: f64,
    #[serde(default)]
    pub fout: f64,
    /// Colours for the music effects (Spectrum, Meter, Visualizer), blended along the strip. Empty = rainbow.
    #[serde(default, with = "hexes")]
    pub pal: Vec<[u8; 3]>,
    /// Brightness follows: 0 nothing, 1 kick, 2 bass, 3 vocals, 4 highs.
    #[serde(default)]
    pub react: u8,
    /// Mirror the effect out from the centre.
    #[serde(default)]
    pub mirror: bool,
    /// Repeat the effect this many times along the strip.
    #[serde(default = "one_u8")]
    pub repeat: u8,
    /// How this clip comes in after the clip that ends where it starts: 0 cut, 1 crossfade, 2 wipe.
    #[serde(default)]
    pub trans: u8,
    #[serde(default = "half")]
    pub tlen: f64,
}

impl Default for Clip {
    fn default() -> Self {
        Clip { fx: "Solid".into(), start: 0.0, len: 4.0, col: teal(), col2: purple(), bri: 100.0, speed: 1.0, rev: false, fin: 0.0, fout: 0.0, pal: vec![], react: 0, mirror: false, repeat: 1, trans: 0, tlen: 0.5 }
    }
}

pub const REACT: [&str; 5] = ["Nothing", "Kick", "Bass", "Vocals", "Highs"];
pub const TRANS: [&str; 3] = ["Cut", "Crossfade", "Wipe"];
fn one_u8() -> u8 { 1 }
fn half() -> f64 { 0.5 }

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    pub segs: usize,
    #[serde(default)]
    pub from: usize,
    #[serde(default)]
    pub dev: Option<String>,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Audio {
    pub path: String,
    pub name: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    pub len: f64,
    pub bpm: f64,
    #[serde(default)]
    pub audio: Option<Audio>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// Per-PC settings.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub names: HashMap<String, String>, // light id -> name
    pub lsegs: HashMap<String, usize>,  // light id -> most colours it accepts
    pub loff_ms: HashMap<String, f64>,  // light id -> its own timing, negative = earlier
    pub lsmooth: HashMap<String, f64>,  // light id -> its own smoothing (else `smooth`)
    pub chain: Vec<String>,             // light ids in the order a chained track runs across them
    pub autonext: bool,                 // Live: play the next scene when one ends
    pub shuffle: bool,
    pub off_ms: f64,
    pub smooth: f64, // 0..100, fade = smooth * 3 ms
}

impl Default for Settings {
    fn default() -> Self {
        Settings { names: HashMap::new(), lsegs: HashMap::new(), loff_ms: HashMap::new(), lsmooth: HashMap::new(), chain: vec![], autonext: false, shuffle: false, off_ms: 0.0, smooth: 40.0 }
    }
}

fn teal() -> [u8; 3] { [0x2f, 0xd8, 0xb4] }
fn purple() -> [u8; 3] { [0x7c, 0x6c, 0xff] }
fn hundred() -> f64 { 100.0 }
fn one() -> f64 { 1.0 }

mod hex {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(c: &[u8; 3], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 3], D::Error> {
        let s = String::deserialize(d)?;
        let v = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(0);
        Ok([v(1), v(3), v(5)])
    }
}

mod hexes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &[[u8; 3]], s: S) -> Result<S::Ok, S::Error> {
        v.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect::<Vec<_>>().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[u8; 3]>, D::Error> {
        let v = Vec::<String>::deserialize(d)?;
        Ok(v.iter().map(|s| {
            let h = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(0);
            [h(1), h(3), h(5)]
        }).collect())
    }
}

pub fn demo() -> Scene {
    let clip = |fx: &str, start: f64, len: f64, col: [u8; 3]| Clip { fx: fx.into(), start, len, col, col2: purple(), bri: 100.0, speed: 1.0, rev: false, fin: 0.0, fout: 0.0, pal: vec![], ..Default::default() };
    let track = |name: &str, clips| Track { name: name.into(), segs: 15, from: 0, dev: None, clips };
    Scene {
        name: "Demo scene".into(),
        len: 60.0,
        bpm: 120.0,
        audio: None,
        tracks: vec![
            track("Zone A", vec![clip("Solid", 0.0, 8.0, [0xff, 0x8a, 0x2a]), clip("Rainbow", 16.0, 20.0, teal())]),
            track("Zone B", vec![clip("Chase", 4.0, 16.0, teal()), clip("Pulse", 30.0, 14.0, [0xff, 0x3d, 0x7f])]),
        ],
    }
}

fn dir() -> Result<PathBuf, String> {
    let d = PathBuf::from(std::env::var("APPDATA").map_err(|e| e.to_string())?).join("com.lightline.app");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Missing file = nothing saved yet. A file that exists but can't be read is an error (never overwrite it).
fn load<T: for<'de> Deserialize<'de>>(name: &str) -> Result<Option<T>, String> {
    match std::fs::read_to_string(dir()?.join(name)) {
        Ok(s) => serde_json::from_str(&s).map(Some).map_err(|e| format!("{name}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Write then rename, so a crash mid-write never leaves a half-written file.
fn save<T: Serialize>(name: &str, v: &T) -> Result<(), String> {
    let path = dir()?.join(name);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string(v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

pub fn load_scenes() -> Result<Option<Vec<Scene>>, String> { load("scenes.json") }
pub fn save_scenes(s: &[Scene]) -> Result<(), String> { save("scenes.json", &s) }
pub fn load_settings() -> Settings { load("settings.json").ok().flatten().unwrap_or_default() }
pub fn save_settings(s: &Settings) -> Result<(), String> { save("settings.json", s) }

pub struct Model {
    pub name: &'static str,
    pub len: &'static str,
    pub segs: usize,
}

/// code|name|length|segments shown in the Govee app (user-supplied sheet).
pub fn models() -> HashMap<&'static str, Model> {
    include_str!("models.txt")
        .lines()
        .filter_map(|l| {
            let mut p = l.split('|');
            let (sku, name, len, segs) = (p.next()?, p.next()?, p.next()?, p.next()?.trim().parse().ok()?);
            Some((sku, Model { name, len, segs }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_old_web_ui_scene_file() {
        let json = r##"[{"name":"S","len":60,"bpm":120,"tracks":[{"name":"T","segs":15,"dev":"AA","clips":[{"fx":"Solid","start":0,"len":8,"col":"#ff8000","col2":"#0000ff"}]},{"name":"U","segs":3,"from":4,"clips":[]}]}]"##;
        let s: Vec<super::Scene> = serde_json::from_str(json).unwrap();
        let c = &s[0].tracks[0].clips[0];
        assert_eq!((c.col, c.col2, c.bri, c.speed), ([255, 128, 0], [0, 0, 255], 100.0, 1.0));
        assert_eq!((s[0].tracks[1].from, s[0].tracks[1].dev.as_deref()), (4, None));
        let back = serde_json::to_string(&s).unwrap();
        assert!(back.contains("\"col\":\"#ff8000\""));
        assert!(c.pal.is_empty(), "old files: rainbow");
        let mut c = c.clone();
        c.pal = vec![[255, 0, 0], [0, 16, 255]];
        let back: super::Clip = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.pal, c.pal);
    }

    #[test]
    fn model_table_parses() {
        let m = super::models();
        assert_eq!(m.len(), 90);
        assert_eq!((m["H61F6"].name, m["H615A"].segs), ("Strip Light 2 Pro", 1));
    }
}
