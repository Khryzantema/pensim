#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    pensim_app::run_desktop()
}

// No Android o ponto de entrada é `android_main` (lib); o binário fica vazio.
#[cfg(target_os = "android")]
fn main() {}
