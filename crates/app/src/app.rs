use crate::plot::{line_plot, Series};
use crate::sketch::{self, SketchEditor, Tool};
use crate::view3d::{self, Camera, View};
use crate::worker::{Job, SweepJob};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use pensim_core::{Ammo, Config, Core, Design, Frame, Layer, Model, Nose, Outcome, OutcomeKind, Preset, AMMO, LIBRARY};
use std::sync::atomic::AtomicBool;

/// tempos médios do último intervalo [µs]: quadro inteiro, viewport 3D
static PROF_FRAME_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PROF_VIEW_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// layout compacto (tela deitada / baixa)
static COMPACT: AtomicBool = AtomicBool::new(false);
fn compact() -> bool {
    COMPACT.load(Ordering::Relaxed)
}
use std::sync::atomic::Ordering;
use std::sync::Arc;

const RES_PRESETS: [(&str, f64); 7] = [("Rascunho", 0.0025), ("Rápida", 0.002), ("Média", 0.0015), ("Fina", 0.001), ("Muito fina", 0.0008), ("Extrema", 0.0005), ("Ultra", 0.00025)];
/// taxa de reprodução (quadros/s) em velocidade 1×
const PLAY_FPS: f32 = 25.0;
const ACCENT: Color32 = Color32::from_rgb(0, 112, 192);

#[derive(Copy, Clone, PartialEq, Eq)]
enum Field {
    Material,
    VonMises,
    Pressure,
    Plastic,
    Temp,
    Damage,
    Speed,
}

