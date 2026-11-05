use crate::plot::{line_plot, Series};
use crate::sketch::{self, SketchEditor, Tool};
use crate::view3d::{self, Camera, View};
use crate::worker::{Job, SweepJob};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use pensim_core::{Config, Design, Frame, Layer, Model, Nose, Outcome, OutcomeKind, Preset, LIBRARY};
use std::sync::atomic::Ordering;
use std::sync::Arc;

const RES_PRESETS: [(&str, f64); 5] = [("Rascunho", 0.0025), ("Rápida", 0.002), ("Média", 0.0015), ("Fina", 0.001), ("Muito fina", 0.0008)];
/// duração (s) da reprodução completa em velocidade 1×
const PLAY_SECONDS: f32 = 6.0;
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
    parts: Arc<Vec<u8>>,
    surface: Arc<Vec<bool>>,
    ref_pos: Arc<Vec<[f32; 3]>>,
}

pub struct PenSim {
    cfg: Config,
    preset: Preset,
    ws: Ws,
    sel: Sel,
    res_tab: ResTab,
    sheet_tab: u8,
    sketch_mode: bool,
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
        let mut v = egui::Visuals::light();
        v.selection.bg_fill = ACCENT;
        v.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals(v);
        cc.egui_ctx.global_style_mut(|s| {
            s.spacing.interact_size.y = 32.0;
            s.spacing.item_spacing = Vec2::new(8.0, 6.0);
            s.spacing.slider_width = 150.0;
        });
        let cfg = Config::default();
        PenSim {
            cfg,
            preset: Preset::Rifle762,
            ws: Ws::Design,
            sel: Sel::Projectile,
            res_tab: ResTab::Summary,
            sheet_tab: 0,
            sketch_mode: false,
            sketch: SketchEditor::default(),
            cam: Camera::front(),
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
        }
    }

    fn key(&self) -> Key {
        let c = &self.cfg;
        (c.design.clone(), [c.dx, c.horizon_m, c.velocity, c.obliquity_deg, c.time_factor], c.clamp_edges)
    }

    fn estimate_nodes(&self) -> f64 {
        let dx3 = self.cfg.dx.powi(3);
        let layers: f64 = self.cfg.design.layers.iter().map(|l| {
            let a = if l.round { std::f64::consts::PI / 4.0 * l.width * l.width } else { l.width * l.height };
            a * l.thickness / dx3
        }).sum();
        let p = &self.cfg.design.projectile;
        let rm = p.max_radius();
        layers + std::f64::consts::PI * rm * rm * p.length() * 0.7 / dx3
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
        match Model::build(&self.cfg) {
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
                Snap { frames: g.frames.clone(), live: g.live.clone(), done: g.done, error: g.error.clone(), n: g.n, dt: g.dt, steps: g.steps, wall: g.wall, sim_time: g.sim_time, t_end: g.t_end, parts: g.parts.clone(), surface: g.surface.clone(), ref_pos: g.ref_pos.clone() }
            }
            None => Snap { frames: vec![], live: None, done: false, error: None, n: 0, dt: 0.0, steps: 0, wall: 0.0, sim_time: 0.0, t_end: 1.0, parts: Default::default(), surface: Default::default(), ref_pos: Default::default() },
        }
    }

    fn running(&self, s: &Snap) -> bool {
        self.job.as_ref().map_or(false, |j| !s.done && !j.paused.load(Ordering::SeqCst))
    }
}

