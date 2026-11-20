#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod app;
mod audio;
mod fx;
mod govee;
mod lights;
mod live;
mod media;
mod scene;
mod show;

use eframe::egui;

/// The app icon: four coloured bars on a dark rounded square.
fn icon() -> egui::IconData {
    let (w, h) = (64u32, 64u32);
    let cols = [[0x2f, 0xd8, 0xb4], [0x7c, 0x6c, 0xff], [0xff, 0x7a, 0x1a], [0x2f, 0xd8, 0xb4]];
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy, r) = (x as f32 + 0.5, y as f32 + 0.5, 14.0);
            let (dx, dy) = ((r - fx).max(fx - (64.0 - r)).max(0.0), (r - fy).max(fy - (64.0 - r)).max(0.0));
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let bar = (x >= 12 && x < 56 && (x - 12) % 11 < 7 && (18..46).contains(&y)).then(|| ((x - 12) / 11) as usize);
            let c = bar.map_or([0x1b, 0x1e, 0x2b], |i| cols[i]);
            let o = ((y * w + x) * 4) as usize;
            rgba[o..o + 3].copy_from_slice(&c);
            rgba[o + 3] = 255;
        }
    }
    egui::IconData { rgba, width: w, height: h }
}

fn main() -> eframe::Result {
    let player = audio::Player::start();
    let lights = lights::Lights::start(player.clone());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Lightline").with_inner_size([1280.0, 820.0]).with_min_inner_size([960.0, 640.0]).with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native("Lightline", opts, Box::new(move |cc| Ok(Box::new(app::App::new(cc, player, lights)))))
}