impl Field {
    const ALL: [Field; 7] = [Field::Material, Field::VonMises, Field::Pressure, Field::Plastic, Field::Temp, Field::Damage, Field::Speed];
    fn label(self) -> &'static str {
        match self {
            Field::Material => "Geometria (materiais)",
            Field::VonMises => "Tensão equivalente (von Mises)",
            Field::Pressure => "Pressão hidrostática",
            Field::Plastic => "Deformação plástica equiv.",
            Field::Temp => "Temperatura",
            Field::Damage => "Dano (ligações rompidas)",
            Field::Speed => "Velocidade",
        }
    }
    fn short(self) -> &'static str {
        match self {
            Field::Material => "Material",
            Field::VonMises => "von Mises",
            Field::Pressure => "Pressão",
            Field::Plastic => "ε plást.",
            Field::Temp => "Temp.",
            Field::Damage => "Dano",
            Field::Speed => "|v|",
        }
    }
    fn unit(self) -> &'static str {
        match self {
            Field::VonMises | Field::Pressure => "GPa",
            Field::Temp => "K",
            Field::Speed => "m/s",
            _ => "-",
        }
    }
    fn scale(self) -> f32 {
        match self {
            Field::VonMises | Field::Pressure => 1e-9,
            _ => 1.0,
        }
    }
    fn data(self, f: &Frame) -> Option<&[f32]> {
        Some(match self {
            Field::Material => return None,
            Field::VonMises => &f.von_mises,
            Field::Pressure => &f.pressure,
            Field::Plastic => &f.plastic,
            Field::Temp => &f.temp,
            Field::Damage => &f.damage,
            Field::Speed => &f.speed,
        })
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Ws {
    Design,
    Resolution,
    Sim,
    Results,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Sel {
    Sketch,
    Projectile,
    Layer(usize),
    Conditions,
    Mesh,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ResTab {
    Summary,
    Charts,
    Limit,
}

struct Preview {
    pos: Vec<[f32; 3]>,
    part: Vec<u8>,
    idx: Vec<u32>,
    center: [f32; 3],
    radius: f32,
    err: Option<String>,
}

type Key = (Design, [f64; 5], bool);

struct Snap {
    frames: Vec<Arc<Frame>>,
    live: Option<Outcome>,
    done: bool,
    error: Option<String>,
    n: usize,
    dt: f64,
    steps: u64,
    wall: f64,
    sim_time: f64,
    t_end: f64,
    sym: u8,
    parts: Arc<Vec<u8>>,
    surface: Arc<Vec<bool>>,
    ref_pos: Arc<Vec<[f32; 3]>>,
}

pub struct PenSim {
    cfg: Config,
    ammo_idx: usize,
    ammo_scale: f64,
    show_side: bool,
    show_fps: bool,
    /// margens extras (pt) somadas à área segura automática: [esq, topo, dir, base]
    extra_inset: [f32; 4],
    ws: Ws,
    sel: Sel,
    res_tab: ResTab,
    sheet_tab: u8,
    sketch_mode: bool,
    target_mode: bool,
    sketch: SketchEditor,
    cam: Camera,
    job: Option<Job>,
    started_key: Option<Key>,
    sweep: Option<SweepJob>,
    sweep_range: (f64, f64),
    sweep_n: usize,
    field: Field,
    bands: bool,
    section: bool,
    range: [(f32, f32); 7],
    view_idx: usize,
    live_view: bool,
    autoplayed: bool,
    playing: bool,
    loop_play: bool,
    speed: f32,
    play_pos: f32,
    started_at: std::time::Instant,
    preview: Preview,
    preview_key: Option<Key>,
}

fn mat_color(name: &str) -> Color32 {
    let c = |r, g, b| Color32::from_rgb(r, g, b);
    if name.contains("Aço") { c(118, 134, 156) }
    else if name.contains("Alum") { c(190, 197, 206) }
    else if name.contains("Tit") { c(150, 140, 172) }
    else if name.contains("tungst") { c(64, 96, 108) }
    else if name.contains("Cobre") { c(205, 122, 72) }
    else if name.contains("Chumbo") { c(86, 92, 104) }
    else if name.contains("Concreto") { c(184, 172, 150) }
    else if name.contains("Cer") { c(236, 230, 214) }
    else { c(160, 160, 160) }
}

fn rainbow(t: f32, bands: bool) -> Color32 {
    let n = 12.0;
    let t = if bands { ((t.clamp(0.0, 1.0) * n).floor().min(n - 1.0) + 0.5) / n } else { t.clamp(0.0, 1.0) };
    let h = (1.0 - t) * 4.0;
    let (seg, f) = (h.floor() as i32, h - h.floor());
    let (r, g, b) = match seg {
        0 => (1.0, f, 0.0),
        1 => (1.0 - f, 1.0, 0.0),
        2 => (0.0, 1.0, f),
        3 => (0.0, 1.0 - f, 1.0),
        _ => (0.0, 0.0, 1.0),
    };
    Color32::from_rgb((r * 235.0) as u8, (g * 235.0) as u8, (b * 235.0 + 20.0).min(255.0) as u8)
}

impl PenSim {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::install_fonts(&cc.egui_ctx);
        let mut v = egui::Visuals::light();
        v.selection.bg_fill = ACCENT;
        v.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals(v);
        cc.egui_ctx.global_style_mut(|s| {
            s.spacing.interact_size.y = 32.0;
            s.spacing.item_spacing = Vec2::new(8.0, 6.0);
            s.spacing.slider_width = 150.0;
        });
        let ammo_idx = 4;
        let cfg = AMMO[ammo_idx].config(AMMO[ammo_idx].scale);
        let mut me = PenSim {
            cfg,
            ammo_idx,
            ammo_scale: AMMO[ammo_idx].scale,
            show_side: true,
            show_fps: std::env::var("PENSIM_PROF").is_ok(),
            extra_inset: [0.0, 0.0, 0.0, 0.0],
            ws: Ws::Design,
            sel: Sel::Projectile,
            res_tab: ResTab::Summary,
            sheet_tab: 0,
            sketch_mode: false,
            target_mode: false,
            sketch: SketchEditor::default(),
            cam: Camera::iso(),
            job: None,
            started_key: None,
            sweep: None,
            sweep_range: (400.0, 1500.0),
            sweep_n: 5,
            field: Field::VonMises,
            bands: true,
            section: true,
            range: [(0.0, 0.0); 7],
            view_idx: 0,
            live_view: false,
            autoplayed: false,
            playing: false,
            loop_play: false,
            speed: 1.0,
            play_pos: 0.0,
            started_at: std::time::Instant::now(),
            preview: Preview { pos: vec![], part: vec![], idx: vec![], center: [0.0; 3], radius: 0.05, err: None },
            preview_key: None,
        };
        // ganchos de teste visual (captura de telas): PENSIM_WS, PENSIM_SKETCH, PENSIM_DEMO
        if let Ok(w) = std::env::var("PENSIM_WS") {
            me.ws = match w.as_str() {
                "res" => Ws::Resolution,
                "sim" => Ws::Sim,
                "results" => Ws::Results,
                _ => Ws::Design,
            };
        }
        if std::env::var("PENSIM_TARGET").is_ok() {
            me.target_mode = true;
            me.cfg.design.layers.push(Layer { thickness: 0.008, gap_before: 0.004, width: 0.05, height: 0.05, round: false, material: LIBRARY[2].clone() });
            me.sketch.tsel = Some((1, true));
        }
        if std::env::var("PENSIM_SKETCH").is_ok() {
            me.sketch_mode = true;
            me.sel = Sel::Sketch;
            me.sketch.selected = Some(3);
        }
        if let Ok(a) = std::env::var("PENSIM_AMMO") {
            if let Ok(i) = a.parse::<usize>() {
                me.load_ammo(i.min(AMMO.len() - 1), AMMO[i.min(AMMO.len() - 1)].scale);
            }
        }
        if std::env::var("PENSIM_DEMO").is_ok() {
            me.start(&cc.egui_ctx);
            if std::env::var("PENSIM_ISO").is_ok() {
                me.cam = Camera::iso();
            }
            if std::env::var("PENSIM_NOSECTION").is_ok() {
                me.section = false;
            }
        }
        me
    }

    fn key(&self) -> Key {
        let c = &self.cfg;
        (c.design.clone(), [c.dx, c.horizon_m, c.velocity, c.obliquity_deg, c.time_factor], c.clamp_edges)
    }

    fn estimate_nodes(&self) -> f64 {
        self.cfg.estimate_nodes()
    }

    fn refresh_preview(&mut self) {
        let key = self.key();
        if self.preview_key.as_ref() == Some(&key) {
            return;
        }
        self.preview_key = Some(key);
        if self.estimate_nodes() > 700_000.0 {
            self.preview = Preview { pos: vec![], part: vec![], idx: vec![], center: [0.0; 3], radius: 0.05, err: Some("Malha fina demais para pré-visualizar (> 700 mil nós).".into()) };
            return;
        }
        match Model::build_full(&self.cfg) {
            Ok(m) => {
                let pos: Vec<[f32; 3]> = m.pos.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
                let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
                for p in &pos {
                    for k in 0..3 {
                        mn[k] = mn[k].min(p[k]);
                        mx[k] = mx[k].max(p[k]);
                    }
                }
                let center = [0, 1, 2].map(|k| 0.5 * (mn[k] + mx[k]));
                let radius = (0..3).map(|k| (mx[k] - mn[k]).powi(2)).sum::<f32>().sqrt() * 0.5;
                let idx = (0..pos.len() as u32).collect();
                self.preview = Preview { pos, part: m.part, idx, center, radius, err: None };
            }
            Err(e) => {
                self.preview = Preview { pos: vec![], part: vec![], idx: vec![], center: [0.0; 3], radius: 0.05, err: Some(e) };
            }
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.job = None;
        self.range = [(0.0, 0.0); 7];
        self.view_idx = 0;
        self.autoplayed = false;
        self.playing = false;
        self.play_pos = 0.0;
        self.started_at = std::time::Instant::now();
        self.started_key = Some(self.key());
        self.job = Some(Job::spawn(self.cfg.clone(), ctx.clone()));
        self.ws = Ws::Results;
        self.field = Field::VonMises;
        self.cam = Camera::front();
    }

    fn snapshot(&self) -> Snap {
        match &self.job {
            Some(j) => {
                let g = j.shared.lock().unwrap();
                Snap { frames: g.frames.clone(), live: g.live.clone(), done: g.done, error: g.error.clone(), n: g.n, dt: g.dt, steps: g.steps, wall: g.wall, sim_time: g.sim_time, t_end: g.t_end, sym: g.sym, parts: g.parts.clone(), surface: g.surface.clone(), ref_pos: g.ref_pos.clone() }
            }
            None => Snap { frames: vec![], live: None, done: false, error: None, n: 0, dt: 0.0, steps: 0, wall: 0.0, sim_time: 0.0, t_end: 1.0, sym: 0, parts: Default::default(), surface: Default::default(), ref_pos: Default::default() },
        }
    }

    fn running(&self, s: &Snap) -> bool {
        self.job.as_ref().map_or(false, |j| !s.done && !j.paused.load(Ordering::SeqCst))
    }
}

/// Margens seguras [esq, topo, dir, base] em pontos: barra de status, recorte da câmera,
/// barra de navegação e teclado (somente Android; 0 no desktop).
fn system_insets(ctx: &egui::Context) -> [f32; 4] {
    #[cfg(target_os = "android")]
    {
        if let Some(app) = crate::ANDROID_APP.get() {
            let r = app.content_rect();
            if r.right > r.left && r.bottom > r.top {
                let ppp = ctx.pixels_per_point();
                let full = ctx.viewport_rect();
                let (w, h) = (full.width() * ppp, full.height() * ppp);
                let l = (r.left as f32).clamp(0.0, w * 0.3);
                let t = (r.top as f32).clamp(0.0, h * 0.3);
                let rr = (w - r.right as f32).clamp(0.0, w * 0.3);
                let b = (h - r.bottom as f32).clamp(0.0, h * 0.5);
                return [l / ppp, t / ppp, rr / ppp, b / ppp];
            }
        }
    }
    let _ = ctx;
    [0.0; 4]
}

impl eframe::App for PenSim {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t_frame = std::time::Instant::now();
        self.ui_inner(ui);
        PROF_FRAME_US.store(t_frame.elapsed().as_micros() as u64, Ordering::Relaxed);
    }
}

impl PenSim {
    fn ui_inner(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let snap = self.snapshot();
        // A simulação é calculada por inteiro; só depois é reproduzida (ou ao vivo, se escolhido).
        let nf = snap.frames.len();
        if !snap.done {
            if self.live_view && nf > 0 {
                self.view_idx = nf - 1;
            }
        } else if nf > 0 {
            if !self.autoplayed {
                self.autoplayed = true;
                self.playing = true;
                self.play_pos = 0.0;
                if let Some(f) = std::env::var("PENSIM_AT").ok().and_then(|v| v.parse::<f32>().ok()) {
                    self.playing = false;
                    self.play_pos = f * (nf - 1) as f32;
                    self.view_idx = self.play_pos as usize;
                }
            }
            if self.playing {
                let dt = ctx.input(|i| i.stable_dt).min(0.1);
                self.play_pos += dt * PLAY_FPS * self.speed;
                if self.play_pos >= (nf - 1) as f32 {
                    if self.loop_play {
                        self.play_pos = 0.0;
                    } else {
                        self.play_pos = (nf - 1) as f32;
                        self.playing = false;
                    }
                }
                self.view_idx = self.play_pos as usize;
                ctx.request_repaint();
            }
        }
        self.view_idx = self.view_idx.min(snap.frames.len().saturating_sub(1));
        self.refresh_preview();

        // área segura: mantém a interface fora da barra de status, do recorte da câmera e da navegação
        let sys = system_insets(&ctx);
        let inset = [sys[0] + self.extra_inset[0], sys[1] + self.extra_inset[1], sys[2] + self.extra_inset[2], sys[3] + self.extra_inset[3]];
        let blank = egui::Frame::NONE;
        if inset[1] > 0.5 {
            egui::Panel::top("safe_top").exact_size(inset[1]).frame(blank).show_separator_line(false).show(ui, |_| {});
        }
        if inset[3] > 0.5 {
            egui::Panel::bottom("safe_bottom").exact_size(inset[3]).frame(blank).show_separator_line(false).show(ui, |_| {});
        }
        if inset[0] > 0.5 {
            egui::Panel::left("safe_left").exact_size(inset[0]).frame(blank).show_separator_line(false).show(ui, |_| {});
        }
        if inset[2] > 0.5 {
            egui::Panel::right("safe_right").exact_size(inset[2]).frame(blank).show_separator_line(false).show(ui, |_| {});
        }
        let avail = ui.available_size();
        let is_compact = avail.y < 520.0;
        COMPACT.store(is_compact, Ordering::Relaxed);
        if !is_compact {
            egui::Panel::top("title").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PenSim").strong().size(18.0).color(ACCENT));
                    ui.label(RichText::new("· Simulação de impacto balístico 3D").weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("{} threads", pensim_core::threads()));
                    });
                });
            });
        }
        egui::Panel::top("ribbon").show(ui, |ui| self.ribbon(ui, &ctx, &snap));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &snap));

        if is_compact {
            // tela deitada: um único painel lateral com abas, que pode ser ocultado (☰)
            if self.show_side {
                egui::Panel::right("side").default_size((avail.x * 0.34).clamp(260.0, 340.0)).resizable(true).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.sheet_tab, 0, "Navegador");
                        ui.selectable_value(&mut self.sheet_tab, 1, "Propriedades");
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        if self.sheet_tab == 0 { self.browser(ui) } else { self.props(ui, &ctx, &snap) }
                    });
                });
            }
        } else if avail.x > 820.0 {
            egui::Panel::left("browser").default_size(230.0).resizable(true).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.browser(ui));
            });
            egui::Panel::right("props").default_size(340.0).resizable(true).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.props(ui, &ctx, &snap));
            });
        } else {
            let h = avail.y * 0.42;
            egui::Panel::bottom("sheet").default_size(h).resizable(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.sheet_tab, 0, "Navegador");
                    ui.selectable_value(&mut self.sheet_tab, 1, "Propriedades");
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if self.sheet_tab == 0 { self.browser(ui) } else { self.props(ui, &ctx, &snap) }
                });
            });
        }
        egui::CentralPanel::default().show(ui, |ui| self.viewport(ui, &snap));
        if self.show_fps {
            let dt = ctx.input(|i| i.stable_dt).max(1e-4);
            let (fu, vu) = (PROF_FRAME_US.load(Ordering::Relaxed), PROF_VIEW_US.load(Ordering::Relaxed));
            let txt = format!("{:.0} fps · ui {:.1} ms · 3D {:.1} ms", 1.0 / dt, fu as f32 / 1000.0, vu as f32 / 1000.0);
            if std::env::var("PENSIM_PROF").is_ok() {
                eprintln!("{txt}");
            }
            egui::Area::new(egui::Id::new("fps")).anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(96.0, -(inset[3] + 44.0))).show(&ctx, |ui| {
                ui.add(egui::Label::new(RichText::new(txt).monospace().size(11.0).background_color(Color32::from_rgba_unmultiplied(255, 255, 255, 210))).wrap_mode(egui::TextWrapMode::Extend));
            });
        }
    }
}

