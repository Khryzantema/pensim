mod app;
mod mesh;
#[cfg(target_os = "android")]
pub(crate) static ANDROID_APP: std::sync::OnceLock<android_activity::AndroidApp> = std::sync::OnceLock::new();

mod plot;
mod sketch;
mod view3d;
mod worker;

pub use app::PenSim;

/// Logo embutido (PNG 96×96) decodificado em RGBA.
fn logo_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let dec = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/logo-96.png").as_slice()));
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    Some((buf, info.width, info.height))
}

/// Textura do logo para a faixa superior do app.
pub fn load_logo(ctx: &eframe::egui::Context) -> Option<eframe::egui::TextureHandle> {
    let (rgba, w, h) = logo_rgba()?;
    let img = eframe::egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    Some(ctx.load_texture("logo", img, eframe::egui::TextureOptions::LINEAR))
}

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
        viewport: {
            let mut vb = eframe::egui::ViewportBuilder::default().with_inner_size(size).with_title("PenSim");
            if let Some((rgba, width, height)) = logo_rgba() {
                vb = vb.with_icon(eframe::egui::IconData { rgba, width, height });
            }
            vb
        },
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
