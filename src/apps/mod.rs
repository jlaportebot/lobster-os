//! Core Desktop Applications for LobsterOS

pub mod app_store;
pub mod browser;
pub mod browser_text;
pub mod calculator;
pub mod calculator_engine;
pub mod editor;
pub mod files;
pub mod image_viewer;
pub mod network_manager;
pub mod notes;
pub mod process_manager;
pub mod settings;
pub mod setup_wizard;
pub mod sysmon;
pub mod task_manager;
pub mod terminal;
pub mod welcome;

use crate::println;

/// Initialize all core desktop applications
pub fn init() {
    terminal::init();
    files::init();
    editor::init();
    settings::init();
    calculator::init();
    sysmon::init();
    image_viewer::init();
    process_manager::init();
    network_manager::init();
    app_store::init();
    browser::init();
    notes::init();
    welcome::init();
    println!("[apps] Core desktop applications initialized (13 apps)");
}
