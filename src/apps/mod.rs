//! Core Desktop Applications for LobsterOS

pub mod editor;
pub mod file_manager;
pub mod settings;
pub mod terminal;

use crate::println;

/// Initialize all core desktop applications
pub fn init() {
    terminal::init();
    file_manager::init();
    editor::init();
    settings::init();
    println!("[apps] Core desktop applications initialized");
}
