//! Core Desktop Applications for LobsterOS

pub mod app_store;
pub mod browser;
pub mod calculator;
pub mod editor;
pub mod file_manager;
pub mod image_viewer;
pub mod network_manager;
pub mod notes;
pub mod process_manager;
pub mod settings;
pub mod setup_wizard;
pub mod sysmon;
pub mod task_manager;
pub mod terminal;

use crate::println;

/// Initialize all core desktop applications
pub fn init() {
    terminal::init();
    file_manager::init();
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
    println!("[apps] Core desktop applications initialized (12 apps)");
}
