#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod app;
mod audio;
mod fx;
mod govee;
mod lights;
mod scene;

use eframe::egui;

fn main() -> eframe::Result {
    let player = audio::Player::start();
    let lights = lights::Lights::start(player.clone());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Lightline").with_inner_size([1280.0, 820.0]).with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native("Lightline", opts, Box::new(move |cc| Ok(Box::new(app::App::new(cc, player, lights)))))
}
