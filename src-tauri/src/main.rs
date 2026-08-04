// Prevents a second console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    codex_usage_bar_lib::run();
}