impl eframe::App for PenSim {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
            }
            if self.playing {
                let dt = ctx.input(|i| i.stable_dt).min(0.1);
                let fps = nf as f32 / PLAY_SECONDS;
                self.play_pos += dt * fps * self.speed;
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

        egui::Panel::top("title").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("PenSim").strong().size(18.0).color(ACCENT));
                ui.label(RichText::new("· Simulação de impacto balístico 3D").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("{} threads", pensim_core::threads()));
                });
            });
        });
        egui::Panel::top("ribbon").show(ui, |ui| self.ribbon(ui, &ctx, &snap));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &snap));

        let wide = ui.available_width() > 820.0;
        if wide {
            egui::Panel::left("browser").default_size(230.0).resizable(true).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.browser(ui));
            });
            egui::Panel::right("props").default_size(340.0).resizable(true).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.props(ui, &ctx, &snap));
            });
        } else {
            let h = ui.available_height() * 0.42;
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
    }
}

// ---------------------------------------------------------------- ribbon

fn rbtn(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(format!("{icon}\n{label}")).size(12.0)).min_size(Vec2::new(62.0, 48.0)))
}

fn group(ui: &mut egui::Ui, name: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.horizontal(add);
        ui.label(RichText::new(name).small().weak());
    });
    ui.separator();
}

