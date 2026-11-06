//! Reqora — a fast, lightweight open-source API client for Linux, Windows and macOS.

// Release builds on Windows are GUI apps: don't open a console window next to them.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod curl;
mod highlight;
mod http;
mod model;
mod postman;
mod response;
mod storage;
mod theme;
mod ui_comments;
mod ui_kit;
mod ui_request;
mod ui_response;
mod ui_sidebar;
mod vars;
mod widgets;

use eframe::egui;

pub const APP_ID: &str = "io.github.Reqora";
pub const APP_NAME: &str = "Reqora";

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("reqora {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id(APP_ID)
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([760.0, 480.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
