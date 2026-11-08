// Release builds are GUI applications without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Entry point: parses the command line, creates the window and starts the dungeon maker.
//!
//! Usage: `daggerfall-dungeon-maker [--arena2 <folder>] [--room <name.RDB>] [--dungeon <name>]`
//!
//! Every argument is optional. Without `--arena2` the program uses the
//! folder chosen last time, or asks for one.

use std::path::PathBuf;

use daggerfall_dungeon_maker::app::{DungeonMakerApp, StartupOptions};
use eframe::egui;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let startup_options = parse_arguments(std::env::args().skip(1));

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Daggerfall Dungeon Maker")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Daggerfall Dungeon Maker",
        native_options,
        Box::new(|creation_context| Ok(Box::new(DungeonMakerApp::new(creation_context, startup_options)))),
    )
}

/// Reads `--arena2 <folder>`, `--room <name>` and `--dungeon <name>` from the command line,
/// ignoring anything else.
fn parse_arguments(mut arguments: impl Iterator<Item = String>) -> StartupOptions {
    let mut options = StartupOptions::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--arena2" => options.arena2_folder = arguments.next().map(PathBuf::from),
            "--room" => options.room_name = arguments.next(),
            "--dungeon" => options.dungeon_name = arguments.next(),
            unknown => log::warn!("ignoring unknown argument '{unknown}'"),
        }
    }
    options
}