// ---------------------------------------------------------------- ribbon

fn tbtn(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool, enabled: bool, w: f32) -> egui::Response {
    let (text, size) = if compact() {
        (format!("{icon} {}", label.replace('\n', " ")), Vec2::new(0.0, 30.0))
    } else {
        (format!("{icon}\n{label}"), Vec2::new(w, 48.0))
    };
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text).size(12.0)).min_size(size).selected(selected))
}

fn rbtn(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    tbtn(ui, icon, label, false, true, 62.0)
}

fn group(ui: &mut egui::Ui, name: &str, add: impl FnOnce(&mut egui::Ui)) {
    if compact() {
        ui.horizontal(add);
        ui.separator();
    } else {
        ui.vertical(|ui| {
            ui.horizontal(add);
            ui.label(RichText::new(name).small().weak());
        });
        ui.separator();
    }
}

impl PenSim {
    fn ribbon(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snap) {
        if compact() {
            egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.show_side, "☰").on_hover_text("Mostrar/ocultar painéis").clicked() {
                        self.show_side = !self.show_side;
                    }
                    self.tabs(ui);
                    ui.separator();
                    self.toolbar(ui, ctx, snap);
                });
            });
        } else {
            ui.horizontal(|ui| self.tabs(ui));
            ui.separator();
            egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| self.toolbar(ui, ctx, snap));
        }
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        for (w, l) in [(Ws::Design, "PROJETO"), (Ws::Resolution, "RESOLUÇÃO"), (Ws::Sim, "SIMULAÇÃO"), (Ws::Results, "RESULTADOS")] {
            if ui.selectable_label(self.ws == w, RichText::new(l).strong()).clicked() {
                self.ws = w;
                if w != Ws::Design {
                    self.sketch_mode = false;
                    self.target_mode = false;
                }
            }
        }
    }

    fn load_ammo(&mut self, idx: usize, scale: f64) {
        let keep = (self.cfg.velocity, self.cfg.obliquity_deg, self.cfg.dx, self.cfg.horizon_m, self.cfg.n_frames);
        let a = &AMMO[idx];
        let fresh = idx != self.ammo_idx;
        self.ammo_idx = idx;
        self.ammo_scale = scale;
        self.cfg = a.config(scale);
        if !fresh {
            self.cfg.velocity = keep.0;
            self.cfg.obliquity_deg = keep.1;
            self.cfg.dx = keep.2;
            self.cfg.horizon_m = keep.3;
            self.cfg.n_frames = keep.4;
        }
        self.sel = Sel::Projectile;
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snap) {
        ui.horizontal(|ui| match self.ws {
            Ws::Design => {
                if self.target_mode {
                    group(ui, "CAMADA SELECIONADA", |ui| {
                        let sel = self.sketch.tsel.map(|t| t.0).unwrap_or(0).min(self.cfg.design.layers.len() - 1);
                        let l = &mut self.cfg.design.layers[sel];
                        let (mut g, mut t, mut h, mut w) = (l.gap_before * 1e3, l.thickness * 1e3, l.height * 1e3, l.width * 1e3);
                        ui.label(format!("#{}", sel + 1));
                        ui.add_enabled(sel > 0, egui::DragValue::new(&mut g).prefix("folga ").suffix(" mm").speed(0.1).range(0.0..=80.0).fixed_decimals(1));
                        ui.add(egui::DragValue::new(&mut t).prefix("esp. ").suffix(" mm").speed(0.1).range(1.0..=100.0).fixed_decimals(1));
                        ui.add(egui::DragValue::new(&mut h).prefix("alt. ").suffix(" mm").speed(0.2).range(20.0..=200.0).fixed_decimals(0));
                        ui.add_enabled(!l.round, egui::DragValue::new(&mut w).prefix("larg. ").suffix(" mm").speed(0.2).range(20.0..=200.0).fixed_decimals(0));
                        ui.checkbox(&mut l.round, "Disco");
                        l.gap_before = g / 1e3;
                        l.thickness = t / 1e3;
                        l.height = h / 1e3;
                        l.width = if l.round { h / 1e3 } else { w / 1e3 };
                    });
                    group(ui, "CAMADAS", |ui| {
                        if tbtn(ui, "+", "Camada", false, true, 62.0).clicked() {
                            let mut l = self.cfg.design.layers.last().cloned().unwrap_or_else(|| Preset::Rifle762.config().design.layers[0].clone());
                            l.thickness = 0.008;
                            l.material = LIBRARY[2].clone();
                            self.cfg.design.layers.push(l);
                            self.sketch.tsel = Some((self.cfg.design.layers.len() - 1, true));
                        }
                        let i = self.sketch.tsel.map(|t| t.0).unwrap_or(0);
                        if tbtn(ui, "✕", "Remover", false, self.cfg.design.layers.len() > 1, 62.0).clicked() {
                            self.cfg.design.layers.remove(i.min(self.cfg.design.layers.len() - 1));
                            self.sketch.tsel = None;
                        }
                    });
                    group(ui, "GRADE", |ui| {
                        for s in [0.5, 1.0, 2.0] {
                            ui.selectable_value(&mut self.sketch.snap_mm, s, format!("{s} mm"));
                        }
                    });
                    group(ui, "SAIR", |ui| {
                        if tbtn(ui, "✔", "Concluir", false, true, 62.0).clicked() {
                            self.target_mode = false;
                        }
                    });
                } else if self.sketch_mode {
                    group(ui, "FERRAMENTAS DE ESBOÇO", |ui| {
                        for (t, i, l) in [(Tool::Move, "✥", "Mover"), (Tool::Add, "+", "Ponto"), (Tool::Delete, "✕", "Excluir")] {
                            if tbtn(ui, i, l, self.sketch.tool == t, true, 62.0).clicked() {
                                self.sketch.tool = t;
                            }
                        }
                    });
                    let sel = self.sketch.selected;
                    group(ui, "PONTO SELECIONADO", |ui| {
                        let prof = &mut self.cfg.design.projectile.profile;
                        match sel {
                            Some(i) if i < prof.len() => {
                                let n = prof.len();
                                let lo = if i == 0 { 0.0 } else { prof[i - 1][0] * 1e3 + 0.05 };
                                let hi = if i + 1 == n { 400.0 } else { prof[i + 1][0] * 1e3 - 0.05 };
                                let (mut a, mut r) = (prof[i][0] * 1e3, prof[i][1] * 1e3);
                                ui.add_enabled(i != 0, egui::DragValue::new(&mut a).prefix("a ").suffix(" mm").speed(0.1).range(lo..=hi.max(lo)).fixed_decimals(1));
                                ui.add(egui::DragValue::new(&mut r).prefix("r ").suffix(" mm").speed(0.1).range(0.0..=40.0).fixed_decimals(1));
                                prof[i] = [a / 1e3, r / 1e3];
                            }
                            _ => {
                                ui.weak("toque num ponto");
                            }
                        }
                    });
                    group(ui, "GRADE", |ui| {
                        for s in [0.1, 0.25, 0.5, 1.0] {
                            ui.selectable_value(&mut self.sketch.snap_mm, s, format!("{s} mm"));
                        }
                    });
                    group(ui, "SAIR", |ui| {
                        if tbtn(ui, "✔", "Concluir", false, true, 62.0).clicked() {
                            self.sketch_mode = false;
                        }
                    });
                } else {
                    group(ui, "BIBLIOTECA DE MUNIÇÕES", |ui| {
                        let name = AMMO[self.ammo_idx].name;
                        egui::ComboBox::from_id_salt("ammo").width(if compact() { 190.0 } else { 270.0 }).selected_text(name).show_ui(ui, |ui| {
                            let mut last = "";
                            for (i, a) in AMMO.iter().enumerate() {
                                if a.group != last {
                                    ui.label(RichText::new(a.group).strong().color(ACCENT));
                                    last = a.group;
                                }
                                if ui.selectable_label(i == self.ammo_idx, a.name).clicked() {
                                    self.load_ammo(i, a.scale);
                                }
                            }
                        });
                        let mut sc = self.ammo_scale;
                        if ui.add(egui::DragValue::new(&mut sc).prefix("escala ×").speed(0.01).range(0.03..=3.0).fixed_decimals(2)).on_hover_text("Todas as dimensões × escala (mesma velocidade)").changed() {
                            self.load_ammo(self.ammo_idx, sc);
                        }
                    });
                    group(ui, "ESBOÇO", |ui| {
                        if tbtn(ui, "✎", "Editar\nperfil", false, true, 62.0).clicked() {
                            self.sketch_mode = true;
                            self.target_mode = false;
                            self.sel = Sel::Sketch;
                        }
                    });
                    group(ui, "PONTA", |ui| {
                        for n in Nose::ALL {
                            if tbtn(ui, "◗", n.label(), false, true, 62.0).clicked() {
                                let p = &mut self.cfg.design.projectile;
                                let d = (p.max_radius() * 2.0).max(0.004);
                                let l = p.length().max(0.01);
                                p.profile = n.profile(d, l);
                            }
                        }
                    });
                    group(ui, "ALVO", |ui| {
                        if tbtn(ui, "✎", "Editar\nalvo", false, true, 62.0).clicked() {
                            self.target_mode = true;
                            self.sketch_mode = false;
                            self.sel = Sel::Layer(0);
                        }
                        if tbtn(ui, "+", "Camada", false, true, 62.0).clicked() {
                            let mut l = self.cfg.design.layers.last().cloned().unwrap_or_else(|| Preset::Rifle762.config().design.layers[0].clone());
                            l.thickness = 0.008;
                            l.material = LIBRARY[2].clone();
                            self.cfg.design.layers.push(l);
                            self.sel = Sel::Layer(self.cfg.design.layers.len() - 1);
                        }
                        if let Sel::Layer(i) = self.sel {
                            if tbtn(ui, "✕", "Remover", false, self.cfg.design.layers.len() > 1, 62.0).clicked() {
                                self.cfg.design.layers.remove(i);
                                self.sel = Sel::Layer(0);
                            }
                        }
                    });
                }
            }
            Ws::Resolution => {
                group(ui, "PREDEFINIÇÕES", |ui| {
                    for (l, dx) in RES_PRESETS {
                        let on = (self.cfg.dx - dx).abs() < 1e-9;
                        if tbtn(ui, &format!("{:.1} mm", dx * 1e3), l, on, true, 70.0).clicked() {
                            self.cfg.dx = dx;
                        }
                    }
                });
                group(ui, "AJUSTE FINO", |ui| {
                    let mut mm = self.cfg.dx * 1e3;
                    if ui.add(egui::DragValue::new(&mut mm).range(0.25..=3.0).speed(0.02).suffix(" mm").fixed_decimals(2)).changed() {
                        self.cfg.dx = mm / 1e3;
                    }
                });
                group(ui, "EXECUTAR", |ui| {
                    if tbtn(ui, "▶", "Simular", false, true, 62.0).clicked() {
                        self.start(ctx);
                    }
                });
            }
            Ws::Sim => {
                group(ui, "IMPACTO", |ui| {
                    ui.add(egui::DragValue::new(&mut self.cfg.velocity).range(100.0..=3000.0).suffix(" m/s").speed(5.0));
                    ui.add(egui::DragValue::new(&mut self.cfg.obliquity_deg).range(0.0..=70.0).suffix(" °").speed(0.5));
                });
                group(ui, "EXECUTAR", |ui| {
                    if tbtn(ui, "▶", "Simular", false, true, 62.0).clicked() {
                        self.start(ctx);
                    }
                    self.pause_btn(ui, snap);
                });
                group(ui, "ANÁLISE", |ui| {
                    if tbtn(ui, "◎", "Limite\nbalístico", false, true, 62.0).clicked() {
                        self.ws = Ws::Results;
                        self.res_tab = ResTab::Limit;
                    }
                });
            }
            Ws::Results => {
                group(ui, "CONTORNO", |ui| {
                    egui::ComboBox::from_id_salt("field").width(if compact() { 170.0 } else { 210.0 }).selected_text(self.field.label()).show_ui(ui, |ui| {
                        for f in Field::ALL {
                            ui.selectable_value(&mut self.field, f, f.label());
                        }
                    });
                    ui.checkbox(&mut self.bands, "Bandas");
                });
                group(ui, "VISTA", |ui| {
                    if tbtn(ui, "◧", if self.section { "Corte\nligado" } else { "Corte\ndesl." }, self.section, true, 62.0).clicked() {
                        self.section = !self.section;
                    }
                });
                group(ui, "EXECUÇÃO", |ui| {
                    self.pause_btn(ui, snap);
                    ui.checkbox(&mut self.live_view, "Ao vivo");
                });
                group(ui, "REPETIÇÃO", |ui| {
                    let nf = snap.frames.len();
                    let can = snap.done && nf > 1;
                    if tbtn(ui, if self.playing { "||" } else { "▶" }, if self.playing { "Pausar" } else { "Reproduzir" }, false, can, 70.0).clicked() {
                        if self.playing {
                            self.playing = false;
                        } else {
                            if self.play_pos >= (nf - 1) as f32 {
                                self.play_pos = 0.0;
                            }
                            self.playing = true;
                        }
                    }
                    if tbtn(ui, "|◀", "Início", false, can, 56.0).clicked() {
                        self.play_pos = 0.0;
                        self.view_idx = 0;
                    }
                    for sp in [0.1f32, 0.25, 0.5, 1.0, 2.0, 4.0] {
                        if ui.selectable_label((self.speed - sp).abs() < 1e-6, format!("{sp}×")).clicked() {
                            self.speed = sp;
                        }
                    }
                    ui.checkbox(&mut self.loop_play, "Laço");
                });
            }
        });
    }

    fn pause_btn(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        if let Some(j) = &self.job {
            if !snap.done {
                let paused = j.paused.load(Ordering::SeqCst);
                if rbtn(ui, if paused { "▶" } else { "||" }, if paused { "Continuar" } else { "Pausar" }).clicked() {
                    j.paused.store(!paused, Ordering::SeqCst);
                }
            }
        }
    }

    // ------------------------------------------------------------ navegador

    fn browser(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("NAVEGADOR").small().weak());
        let mut pick = |this: &mut PenSim, ui: &mut egui::Ui, sel: Sel, text: String| {
            if ui.selectable_label(this.sel == sel, text).clicked() {
                this.sel = sel;
                this.sheet_tab = 1;
                if sel == Sel::Sketch {
                    this.ws = Ws::Design;
                }
            }
        };
        egui::CollapsingHeader::new("Documento").default_open(true).show(ui, |ui| {
            ui.weak("▫ Origem (planos XY · XZ · YZ)");
            ui.weak("  X = direção de voo · Z = vertical");
            egui::CollapsingHeader::new("Projétil").default_open(true).show(ui, |ui| {
                pick(self, ui, Sel::Sketch, "✎ Esboço 1 · perfil".into());
                let m = self.cfg.design.projectile.material.name;
                pick(self, ui, Sel::Projectile, format!("⟳ Revolução 1 · {m}"));
            });
            egui::CollapsingHeader::new("Alvo").default_open(true).show(ui, |ui| {
                for i in 0..self.cfg.design.layers.len() {
                    let l = &self.cfg.design.layers[i];
                    let t = format!("▭ Extrusão {} · {} · {:.0} mm", i + 1, l.material.name, l.thickness * 1e3);
                    pick(self, ui, Sel::Layer(i), t);
                }
            });
            egui::CollapsingHeader::new("Ajustes de tela").default_open(false).show(ui, |ui| {
                ui.checkbox(&mut self.show_fps, "Mostrar FPS e tempos");
                ui.weak("Se algum botão do topo, lados ou base não responder (barra de status, câmera ou navegação do Android), aumente a margem.");
                ui.add(egui::Slider::new(&mut self.extra_inset[1], 0.0..=80.0).text("topo"));
                ui.add(egui::Slider::new(&mut self.extra_inset[3], 0.0..=80.0).text("base"));
                ui.add(egui::Slider::new(&mut self.extra_inset[0], 0.0..=80.0).text("esquerda"));
                ui.add(egui::Slider::new(&mut self.extra_inset[2], 0.0..=80.0).text("direita"));
            });
            egui::CollapsingHeader::new("Análise").default_open(true).show(ui, |ui| {
                pick(self, ui, Sel::Conditions, "⚙ Condições de impacto".into());
                pick(self, ui, Sel::Mesh, "▦ Malha (nós)".into());
            });
        });
    }

    // --------------------------------------------------------- propriedades

    fn props(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snap) {
        if self.ws == Ws::Results {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.res_tab, ResTab::Summary, "Resumo");
                ui.selectable_value(&mut self.res_tab, ResTab::Charts, "Gráficos");
                ui.selectable_value(&mut self.res_tab, ResTab::Limit, "Limite");
            });
            ui.separator();
            match self.res_tab {
                ResTab::Summary => self.ui_summary(ui, snap),
                ResTab::Charts => self.ui_charts(ui, snap),
                ResTab::Limit => self.ui_limit(ui, ctx),
            }
            return;
        }
        if self.ws == Ws::Resolution {
            self.resolution_panel(ui, ctx);
            return;
        }
        ui.label(RichText::new("PROPRIEDADES").small().weak());
        match self.sel {
            Sel::Sketch => {
                ui.heading("Esboço 1 · perfil");
                ui.label("Contorno externo do projétil no plano a–r, revolvido em torno do eixo. A ponta fica em a = 0.");
                if ui.button("✎ Editar esboço").clicked() {
                    self.sketch_mode = true;
                    self.ws = Ws::Design;
                }
                let p = &self.cfg.design.projectile;
                ui.label(format!("Pontos: {} · L = {:.1} mm · ⌀ = {:.1} mm", p.profile.len(), p.length() * 1e3, p.max_radius() * 2e3));
                ui.separator();
                ui.label("Gerar perfil paramétrico:");
                let (mut d, mut l) = (p.max_radius() * 2e3, p.length() * 1e3);
                ui.add(egui::Slider::new(&mut d, 3.0..=30.0).text("⌀ mm"));
                ui.add(egui::Slider::new(&mut l, 5.0..=150.0).text("L mm"));
                ui.horizontal_wrapped(|ui| {
                    for n in Nose::ALL {
                        if ui.button(n.label()).clicked() {
                            self.cfg.design.projectile.profile = n.profile(d / 1e3, l / 1e3);
                        }
                    }
                });
            }
            Sel::Projectile => {
                let am: &Ammo = &AMMO[self.ammo_idx];
                ui.heading("Revolução 1 · projétil");
                ui.label(RichText::new(am.name).strong());
                ui.weak(am.note);
                egui::Grid::new("ammoinfo").num_columns(2).show(ui, |ui| {
                    ui.label("Real");
                    ui.label(format!("⌀ {:.1} × {:.0} mm · {:.1} g · {:.0} m/s", am.caliber_mm, am.length_mm, am.mass_g, am.velocity));
                    ui.end_row();
                    ui.label("Modelo");
                    ui.label(format!("escala ×{:.2} · {:.1} g (massa × s³)", self.ammo_scale, am.scaled_mass_g(self.ammo_scale)));
                    ui.end_row();
                });
                ui.weak("Valores aproximados. Camisas finas (cobre/latão) ficam abaixo da resolução da malha e não são modeladas.");
                ui.separator();
                ui.label("Material do corpo:");
                mat_combo(ui, "pm", &mut self.cfg.design.projectile.material);
                let pr = &mut self.cfg.design.projectile;
                match pr.core.as_mut() {
                    Some(c) => {
                        ui.label("Material do núcleo / enchimento:");
                        mat_combo(ui, "pc", &mut c.material);
                        if ui.button("✕ Remover núcleo").clicked() {
                            pr.core = None;
                        }
                    }
                    None => {
                        if ui.button("+ Adicionar núcleo").clicked() {
                            let (l, r) = (pr.length(), pr.max_radius());
                            pr.core = Some(Core { profile: vec![[0.1 * l, 0.55 * r], [0.8 * l, 0.55 * r]], material: LIBRARY[11].clone() });
                        }
                    }
                }
                let p = &self.cfg.design.projectile;
                ui.label(format!("L = {:.1} mm · ⌀ = {:.1} mm", p.length() * 1e3, p.max_radius() * 2e3));
            }
            Sel::Layer(i) => {
                let n = self.cfg.design.layers.len();
                if i >= n {
                    self.sel = Sel::Layer(0);
                    return;
                }
                ui.heading(format!("Extrusão {}", i + 1));
                if ui.button("✎ Editar no esboço do alvo").clicked() {
                    self.target_mode = true;
                    self.sketch_mode = false;
                    self.ws = Ws::Design;
                    self.sketch.tsel = Some((i, true));
                }
                let l: &mut Layer = &mut self.cfg.design.layers[i];
                mat_combo(ui, "lm", &mut l.material);
                let (mut t, mut g, mut w, mut h) = (l.thickness * 1e3, l.gap_before * 1e3, l.width * 1e3, l.height * 1e3);
                ui.add(egui::Slider::new(&mut t, 1.0..=80.0).text("espessura mm"));
                ui.add(egui::Slider::new(&mut g, 0.0..=40.0).text("folga antes mm"));
                ui.add(egui::Slider::new(&mut w, 20.0..=150.0).text(if l.round { "diâmetro mm" } else { "largura mm" }));
                if !l.round {
                    ui.add(egui::Slider::new(&mut h, 20.0..=150.0).text("altura mm"));
                }
                ui.checkbox(&mut l.round, "Seção circular (disco)");
                l.thickness = t / 1e3;
                l.gap_before = g / 1e3;
                l.width = w / 1e3;
                l.height = h / 1e3;
                ui.horizontal(|ui| {
                    if ui.add_enabled(i > 0, egui::Button::new("▲ Subir")).clicked() {
                        self.cfg.design.layers.swap(i, i - 1);
                        self.sel = Sel::Layer(i - 1);
                    }
                    if ui.add_enabled(i + 1 < n, egui::Button::new("▼ Descer")).clicked() {
                        self.cfg.design.layers.swap(i, i + 1);
                        self.sel = Sel::Layer(i + 1);
                    }
                    if ui.add_enabled(n > 1, egui::Button::new("✕ Remover")).clicked() {
                        self.cfg.design.layers.remove(i);
                        self.sel = Sel::Layer(0);
                    }
                });
            }
            Sel::Conditions => {
                ui.heading("Condições de impacto");
                ui.add(egui::Slider::new(&mut self.cfg.velocity, 100.0..=3000.0).text("velocidade m/s"));
                ui.add(egui::Slider::new(&mut self.cfg.obliquity_deg, 0.0..=70.0).text("obliquidade °"));
                ui.add(egui::Slider::new(&mut self.cfg.time_factor, 0.5..=3.0).text("tempo final ×"));
                ui.checkbox(&mut self.cfg.clamp_edges, "Engastar bordas do alvo (apoio fixo)");
                ui.weak("Obliquidade no plano X–Z, medida a partir da normal da placa.");
            }
            Sel::Mesh => {
                self.resolution_panel(ui, ctx);
                return;
            }
        }
        if self.ws != Ws::Design || !matches!(self.sel, Sel::Sketch) {
            ui.separator();
            let stale = self.started_key.as_ref() != Some(&self.key());
            if ui.add_sized([ui.available_width(), 42.0], egui::Button::new(RichText::new("▶ Simular").strong().color(Color32::WHITE)).fill(ACCENT)).clicked() {
                self.start(ctx);
            }
            if stale && self.started_key.is_some() {
                ui.colored_label(Color32::from_rgb(190, 120, 0), "Modelo alterado desde a última simulação.");
            }
        }
    }

    fn resolution_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Resolução da simulação");
        ui.label("Quanto menor o espaçamento entre nós, mais fiel é a fratura e a geometria do projétil, mas o custo cresce rápido.");
        ui.add_space(4.0);
        ui.label("Predefinições:");
        ui.horizontal_wrapped(|ui| {
            for (l, dx) in RES_PRESETS {
                ui.selectable_value(&mut self.cfg.dx, dx, format!("{l} · {:.1} mm", dx * 1e3));
            }
        });
        let mut mm = self.cfg.dx * 1e3;
        if ui.add(egui::Slider::new(&mut mm, 0.25..=3.0).logarithmic(true).text("espaçamento mm")).changed() {
            self.cfg.dx = mm / 1e3;
        }
        ui.label("Horizonte (× espaçamento): 2 = rápido · 3 = mais preciso");
        ui.horizontal(|ui| {
            for h in [2.0, 2.5, 3.0] {
                ui.selectable_value(&mut self.cfg.horizon_m, h, format!("{h}"));
            }
        });
        let mut secs = (self.cfg.n_frames as f32 / PLAY_FPS).clamp(1.0, 16.0);
        if ui.add(egui::Slider::new(&mut secs, 1.0..=16.0).text("duração da repetição s")).changed() {
            self.cfg.n_frames = (secs * PLAY_FPS).round() as usize;
        }
        ui.weak(format!("Repetição a {PLAY_FPS:.0} fps em 1× · até {} quadros gravados (mais quadros = mais memória).", self.cfg.n_frames));
        ui.separator();

        let n = self.estimate_nodes();
        let bonds = self.cfg.bonds_per_node();
        let mem = self.cfg.estimate_memory_mb();
        let free = pensim_core::available_memory_mb();
        let base = Preset::Rifle762.config();
        let base_n = 30000.0_f64;
        let rel = (n / base_n) * (self.cfg.horizon_m / base.horizon_m).powi(3) * (base.dx / self.cfg.dx);
        let d = self.cfg.design.projectile.max_radius() * 2.0;
        let thin = self.cfg.design.layers.iter().map(|l| l.thickness).fold(f64::MAX, f64::min);
        let across_d = d / self.cfg.dx;
        let across_t = thin / self.cfg.dx;
        egui::Grid::new("resgrid").num_columns(2).striped(true).show(ui, |ui| {
            ui.label("Nós estimados");
            ui.label(format!("≈ {:.0}", n));
            ui.end_row();
            ui.label("Ligações por nó");
            ui.label(format!("≈ {:.0}", bonds));
            ui.end_row();
            ui.label("Memória estimada / livre");
            ui.label(match free {
                Some(f) => format!("≈ {:.0} MB / {:.0} MB", mem, f),
                None => format!("≈ {:.0} MB", mem),
            });
            ui.end_row();
            ui.label("Nós no diâmetro do projétil");
            ui.label(format!("{:.1}", across_d));
            ui.end_row();
            ui.label("Nós na camada mais fina");
            ui.label(format!("{:.1}", across_t));
            ui.end_row();
            ui.label("Custo relativo");
            ui.label(format!("× {:.1} (vs. padrão ≈ 1,0)", rel));
            ui.end_row();
        });
        let warn = Color32::from_rgb(190, 120, 0);
        let bad = Color32::from_rgb(200, 50, 40);
        if across_d < 4.0 {
            ui.colored_label(warn, "Projétil com menos de 4 nós de diâmetro: geometria grosseira. Reduza o espaçamento.");
        }
        if across_t < 3.0 {
            ui.colored_label(warn, "Camada com menos de 3 nós de espessura: perfuração pouco confiável.");
        }
        if free.map_or(false, |f| mem > 0.85 * f) {
            ui.colored_label(bad, "Não cabe na memória livre: a simulação será recusada. Aumente o espaçamento.");
        } else if mem > 1500.0 || n > 120_000.0 {
            ui.colored_label(bad, "Muito pesado: pode levar muito tempo e fechar o app por falta de memória. Você decide.");
        } else if mem > 600.0 || n > 50_000.0 {
            ui.colored_label(warn, "Pesado para celular: a simulação pode levar vários minutos.");
        } else {
            ui.colored_label(Color32::from_rgb(30, 140, 80), "Adequado para celular.");
        }
        ui.weak("O viewport mostra a malha atual: ajuste e veja os nós mudarem antes de simular.");
        ui.separator();
        if ui.add_sized([ui.available_width(), 42.0], egui::Button::new(RichText::new("▶ Simular com esta resolução").strong().color(Color32::WHITE)).fill(ACCENT)).clicked() {
            self.start(ctx);
        }
    }

    fn progress_card(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, s: &Snap) {
        let p = (s.sim_time / s.t_end.max(1e-12)).clamp(0.0, 1.0);
        let elapsed = self.started_at.elapsed().as_secs_f64();
        let eta = if p > 0.03 { Some(elapsed * (1.0 - p) / p) } else { None };
        let card = Rect::from_center_size(rect.center(), Vec2::new(rect.width().min(380.0) - 20.0, 150.0));
        painter.rect_filled(card, 10.0, Color32::from_rgba_unmultiplied(255, 255, 255, 235));
        painter.rect_stroke(card, 10.0, Stroke::new(1.0, Color32::from_gray(190)), egui::StrokeKind::Inside);
        painter.text(card.center_top() + Vec2::new(0.0, 14.0), Align2::CENTER_TOP, "Calculando a simulação…", FontId::proportional(17.0), Color32::from_gray(30));
        let bar = Rect::from_center_size(card.center() + Vec2::new(0.0, -4.0), Vec2::new(card.width() - 40.0, 16.0));
        painter.rect_filled(bar, 8.0, Color32::from_gray(225));
        painter.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * p as f32, bar.height())), 8.0, ACCENT);
        let txt = format!(
            "{:.0}%  ·  {:.0} s decorridos{}  ·  {} nós",
            p * 100.0,
            elapsed,
            eta.map_or(String::new(), |e| format!("  ·  ~{:.0} s restantes", e)),
            s.n
        );
        painter.text(card.center() + Vec2::new(0.0, 22.0), Align2::CENTER_TOP, txt, FontId::proportional(12.0), Color32::from_gray(70));
        painter.text(card.center() + Vec2::new(0.0, 42.0), Align2::CENTER_TOP, "A reprodução começa sozinha quando terminar.", FontId::proportional(11.0), Color32::from_gray(110));
        let btn = Rect::from_center_size(card.center_bottom() + Vec2::new(0.0, -2.0), Vec2::ZERO);
        let _ = btn;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    fn ui_summary(&mut self, ui: &mut egui::Ui, s: &Snap) {
        if let Some(e) = &s.error {
            ui.colored_label(Color32::RED, e);
            return;
        }
        let Some(o) = s.live.as_ref() else {
            ui.label(if self.job.is_some() { "Montando malha e ligações…" } else { "Nenhuma simulação ainda. Use ▶ Simular." });
            return;
        };
        let (txt, col) = match (s.done, o.kind) {
            (false, _) => ("Em andamento…", Color32::GRAY),
            (_, OutcomeKind::Perforation) => (o.kind.label(), Color32::from_rgb(200, 50, 40)),
            (_, OutcomeKind::PartialPenetration) => (o.kind.label(), Color32::from_rgb(200, 140, 0)),
            (_, OutcomeKind::Rejected) => (o.kind.label(), Color32::from_rgb(30, 150, 80)),
        };
        ui.label(RichText::new(txt).size(20.0).strong().color(col));
        egui::Grid::new("res").num_columns(2).striped(true).show(ui, |ui| {
            let mut row = |a: &str, b: String| {
                ui.label(a);
                ui.label(b);
                ui.end_row();
            };
            row("Velocidade de impacto", format!("{:.0} m/s", self.cfg.velocity));
            row("Velocidade residual (CM)", format!("{:.0} m/s", o.v_residual));
            row("Energia absorvida", format!("{:.1} %", o.energy_absorbed_pct));
            row("Projétil atrás do alvo", format!("{:.0} %", o.proj_behind_pct));
            row("Alvo ejetado (tampão)", format!("{:.1} %", o.target_ejected_pct));
            row("Erro do balanço de energia", format!("{:+.2} %", o.energy_error_pct));
            row("Nós / passo dt", format!("{} / {:.1} ns", s.n, s.dt * 1e9));
            row("Passos / tempo de parede", format!("{} / {:.1} s", s.steps, s.wall));
            row("Quadros gravados", format!("{}", s.frames.len()));
        });
        ui.add_space(6.0);
        ui.weak("Peridinâmica 3D (ν = 1/4) com plasticidade, amolecimento térmico e fratura por estiramento. Resultados qualitativos; não substituem ensaio nem um código validado.");
    }

    fn ui_charts(&mut self, ui: &mut egui::Ui, s: &Snap) {
        if s.frames.len() < 2 {
            ui.label("Sem dados ainda.");
            return;
        }
        let fr = &s.frames;
        let mk = |name: &'static str, color, g: &dyn Fn(&Frame) -> f64| Series { name, color, markers: false, pts: fr.iter().map(|f| (f.time * 1e6, g(f))).collect() };
        line_plot(ui, 190.0, "Balanço de energia", "t [µs]", "J", &[
            mk("Cinética proj.", Color32::from_rgb(30, 110, 220), &|f| f.ke_proj),
            mk("Cinética alvo", Color32::from_rgb(30, 160, 90), &|f| f.ke_target),
            mk("Plástica", Color32::from_rgb(240, 130, 20), &|f| f.e_plastic),
            mk("Fratura", Color32::from_rgb(210, 50, 50), &|f| f.e_fracture),
            mk("Elástica", Color32::from_rgb(140, 90, 210), &|f| f.e_elastic),
            mk("Contato", Color32::GRAY, &|f| f.e_contact),
        ]);
        ui.add_space(8.0);
        line_plot(ui, 170.0, "Velocidade do projétil (vx do CM)", "t [µs]", "m/s", &[mk("vx", Color32::from_rgb(30, 110, 220), &|f| f.vx_proj)]);
    }

    fn ui_limit(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label("Varre velocidades em paralelo (várias simulações 3D simultâneas) e refina por bissecção para estimar o limite balístico V50. Pode levar minutos.");
        ui.add(egui::Slider::new(&mut self.sweep_range.0, 100.0..=2500.0).text("v mín m/s"));
        ui.add(egui::Slider::new(&mut self.sweep_range.1, 200.0..=3000.0).text("v máx m/s"));
        ui.add(egui::Slider::new(&mut self.sweep_n, 3..=8).text("pontos"));
        let busy = self.sweep.as_ref().map_or(false, |s| !s.state.lock().unwrap().finished);
        if ui.add_enabled(!busy, egui::Button::new("◎ Calcular limite balístico")).clicked() && self.sweep_range.0 < self.sweep_range.1 {
            self.sweep = Some(SweepJob::spawn(self.cfg.clone(), self.sweep_range.0, self.sweep_range.1, self.sweep_n, 2, ctx.clone()));
        }
        if let Some(s) = &self.sweep {
            let g = s.state.lock().unwrap();
            if !g.finished {
                ui.add(egui::ProgressBar::new(g.done_runs as f32 / g.total_runs.max(1) as f32).text(format!("{}/{} simulações", g.done_runs, g.total_runs)));
            } else {
                match g.v50 {
                    Some(v) => ui.label(RichText::new(format!("V50 ≈ {v:.0} m/s")).size(20.0).strong()),
                    None => ui.colored_label(Color32::from_rgb(190, 120, 0), "A faixa não contém a transição – ajuste v mín/máx."),
                };
                let perf: Vec<(f64, f64)> = g.points.iter().filter(|p| p.outcome.kind == OutcomeKind::Perforation).map(|p| (p.velocity, p.outcome.v_residual)).collect();
                let stop: Vec<(f64, f64)> = g.points.iter().filter(|p| p.outcome.kind != OutcomeKind::Perforation).map(|p| (p.velocity, 0.0)).collect();
                line_plot(ui, 190.0, "Velocidade residual × impacto", "v impacto [m/s]", "v residual [m/s]", &[
                    Series { name: "perfurou", color: Color32::from_rgb(210, 50, 40), pts: perf, markers: true },
                    Series { name: "retido/rejeitado", color: Color32::from_rgb(30, 150, 80), pts: stop, markers: true },
                ]);
            }
        }
    }

    // ------------------------------------------------------------ status

    fn status_bar(&mut self, ui: &mut egui::Ui, s: &Snap) {
        if self.ws == Ws::Results && self.job.is_some() && !s.done && s.error.is_none() {
            ui.horizontal(|ui| {
                let p = (s.sim_time / s.t_end.max(1e-12)).clamp(0.0, 1.0) as f32;
                ui.add(egui::ProgressBar::new(p).desired_width((ui.available_width() - 130.0).max(80.0)).text(format!("calculando… {:.0}%", p * 100.0)));
                ui.label(if self.running(s) { "● em execução" } else { "pausado" });
            });
        } else if self.ws == Ws::Results && s.frames.len() > 1 {
            ui.horizontal(|ui| {
                let nf = s.frames.len();
                if ui.button(if self.playing { "||" } else { "▶" }).clicked() {
                    if self.playing {
                        self.playing = false;
                    } else {
                        if self.play_pos >= (nf - 1) as f32 {
                            self.play_pos = 0.0;
                        }
                        self.playing = true;
                    }
                }
                let t = s.frames[self.view_idx.min(nf - 1)].time * 1e6;
                ui.monospace(format!("t = {t:7.1} µs · {}×", self.speed));
                let mut i = self.view_idx;
                let w = (ui.available_width() - 20.0).max(80.0);
                if ui.add_sized([w, 26.0], egui::Slider::new(&mut i, 0..=nf - 1).show_value(false)).changed() {
                    self.view_idx = i;
                    self.play_pos = i as f32;
                    self.playing = false;
                }
            });
        } else {
            // linha do tempo de recursos (estilo Fusion)
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("LINHA DO TEMPO").small().weak());
                let mut chip = |ui: &mut egui::Ui, sel: Sel, t: String| {
                    if ui.add(egui::Button::new(RichText::new(t).small()).selected(self.sel == sel)).clicked() {
                        self.sel = sel;
                        self.sheet_tab = 1;
                    }
                };
                chip(ui, Sel::Sketch, "✎ Esboço 1".into());
                chip(ui, Sel::Projectile, "⟳ Revolução 1".into());
                for i in 0..self.cfg.design.layers.len() {
                    chip(ui, Sel::Layer(i), format!("▭ Extrusão {}", i + 1));
                }
                chip(ui, Sel::Conditions, "⚙ Impacto".into());
            });
        }
    }

    // --------------------------------------------------------- viewport 3D

    fn viewport(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        let t = std::time::Instant::now();
        self.viewport_inner(ui, snap);
        PROF_VIEW_US.store(t.elapsed().as_micros() as u64, Ordering::Relaxed);
    }

    fn viewport_inner(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        if self.target_mode && self.ws == Ws::Design {
            let colors: Vec<Color32> = self.cfg.design.layers.iter().map(|l| mat_color(l.material.name)).collect();
            let proj = self.cfg.design.projectile.profile.clone();
            sketch::show_target(ui, &mut self.sketch, &mut self.cfg.design.layers, &proj, &colors);
            return;
        }
        if self.sketch_mode && self.ws == Ws::Design {
            let fill = mat_color(self.cfg.design.projectile.material.name);
            let core = self.cfg.design.projectile.core.as_ref().map(|c| (c.profile.clone(), mat_color(c.material.name)));
            sketch::show(ui, &mut self.sketch, &mut self.cfg.design.projectile.profile, fill, core.as_ref().map(|(p, c)| (p.as_slice(), *c)));
            return;
        }
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        view3d::background(&painter, rect, false);
        view3d::input(ui, &resp, &mut self.cam);
        if resp.double_clicked() {
            self.cam.zoom = 1.0;
            self.cam.pan = Vec2::ZERO;
        }
        let results = self.ws == Ws::Results && !snap.frames.is_empty() && (snap.done || self.live_view);
        let dx = self.cfg.dx as f32;
        let cut = 0.25 * dx;
        // o corte remove a metade voltada para a câmera, para mostrar a face interna (plano y = 0)
        let eye_y = self.cam.pitch.cos() * self.cam.yaw.sin();
        let keep_pos = eye_y <= 0.0;
        let in_section = move |ry: f32| if keep_pos { ry >= -cut } else { ry <= cut };
        let near_face = move |ry: f32| if keep_pos { ry <= -cut + 1.1 * dx } else { ry >= cut - 1.1 * dx };
        let p = &self.preview;
        let view = View { cam: self.cam, rect, center: p.center, radius: p.radius };

        if let Some(e) = snap.error.as_ref().or(p.err.as_ref()) {
            painter.text(rect.center(), Align2::CENTER_CENTER, e, FontId::proportional(15.0), Color32::from_rgb(190, 40, 40));
        } else if results {
            let f = &snap.frames[self.view_idx];
            let n = f.pos.len();
            // a simulação calcula só 1/2 ou 1/4 do modelo (simetria); as demais cópias são espelhadas aqui
            let masks: Vec<u8> = match snap.sym {
                0 => vec![0],
                1 => vec![0, 1],
                2 => vec![0, 2],
                _ => vec![0, 1, 2, 3],
            };
            let flip = |q: [f32; 3], m: u8| [q[0], if m & 1 != 0 { -q[1] } else { q[1] }, if m & 2 != 0 { -q[2] } else { q[2] }];
            let mut ext_pos: Vec<[f32; 3]> = Vec::with_capacity(n * masks.len());
            for &m in &masks {
                ext_pos.extend(f.pos.iter().map(|&q| flip(q, m)));
            }
            let mut idx: Vec<u32> = Vec::new();
            for (mi, &m) in masks.iter().enumerate() {
                for i in 0..n {
                    let ry = flip(snap.ref_pos[i], m)[1];
                    if self.section && !in_section(ry) {
                        continue;
                    }
                    if snap.surface[i] || (self.section && near_face(ry)) || f.damage[i] > 0.4 {
                        idx.push((mi * n + i) as u32);
                    }
                }
            }
            let fi = Field::ALL.iter().position(|x| *x == self.field).unwrap();
            let sc_f = self.field.scale();
            if let Some(data) = self.field.data(f) {
                let (mut mx, mut mn) = (f32::MIN, f32::MAX);
                for &i in &idx {
                    let v = data[i as usize % n] * sc_f;
                    mx = mx.max(v);
                    mn = mn.min(v);
                }
                let r = &mut self.range[fi];
                r.1 = r.1.max(mx);
                r.0 = match self.field {
                    Field::Pressure => r.0.min(mn),
                    Field::Temp => 293.0,
                    _ => 0.0,
                };
                let (lo, hi) = (r.0, if r.1 > r.0 { r.1 } else { r.0 + 1.0 });
                let bands = self.bands;
                view3d::render(&painter, &view, dx, &ext_pos, &idx, false, &|i| rainbow((data[i % n] * sc_f - lo) / (hi - lo), bands));
                legend(&painter, rect, self.field, lo, hi);
            } else {
                let parts = snap.parts.clone();
                let mats: Vec<Color32> = self.cfg.design.materials().iter().map(|m| mat_color(m.name)).collect();
                view3d::render(&painter, &view, dx, &ext_pos, &idx, true, &|i| mats[(parts[i % n] as usize).min(mats.len() - 1)]);
            }
            painter.text(rect.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, format!("t = {:.1} µs", f.time * 1e6), FontId::monospace(15.0), Color32::from_gray(30));
        } else {
            let mats: Vec<Color32> = self.cfg.design.materials().iter().map(|m| mat_color(m.name)).collect();
            let idx: Vec<u32> = if self.section { p.idx.iter().cloned().filter(|&i| in_section(p.pos[i as usize][1])).collect() } else { p.idx.clone() };
            view3d::render(&painter, &view, dx, &p.pos, &idx, true, &|i| mats[(p.part[i] as usize).min(mats.len() - 1)]);
            painter.text(rect.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, format!("Modelo · {} nós", p.pos.len()), FontId::proportional(13.0), Color32::from_gray(60));
        }
        view3d::triad(&painter, &view);
        if self.ws == Ws::Results && self.job.is_some() && !snap.done && snap.error.is_none() && !results {
            self.progress_card(ui, &painter, rect, snap);
        }

        // ViewCube simplificado
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("viewcube")).fixed_pos(rect.right_top() + Vec2::new(-246.0, 8.0)).show(&ctx, |ui| {
            ui.horizontal(|ui| {
                for (l, c) in [("ISO", Camera::iso()), ("Lado", Camera::front()), ("Topo", Camera::top()), ("Frente", Camera::right())] {
                    if ui.small_button(l).clicked() {
                        self.cam = c;
                    }
                }
                if ui.small_button(if self.section { "Corte ✔" } else { "Corte" }).clicked() {
                    self.section = !self.section;
                }
            });
        });
    }
}