impl PenSim {
    fn ribbon(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snap) {
        ui.horizontal(|ui| {
            for (w, l) in [(Ws::Design, "PROJETO"), (Ws::Resolution, "RESOLUÇÃO"), (Ws::Sim, "SIMULAÇÃO"), (Ws::Results, "RESULTADOS")] {
                if ui.selectable_label(self.ws == w, RichText::new(l).strong()).clicked() {
                    self.ws = w;
                    if w != Ws::Design {
                        self.sketch_mode = false;
                    }
                }
            }
        });
        ui.separator();
        egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| match self.ws {
                Ws::Design => {
                    if self.sketch_mode {
                        group(ui, "FERRAMENTAS DE ESBOÇO", |ui| {
                            for (t, i, l) in [(Tool::Move, "✥", "Mover"), (Tool::Add, "+", "Ponto"), (Tool::Delete, "✕", "Excluir")] {
                                if ui.add(egui::Button::new(RichText::new(format!("{i}\n{l}")).size(12.0)).min_size(Vec2::new(62.0, 48.0)).selected(self.sketch.tool == t)).clicked() {
                                    self.sketch.tool = t;
                                }
                            }
                        });
                        group(ui, "GRADE", |ui| {
                            for s in [0.25, 0.5, 1.0] {
                                ui.selectable_value(&mut self.sketch.snap_mm, s, format!("{s} mm"));
                            }
                        });
                        group(ui, "SAIR", |ui| {
                            if rbtn(ui, "✔", "Concluir").clicked() {
                                self.sketch_mode = false;
                            }
                        });
                    } else {
                        group(ui, "ESBOÇO", |ui| {
                            if rbtn(ui, "✎", "Editar\nperfil").clicked() {
                                self.sketch_mode = true;
                                self.sel = Sel::Sketch;
                            }
                        });
                        group(ui, "PERFIS RÁPIDOS", |ui| {
                            for n in Nose::ALL {
                                if rbtn(ui, "◗", n.label()).clicked() {
                                    let p = &mut self.cfg.design.projectile;
                                    let d = (p.max_radius() * 2.0).max(0.004);
                                    let l = p.length().max(0.01);
                                    p.profile = n.profile(d, l);
                                }
                            }
                        });
                        group(ui, "ALVO", |ui| {
                            if rbtn(ui, "＋", "Camada").clicked() {
                                let mut l = self.cfg.design.layers.last().cloned().unwrap_or_else(|| Preset::Rifle762.config().design.layers[0].clone());
                                l.thickness = 0.008;
                                l.material = LIBRARY[2].clone();
                                self.cfg.design.layers.push(l);
                                self.sel = Sel::Layer(self.cfg.design.layers.len() - 1);
                            }
                            if let Sel::Layer(i) = self.sel {
                                if rbtn(ui, "✕", "Remover").clicked() && self.cfg.design.layers.len() > 1 {
                                    self.cfg.design.layers.remove(i);
                                    self.sel = Sel::Layer(0);
                                }
                            }
                        });
                        group(ui, "BIBLIOTECA", |ui| {
                            egui::ComboBox::from_id_salt("preset").width(210.0).selected_text(self.preset.label()).show_ui(ui, |ui| {
                                for p in Preset::ALL {
                                    if ui.selectable_value(&mut self.preset, p, p.label()).clicked() {
                                        self.cfg = p.config();
                                    }
                                }
                            });
                        });
                    }
                }
                Ws::Resolution => {
                    group(ui, "PREDEFINIÇÕES", |ui| {
                        for (l, dx) in RES_PRESETS {
                            let on = (self.cfg.dx - dx).abs() < 1e-9;
                            if ui.add(egui::Button::new(RichText::new(format!("{:.1} mm\n{l}", dx * 1e3)).size(12.0)).min_size(Vec2::new(70.0, 48.0)).selected(on)).clicked() {
                                self.cfg.dx = dx;
                            }
                        }
                    });
                    group(ui, "AJUSTE FINO", |ui| {
                        let mut mm = self.cfg.dx * 1e3;
                        if ui.add(egui::DragValue::new(&mut mm).range(0.6..=3.0).speed(0.02).suffix(" mm").fixed_decimals(2)).changed() {
                            self.cfg.dx = mm / 1e3;
                        }
                    });
                    group(ui, "EXECUTAR", |ui| {
                        if rbtn(ui, "▶", "Simular").clicked() {
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
                        if rbtn(ui, "▶", "Simular").clicked() {
                            self.start(ctx);
                        }
                        self.pause_btn(ui, snap);
                    });
                    group(ui, "ANÁLISE", |ui| {
                        if rbtn(ui, "◎", "Limite\nbalístico").clicked() {
                            self.ws = Ws::Results;
                            self.res_tab = ResTab::Limit;
                        }
                    });
                }
                Ws::Results => {
                    group(ui, "CONTORNO", |ui| {
                        egui::ComboBox::from_id_salt("field").width(210.0).selected_text(self.field.label()).show_ui(ui, |ui| {
                            for f in Field::ALL {
                                ui.selectable_value(&mut self.field, f, f.label());
                            }
                        });
                        ui.checkbox(&mut self.bands, "Bandas");
                    });
                    group(ui, "VISTA", |ui| {
                        if rbtn(ui, "◧", if self.section { "Corte\nligado" } else { "Corte\ndesligado" }).clicked() {
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
                        if ui.add_enabled(can, egui::Button::new(RichText::new(if self.playing { "⏸\nPausar" } else { "▶\nReproduzir" }).size(12.0)).min_size(Vec2::new(70.0, 48.0))).clicked() {
                            if self.playing {
                                self.playing = false;
                            } else {
                                if self.play_pos >= (nf - 1) as f32 {
                                    self.play_pos = 0.0;
                                }
                                self.playing = true;
                            }
                        }
                        if ui.add_enabled(can, egui::Button::new(RichText::new("⏮\nInício").size(12.0)).min_size(Vec2::new(56.0, 48.0))).clicked() {
                            self.play_pos = 0.0;
                            self.view_idx = 0;
                        }
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                for sp in [0.1f32, 0.25, 0.5, 1.0, 2.0, 4.0] {
                                    if ui.selectable_label((self.speed - sp).abs() < 1e-6, format!("{sp}×")).clicked() {
                                        self.speed = sp;
                                    }
                                }
                            });
                            ui.checkbox(&mut self.loop_play, "Repetir em laço");
                        });
                    });
                }
            });
        });
    }

    fn pause_btn(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        if let Some(j) = &self.job {
            if !snap.done {
                let paused = j.paused.load(Ordering::SeqCst);
                if rbtn(ui, if paused { "▶" } else { "⏸" }, if paused { "Continuar" } else { "Pausar" }).clicked() {
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
                ui.heading("Revolução 1");
                mat_combo(ui, "pm", &mut self.cfg.design.projectile.material);
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
        if ui.add(egui::Slider::new(&mut mm, 0.6..=3.0).logarithmic(true).text("espaçamento mm")).changed() {
            self.cfg.dx = mm / 1e3;
        }
        ui.label("Horizonte (× espaçamento): 2 = rápido · 3 = mais preciso");
        ui.horizontal(|ui| {
            for h in [2.0, 2.5, 3.0] {
                ui.selectable_value(&mut self.cfg.horizon_m, h, format!("{h}"));
            }
        });
        ui.add(egui::Slider::new(&mut self.cfg.n_frames, 16..=120).text("quadros gravados"));
        ui.separator();

        let n = self.estimate_nodes();
        let bonds = 4.19 * self.cfg.horizon_m.powi(3);
        let mem_bonds = n * bonds * 28.0 / 1e6;
        let mem_frames = n * 36.0 * self.cfg.n_frames as f64 / 1e6;
        let mem = mem_bonds + mem_frames;
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
            ui.label("Memória estimada");
            ui.label(format!("≈ {:.0} MB", mem));
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
        if mem > 1500.0 || n > 120_000.0 {
            ui.colored_label(bad, "Muito pesado para celular: provável falta de memória ou fechamento do app.");
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
                if ui.button(if self.playing { "⏸" } else { "▶" }).clicked() {
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
        if self.sketch_mode && self.ws == Ws::Design {
            let fill = mat_color(self.cfg.design.projectile.material.name);
            sketch::show(ui, &mut self.sketch, &mut self.cfg.design.projectile.profile, fill);
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
        let p = &self.preview;
        let view = View { cam: self.cam, rect, center: p.center, radius: p.radius };

        if let Some(e) = snap.error.as_ref().or(p.err.as_ref()) {
            painter.text(rect.center(), Align2::CENTER_CENTER, e, FontId::proportional(15.0), Color32::from_rgb(190, 40, 40));
        } else if results {
            let f = &snap.frames[self.view_idx];
            let n = f.pos.len();
            let idx: Vec<u32> = (0..n as u32)
                .filter(|&i| {
                    let i = i as usize;
                    let ry = snap.ref_pos[i][1];
                    if self.section && ry > cut {
                        return false;
                    }
                    snap.surface[i] || (self.section && ry >= cut - 1.1 * dx) || f.damage[i] > 0.4
                })
                .collect();
            let fi = Field::ALL.iter().position(|x| *x == self.field).unwrap();
            let sc_f = self.field.scale();
            if let Some(data) = self.field.data(f) {
                let (mut mx, mut mn) = (f32::MIN, f32::MAX);
                for &i in &idx {
                    let v = data[i as usize] * sc_f;
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
                view3d::render(&painter, &view, dx, &f.pos, &idx, false, &|i| rainbow((data[i] * sc_f - lo) / (hi - lo), bands));
                legend(&painter, rect, self.field, lo, hi);
            } else {
                let parts = snap.parts.clone();
                let mats: Vec<Color32> = std::iter::once(mat_color(self.cfg.design.projectile.material.name)).chain(self.cfg.design.layers.iter().map(|l| mat_color(l.material.name))).collect();
                view3d::render(&painter, &view, dx, &f.pos, &idx, true, &|i| mats[(parts[i] as usize).min(mats.len() - 1)]);
            }
            painter.text(rect.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, format!("t = {:.1} µs", f.time * 1e6), FontId::monospace(15.0), Color32::from_gray(30));
        } else {
            let mats: Vec<Color32> = std::iter::once(mat_color(self.cfg.design.projectile.material.name)).chain(self.cfg.design.layers.iter().map(|l| mat_color(l.material.name))).collect();
            let idx: Vec<u32> = if self.section { p.idx.iter().cloned().filter(|&i| p.pos[i as usize][1] <= cut).collect() } else { p.idx.clone() };
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
