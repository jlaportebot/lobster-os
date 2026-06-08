//! Core Desktop Applications for LobsterOS

pub mod terminal;
pub mod file_manager;
pub mod editor;
pub mod settings;

use crate::println;

/// Initialize all core desktop applications
pub fn init() {
    terminal::init();
    file_manager::init();
    editor::init();
    settings::init();
    println!("[apps] Core desktop applications initialized");
}