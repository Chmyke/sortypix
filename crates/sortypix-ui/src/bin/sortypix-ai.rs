#![windows_subsystem = "windows"]
//! Sortypix AI: the same application with the background analysis (`--features ai`).
fn main() -> eframe::Result {
    sortypix_ui::launch::run()
}
