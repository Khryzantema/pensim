mod app;
mod plot;
mod sketch;
mod view3d;
mod worker;

pub use app::PenSim;

#[cfg(not(target_os = "android"))]
pub fn run_desktop() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native("PenSim", options, Box::new(|cc| Ok(Box::new(PenSim::new(cc)))))
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    eframe::run_native("PenSim", options, Box::new(|cc| Ok(Box::new(PenSim::new(cc)))))
        .expect("falha ao iniciar a UI");
}