fn legend(painter: &egui::Painter, rect: Rect, field: Field, lo: f32, hi: f32) {
    let lx = rect.right() - 86.0;
    let bars = 12;
    let bh = ((rect.height() - 120.0) / bars as f32).clamp(8.0, 22.0);
    let ink = Color32::from_gray(25);
    painter.text(Pos2::new(lx, rect.top() + 40.0), Align2::LEFT_TOP, format!("{} [{}]", field.short(), field.unit()), FontId::proportional(11.0), ink);
    for k in 0..bars {
        let t = 1.0 - (k as f32 + 0.5) / bars as f32;
        let y = rect.top() + 58.0 + k as f32 * bh;
        painter.rect_filled(Rect::from_min_size(Pos2::new(lx, y), Vec2::new(14.0, bh)), 0.0, rainbow(t, false));
        if k % 3 == 0 || k == bars - 1 {
            let v = lo + (hi - lo) * (1.0 - k as f32 / (bars - 1) as f32);
            painter.text(Pos2::new(lx + 18.0, y + bh * 0.5), Align2::LEFT_CENTER, fmt_v(v), FontId::proportional(10.0), ink);
        }
    }
    painter.rect_stroke(Rect::from_min_size(Pos2::new(lx, rect.top() + 58.0), Vec2::new(14.0, bh * bars as f32)), 0.0, Stroke::new(1.0, ink), egui::StrokeKind::Outside);
}

fn mat_combo(ui: &mut egui::Ui, id: &str, m: &mut pensim_core::Material) {
    egui::ComboBox::from_id_salt(id).selected_text(m.name).width(240.0).show_ui(ui, |ui| {
        for lm in LIBRARY {
            if ui.selectable_label(lm.name == m.name, lm.name).clicked() {
                *m = lm.clone();
            }
        }
    });
    ui.weak(format!("ρ={:.0} kg/m³ · E={:.0} GPa · σy={:.3} GPa", m.rho, m.young / 1e9, m.yield_strength / 1e9));
}

fn fmt_v(v: f32) -> String {
    if v.abs() >= 100.0 { format!("{v:.0}") } else if v.abs() >= 1.0 { format!("{v:.1}") } else { format!("{v:.2}") }
}
