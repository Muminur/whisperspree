// Hide the extra console window on non-macOS release builds; inert on macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    whisperspree_lib::run();
}
