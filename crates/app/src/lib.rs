mod app;
#[cfg(target_os = "android")]
pub(crate) static ANDROID_APP: std::sync::OnceLock<android_activity::AndroidApp> = std::sync::OnceLock::new();

mod plot;
mod sketch;
mod view3d;
mod worker;

pub use app::PenSim;

/// Fonte de reserva com cobertura de símbolos (setas, ▶, ✎, ☰…), que a fonte padrão não tem.
pub fn install_fonts(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".into(),
        std::sync::Arc::new(FontData::from_static(include_bytes!("../assets/DejaVuSans.ttf"))),
    );
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().push("dejavu".into());
    }
    ctx.set_fonts(fonts);
}

#[cfg(not(target_os = "android"))]
pub fn run_desktop() -> eframe::Result {
    // PENSIM_SIZE=923x415 simula a tela de um celular deitado
    let size = std::env::var("PENSIM_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1100.0, 720.0]);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size(size),
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
    let _ = ANDROID_APP.set(app.clone());
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    eframe::run_native("PenSim", options, Box::new(|cc| Ok(Box::new(PenSim::new(cc)))))
        .expect("falha ao iniciar a UI");
}
