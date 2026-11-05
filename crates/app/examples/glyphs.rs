use eframe::egui;
struct G;
impl eframe::App for G {
    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        let all = "☰ ✎ ⟳ ▭ ▫ ✔ ✕ ＋ + ✥ ◗ ◧ ◎ ⏸ ▶ ⏮ ↺ ● ▲ ▼ ⚙ ▦ ◨ ■ □ ◀ ▷ ✓ ✗ × ÷ ° ² ³ µ ρ σ ε θ ≈ · … → ← ↑ ↓ ⏵ ⏹ ⏪ ⏩ ⌂ ⛶ ⚠ ℹ ✂ ⟲ ↻ ⤢ ⊕ ⊖ ✖ ☐ ☑ ≡ ⋮ ◉ ○";
        ui.horizontal_wrapped(|ui| {
            for g in all.split(' ') {
                let code = g.chars().next().map(|c| c as u32).unwrap_or(0);
                ui.label(egui::RichText::new(format!("{g}")).size(26.0));
                ui.label(egui::RichText::new(format!("{code:04X}")).size(10.0).weak());
                ui.add_space(6.0);
            }
        });
    }
}
fn main() -> eframe::Result {
    let o = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 900.0]), ..Default::default() };
    eframe::run_native("g", o, Box::new(|cc| { pensim_app::install_fonts(&cc.egui_ctx); Ok(Box::new(G)) }))
}
