use crate::plot::{line_plot, Series};
use crate::sketch::{self, SketchEditor, Tool};
use crate::view3d::{self, Camera, View};
use crate::worker::{Job, SweepJob};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use pensim_core::{Ammo, Config, Core, Design, Frame, Layer, Model, Nose, Outcome, OutcomeKind, PackedFrame, Preset, AMMO, LIBRARY};
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

/// qualidade da malha: nós no diâmetro do projétil
const RES_PRESETS: [(&str, f64); 7] = [("Rascunho", 5.0), ("Rápida", 6.0), ("Média", 8.0), ("Fina", 12.0), ("Muito fina", 16.0), ("Extrema", 24.0), ("Ultra", 32.0)];
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
    Projectile,
    Target,
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
    Files,
}

struct Preview {
    pos: Vec<[f32; 3]>,
    part: Vec<u8>,
    idx: Vec<u32>,
    center: [f32; 3],
    radius: f32,
    err: Option<String>,
    /// espaçamento usado no desenho (≥ o da simulação quando a malha real é pesada demais)
    dx: f32,
    note: Option<String>,
}

type Key = (Design, [f64; 9], [bool; 2]);

/// Simulação aberta de um arquivo salvo.
struct Loaded {
    title: String,
    scale: f64,
    frames: Vec<Arc<PackedFrame>>,
    parts: Arc<Vec<u8>>,
    surface: Arc<Vec<bool>>,
    normals: Arc<Vec<[f32; 3]>>,
    ref_pos: Arc<Vec<[f32; 3]>>,
    sym: u8,
    n: usize,
    dt: f64,
    steps: u64,
    wall: f64,
    t_end: f64,
    back_x: f64,
    outcome: Outcome,
}

struct SaveEntry {
    path: std::path::PathBuf,
    title: String,
    frames: usize,
    size: u64,
}

/// Pasta das simulações salvas (armazenamento interno do app no Android).
fn saves_dir() -> std::path::PathBuf {
    #[cfg(target_os = "android")]
    let base = crate::ANDROID_APP.get().and_then(|a| a.internal_data_path());
    #[cfg(not(target_os = "android"))]
    let base = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share/pensim"));
    let dir = base.unwrap_or_else(|| std::path::PathBuf::from(".")).join("saves");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1e6
}

type MeshKey = (u64, usize, bool, bool, u32, bool, bool, bool, u32, u64);

struct Snap {
    frames: Vec<Arc<PackedFrame>>,
    back_x: f64,
    live: Option<Outcome>,
    done: bool,
    error: Option<String>,
    n: usize,
    dt: f64,
    steps: u64,
    wall: f64,
    sim_time: f64,
    progress: f64,
    t_end: f64,
    sym: u8,
    normals: Arc<Vec<[f32; 3]>>,
    parts: Arc<Vec<u8>>,
    surface: Arc<Vec<bool>>,
    ref_pos: Arc<Vec<[f32; 3]>>,
}

pub struct PenSim {
    cfg: Config,
    ammo_idx: usize,
    ammo_scale: f64,
    /// qualidade da malha: nós no diâmetro do projétil (define a escala interna, invisível)
    res_nodes: f64,
    show_side: bool,
    show_fps: bool,
    loaded: Option<Loaded>,
    started_cfg: Option<Config>,
    /// escala do modelo usada na simulação em exibição (para converter resultados em valores reais)
    started_scale: f64,
    started_obl: f64,
    saves: Vec<SaveEntry>,
    saves_dirty: bool,
    save_msg: String,
    /// quadro descomprimido em exibição: (id da simulação, índice, quadro)
    decoded: Option<(u64, usize, Arc<Frame>)>,
    job_id: u64,
    logo_tex: Option<egui::TextureHandle>,
    /// esconde nós quase totalmente desligados (fragmentos soltos)
    hide_loose: bool,
    hide_proj: bool,
    /// face do corte colorida pelo contorno (desligado: cor neutra do material)
    cut_colored: bool,
    hide_target: bool,
    /// contorno escuro na silhueta do projétil
    outline: bool,
    /// desenha os nós como esferas suaves em vez de quadrados
    round_nodes: bool,
    faces: bool,
    mesh_cache: Option<(MeshKey, Option<Arc<crate::mesh::SurfMesh>>)>,
    disc_tex: Option<egui::TextureHandle>,
    /// margens extras (pt) somadas à área segura automática: [esq, topo, dir, base]
    extra_inset: [f32; 4],
    ws: Ws,
    sel: Sel,
    res_tab: ResTab,
    sheet_tab: u8,
    /// nas abas PROJÉTIL/ALVO: mostra o modelo em 3D em vez do editor 2D
    peek3d: bool,
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
        let cfg = AMMO[ammo_idx].config(1.0);
        let mut me = PenSim {
            cfg,
            ammo_idx,
            ammo_scale: 1.0,
            res_nodes: 8.0,
            show_side: true,
            show_fps: std::env::var("PENSIM_PROF").is_ok(),
            loaded: None,
            started_cfg: None,
            started_scale: 1.0,
            started_obl: 0.0,
            saves: vec![],
            saves_dirty: true,
            save_msg: String::new(),
            decoded: None,
            job_id: 0,
            logo_tex: crate::load_logo(&cc.egui_ctx),
            hide_loose: false,
            hide_proj: false,
            cut_colored: false,
            hide_target: false,
            outline: true,
            round_nodes: true,
            faces: false,
            mesh_cache: None,
            disc_tex: Some(view3d::disc_texture(&cc.egui_ctx)),
            extra_inset: [0.0, 0.0, 0.0, 0.0],
            ws: Ws::Projectile,
            sel: Sel::Projectile,
            res_tab: ResTab::Summary,
            sheet_tab: 0,
            peek3d: false,
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
            preview: Preview { pos: vec![], part: vec![], idx: vec![], center: [0.0; 3], radius: 0.05, err: None, dx: 0.0015, note: None },
            preview_key: None,
        };
        // ganchos de teste visual (captura de telas): PENSIM_WS, PENSIM_SKETCH, PENSIM_DEMO
        if let Ok(w) = std::env::var("PENSIM_WS") {
            me.ws = match w.as_str() {
                "res" => Ws::Resolution,
                "sim" => Ws::Sim,
                "results" => Ws::Results,
                "target" => Ws::Target,
                _ => Ws::Projectile,
            };
        }
        if let Some(v) = std::env::var("PENSIM_DX").ok().and_then(|v| v.parse::<f64>().ok()) {
            me.cfg.dx = v / 1e3;
        }
        if let Some(v) = std::env::var("PENSIM_ANGLE").ok().and_then(|v| v.parse::<f64>().ok()) {
            me.cfg.obliquity_deg = v;
        }
        if std::env::var("PENSIM_TARGET").is_ok() {
            me.ws = Ws::Target;
            me.cfg.design.layers.push(Layer { thickness: 0.008, gap_before: 0.004, width: 0.05, height: 0.05, round: false, material: LIBRARY[2].clone() });
            me.sketch.tsel = Some((1, true));
        }
        if std::env::var("PENSIM_SKETCH").is_ok() {
            me.ws = Ws::Projectile;
            me.sel = Sel::Sketch;
            me.sketch.selected = Some(3);
        }
        if let Ok(a) = std::env::var("PENSIM_AMMO") {
            if let Ok(i) = a.parse::<usize>() {
                me.load_ammo(i.min(AMMO.len() - 1), 1.0);
            }
        }
        if std::env::var("PENSIM_OPENLATEST").is_ok() {
            me.refresh_saves();
            if let Some(e) = me.saves.first() {
                let p = e.path.clone();
                me.open_save(&p);
            }
            me.res_tab = ResTab::Files;
            me.sheet_tab = 1;
        }
        if std::env::var("PENSIM_SIDE").is_ok() {
            me.cam = Camera::front();
        }
        if std::env::var("PENSIM_HIDETARGET").is_ok() {
            me.hide_target = true;
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
        (c.design.clone(), [c.dx, c.horizon_m, c.velocity, c.obliquity_deg, c.time_factor, c.noise, c.jitter, c.sleep_ratio, self.res_nodes], [c.clamp_edges, c.symmetry])
    }

    /// Escala interna (invisível): faz o diâmetro do projétil ter `res_nodes` nós na malha.
    /// Vale a similaridade geométrica; tempos e energias são convertidos de volta nos resultados.
    fn model_scale(&self) -> f64 {
        let d = self.cfg.design.projectile.max_radius() * 2.0;
        if d <= 1e-9 { 1.0 } else { (self.res_nodes * self.cfg.dx / d).clamp(0.004, 6.0) }
    }

    /// Configuração que de fato é simulada/pré-visualizada: o projeto real × escala interna.
    fn scaled_cfg(&self) -> Config {
        let f = self.model_scale();
        let mut c = self.cfg.clone();
        for q in c.design.projectile.profile.iter_mut() {
            q[0] *= f;
            q[1] *= f;
        }
        if let Some(core) = c.design.projectile.core.as_mut() {
            for q in core.profile.iter_mut() {
                q[0] *= f;
                q[1] *= f;
            }
        }
        for l in c.design.layers.iter_mut() {
            l.thickness *= f;
            l.gap_before *= f;
            l.width *= f;
            l.height *= f;
        }
        c
    }

    fn estimate_nodes(&self) -> f64 {
        self.scaled_cfg().estimate_nodes()
    }

    fn refresh_preview(&mut self) {
        let key = self.key();
        if self.preview_key.as_ref() == Some(&key) {
            return;
        }
        self.preview_key = Some(key);
        // a pré-visualização desenha cada nó; malhas muito finas são mostradas numa malha mais grossa
        const MAX_PREVIEW_NODES: f64 = 120_000.0;
        let est = self.estimate_nodes();
        let mut pcfg = self.scaled_cfg();
        let mut note = None;
        if est > MAX_PREVIEW_NODES {
            let f = (est / MAX_PREVIEW_NODES).cbrt().ceil().max(2.0);
            pcfg.dx *= f;
            note = Some(format!("Prévia simplificada · malha real ≈ {:.1} mi nós", est / 1e6));
        }
        let pdx = pcfg.dx as f32;
        match Model::build_full(&pcfg) {
            Ok(m) => {
                // mesmo referencial do resultado: projétil na horizontal, alvo inclinado
                let (sa, ca) = (self.cfg.obliquity_deg.to_radians().sin() as f32, self.cfg.obliquity_deg.to_radians().cos() as f32);
                let pos: Vec<[f32; 3]> = m.pos.iter().map(|p| {
                    let (x, z) = (p[0] as f32, p[2] as f32);
                    [ca * x - sa * z, p[1] as f32, -(sa * x + ca * z)]
                }).collect();
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
                self.preview = Preview { pos, part: m.part, idx, center, radius, err: None, dx: pdx, note };
            }
            Err(e) => {
                self.preview = Preview { pos: vec![], part: vec![], idx: vec![], center: [0.0; 3], radius: 0.05, err: Some(e), dx: 0.0015, note: None };
            }
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.start_with(ctx, false);
    }

    /// Cancela a simulação em andamento (a thread de cálculo para) e volta ao modelo.
    fn cancel_job(&mut self) {
        self.job = None; // Drop cancela a thread
        self.job_id += 1;
        self.decoded = None;
        self.playing = false;
        self.save_msg = "Simulação cancelada.".into();
        if self.ws == Ws::Results {
            self.ws = Ws::Sim;
        }
    }

    /// `force`: ignora a verificação de memória (o usuário escolheu continuar mesmo assim).
    fn start_with(&mut self, ctx: &egui::Context, force: bool) {
        self.job = None;
        self.job_id += 1;
        self.decoded = None;
        self.range = [(0.0, 0.0); 7];
        self.view_idx = 0;
        self.autoplayed = false;
        self.playing = false;
        self.play_pos = 0.0;
        self.started_at = std::time::Instant::now();
        self.started_key = Some(self.key());
        self.loaded = None;
        let mut jcfg = self.scaled_cfg();
        jcfg.ignore_memory = force;
        self.started_cfg = Some(self.cfg.clone());
        self.started_scale = self.model_scale();
        self.started_obl = self.cfg.obliquity_deg;
        self.job = Some(Job::spawn(jcfg, ctx.clone()));
        self.ws = Ws::Results;
        self.field = Field::VonMises;
        self.cam = Camera::front();
    }

    fn snapshot(&self) -> Snap {
        if let (None, Some(l)) = (&self.job, &self.loaded) {
            return Snap { frames: l.frames.clone(), back_x: l.back_x, live: Some(l.outcome.clone()), done: true, error: None, n: l.n, dt: l.dt, steps: l.steps, wall: l.wall, sim_time: l.t_end, progress: 1.0, t_end: l.t_end, sym: l.sym, normals: l.normals.clone(), parts: l.parts.clone(), surface: l.surface.clone(), ref_pos: l.ref_pos.clone() };
        }
        match &self.job {
            Some(j) => {
                let g = j.shared.lock().unwrap();
                Snap { frames: g.frames.clone(), back_x: g.back_x, live: g.live.clone(), done: g.done, error: g.error.clone(), n: g.n, dt: g.dt, steps: g.steps, wall: g.wall, sim_time: g.sim_time, progress: g.progress, t_end: g.t_end, sym: g.sym, normals: g.normals.clone(), parts: g.parts.clone(), surface: g.surface.clone(), ref_pos: g.ref_pos.clone() }
            }
            None => Snap { frames: vec![], back_x: 0.0, live: None, done: false, error: None, n: 0, dt: 0.0, steps: 0, wall: 0.0, sim_time: 0.0, progress: 0.0, t_end: 1.0, sym: 0, normals: Default::default(), parts: Default::default(), surface: Default::default(), ref_pos: Default::default() },
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
                let (cw, ch) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
                // Só há sobreposição com as barras do sistema se o retângulo útil for MENOR que a
                // superfície. Se tiver o mesmo tamanho, a superfície já começa abaixo das barras
                // (reservar margem aqui seria contar em dobro e geraria uma faixa vazia).
                let (dh, dw) = ((h - ch).max(0.0), (w - cw).max(0.0));
                let (mut l, mut t, mut rr, mut b) = (0.0, 0.0, 0.0, 0.0);
                if dh > 2.0 {
                    t = (r.top as f32).clamp(0.0, dh);
                    b = dh - t;
                }
                if dw > 2.0 {
                    l = (r.left as f32).clamp(0.0, dw);
                    rr = dw - l;
                }
                let cap = |v: f32, m: f32| (v / ppp).min(m);
                return [cap(l, 48.0), cap(t, 32.0), cap(rr, 48.0), cap(b, 32.0)];
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
        if std::env::var("PENSIM_AUTOSAVE").is_ok() && snap.done && !snap.frames.is_empty() && self.loaded.is_none() && self.save_msg.is_empty() {
            self.save_current(&snap);
            self.res_tab = ResTab::Files;
            self.sheet_tab = 1;
        }
        self.refresh_preview();
        // painel de propriedades acompanha a aba: PROJÉTIL → projétil, ALVO → camada selecionada
        match self.ws {
            Ws::Projectile => {
                if !matches!(self.sel, Sel::Projectile | Sel::Sketch) {
                    self.sel = Sel::Projectile;
                }
            }
            Ws::Target => {
                let n = self.cfg.design.layers.len().max(1);
                let i = self.sketch.tsel.map(|t| t.0).unwrap_or(0).min(n - 1);
                self.sel = Sel::Layer(i);
            }
            _ => {}
        }

        // área segura: mantém a interface fora da barra de status, do recorte da câmera e da navegação
        let mut sys = system_insets(&ctx);
        if ui.available_width() > ui.available_height() {
            sys[1] = 0.0; // deitado: a faixa do topo fica igual à do celular em pé
        }
        let inset = [sys[0] + self.extra_inset[0], sys[1] + self.extra_inset[1], sys[2] + self.extra_inset[2], sys[3] + self.extra_inset[3]];
        let blank = egui::Frame::NONE.fill(ctx.global_style().visuals.panel_fill);
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
        // faixa do topo: sempre visível (na tela deitada é fina). Além da marca, ela mantém a
        // primeira linha de botões fora da barra de status e do recorte da câmera.
        egui::Panel::top("title")
            .frame(egui::Frame::NONE.fill(ctx.global_style().visuals.panel_fill).inner_margin(egui::Margin::symmetric(8, 2)))
            .show_separator_line(true)
            .show(ui, |ui| {
                // linha fina: sem a altura mínima de toque dos botões
                ui.spacing_mut().interact_size.y = 22.0;
                ui.horizontal(|ui| {
                    if let Some(t) = &self.logo_tex {
                        ui.image((t.id(), Vec2::splat(20.0)));
                    }
                    ui.label(RichText::new("PenSim").strong().size(16.0).color(ACCENT));
                    ui.label(RichText::new("· Simulação de impacto balístico 3D").weak().size(12.0));
                    ui.label(RichText::new(format!("· {} threads", pensim_core::threads())).weak().size(11.0));
                });
            });
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
        for (w, l) in [(Ws::Projectile, "PROJÉTIL"), (Ws::Target, "ALVO"), (Ws::Resolution, "RESOLUÇÃO"), (Ws::Sim, "SIMULAÇÃO"), (Ws::Results, "RESULTADOS")] {
            if ui.selectable_label(self.ws == w, RichText::new(l).strong()).clicked() {
                self.ws = w;
                self.peek3d = false;
                if matches!(w, Ws::Projectile | Ws::Target) {
                    self.sheet_tab = 1; // abre as propriedades (material, espessura…)
                }
            }
        }
    }

    /// Seleciona um item e abre a aba correspondente (PROJÉTIL, ALVO, RESOLUÇÃO ou SIMULAÇÃO).
    fn goto(&mut self, sel: Sel) {
        self.sel = sel;
        self.sheet_tab = 1;
        self.peek3d = false;
        match sel {
            Sel::Sketch | Sel::Projectile => self.ws = Ws::Projectile,
            Sel::Layer(i) => {
                self.ws = Ws::Target;
                self.sketch.tsel = Some((i, true));
            }
            Sel::Conditions => self.ws = Ws::Sim,
            Sel::Mesh => self.ws = Ws::Resolution,
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
            Ws::Target => {
                    let u = self.ammo_scale.max(1e-6);
                    group(ui, "CAMADA SELECIONADA (mm reais)", |ui| {
                        let sel = self.sketch.tsel.map(|t| t.0).unwrap_or(0).min(self.cfg.design.layers.len() - 1);
                        let l = &mut self.cfg.design.layers[sel];
                        let (mut g, mut t, mut h, mut w) = (l.gap_before * 1e3 / u, l.thickness * 1e3 / u, l.height * 1e3 / u, l.width * 1e3 / u);
                        ui.label(format!("#{}", sel + 1));
                        ui.add_enabled(sel > 0, egui::DragValue::new(&mut g).prefix("folga ").suffix(" mm").speed(0.5).range(0.0..=80.0 / u).fixed_decimals(0));
                        ui.add(egui::DragValue::new(&mut t).prefix("esp. ").suffix(" mm").speed(1.0).range(1.0 / u..=400.0 / u).fixed_decimals(0));
                        ui.add(egui::DragValue::new(&mut h).prefix("alt. ").suffix(" mm").speed(1.0).range(20.0 / u..=500.0 / u).fixed_decimals(0));
                        ui.add_enabled(!l.round, egui::DragValue::new(&mut w).prefix("larg. ").suffix(" mm").speed(1.0).range(20.0 / u..=500.0 / u).fixed_decimals(0));
                        ui.checkbox(&mut l.round, "Disco");
                        l.gap_before = g * u / 1e3;
                        l.thickness = t * u / 1e3;
                        l.height = h * u / 1e3;
                        l.width = if l.round { h * u / 1e3 } else { w * u / 1e3 };
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
                    group(ui, "ÂNGULO DO ALVO", |ui| {
                        ui.add(egui::DragValue::new(&mut self.cfg.obliquity_deg).range(0.0..=85.0).suffix(" °").speed(0.5).fixed_decimals(0));
                        for deg in [0.0, 15.0, 30.0, 45.0, 60.0] {
                            if ui.selectable_label((self.cfg.obliquity_deg - deg).abs() < 0.5, format!("{deg:.0}°")).clicked() {
                                self.cfg.obliquity_deg = deg;
                            }
                        }
                    });
                    group(ui, "GRADE (mm reais)", |ui| {
                        let u = self.ammo_scale.max(1e-6);
                        for s in [1.0, 5.0, 10.0, 25.0] {
                            if ui.selectable_label((self.sketch.snap_mm / u - s).abs() < 1e-6 * s.max(1.0), format!("{s}")).clicked() {
                                self.sketch.snap_mm = s * u;
                            }
                        }
                    });
                    group(ui, "VISTA", |ui| {
                        if tbtn(ui, "▣", if self.peek3d { "Voltar ao\ndesenho" } else { "Ver em\n3D" }, self.peek3d, true, 70.0).clicked() {
                            self.peek3d = !self.peek3d;
                        }
                    });
            }
            Ws::Projectile => {
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
                                    self.load_ammo(i, 1.0);
                                }
                            }
                        });
                    });
                    group(ui, "FERRAMENTAS DE ESBOÇO", |ui| {
                        for (t, i, l) in [(Tool::Move, "✥", "Mover"), (Tool::Add, "+", "Ponto"), (Tool::Delete, "✕", "Excluir")] {
                            if tbtn(ui, i, l, self.sketch.tool == t, true, 62.0).clicked() {
                                self.sketch.tool = t;
                            }
                        }
                    });
                    let sel = self.sketch.selected;
                    let u = self.ammo_scale.max(1e-6);
                    group(ui, "PONTO SELECIONADO", |ui| {
                        let prof = &mut self.cfg.design.projectile.profile;
                        match sel {
                            Some(i) if i < prof.len() => {
                                let n = prof.len();
                                let lo = if i == 0 { 0.0 } else { prof[i - 1][0] * 1e3 / u + 0.05 };
                                let hi = if i + 1 == n { 400.0 / u } else { prof[i + 1][0] * 1e3 / u - 0.05 };
                                let (mut a, mut r) = (prof[i][0] * 1e3 / u, prof[i][1] * 1e3 / u);
                                ui.add_enabled(i != 0, egui::DragValue::new(&mut a).prefix("a ").suffix(" mm").speed(0.1).range(lo..=hi.max(lo)).fixed_decimals(1));
                                ui.add(egui::DragValue::new(&mut r).prefix("r ").suffix(" mm").speed(0.1).range(0.0..=40.0 / u).fixed_decimals(1));
                                prof[i] = [a * u / 1e3, r * u / 1e3];
                            }
                            _ => {
                                ui.weak("toque num ponto");
                            }
                        }
                    });
                    group(ui, "GRADE (mm reais)", |ui| {
                        let u = self.ammo_scale.max(1e-6);
                        for s in [0.5, 1.0, 2.0, 5.0] {
                            if ui.selectable_label((self.sketch.snap_mm / u - s).abs() < 1e-6 * s.max(1.0), format!("{s}")).clicked() {
                                self.sketch.snap_mm = s * u;
                            }
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
                    group(ui, "VISTA", |ui| {
                        if tbtn(ui, "▣", if self.peek3d { "Voltar ao\ndesenho" } else { "Ver em\n3D" }, self.peek3d, true, 70.0).clicked() {
                            self.peek3d = !self.peek3d;
                        }
                    });
            }
            Ws::Resolution => {
                group(ui, "QUALIDADE DA MALHA (nós no diâmetro)", |ui| {
                    for (l, n) in RES_PRESETS {
                        let on = (self.res_nodes - n).abs() < 0.5;
                        if tbtn(ui, &format!("{n:.0} nós"), l, on, true, 70.0).clicked() {
                            self.res_nodes = n;
                        }
                    }
                });
                group(ui, "AJUSTE FINO", |ui| {
                    ui.add(egui::DragValue::new(&mut self.res_nodes).range(3.0..=48.0).speed(0.2).suffix(" nós").fixed_decimals(0));
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
                    ui.add(egui::DragValue::new(&mut self.cfg.obliquity_deg).range(0.0..=85.0).suffix(" °").speed(0.5));
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
                    if tbtn(ui, "◩", "Corte\ncolorido", self.cut_colored, true, 62.0).clicked() {
                        self.cut_colored = !self.cut_colored;
                    }
                    if tbtn(ui, "●", "Nós\nredondos", self.round_nodes, true, 62.0).clicked() {
                        self.round_nodes = !self.round_nodes;
                    }
                    if tbtn(ui, "∴", "Ocultar\nsoltas", self.hide_loose, true, 62.0).clicked() {
                        self.hide_loose = !self.hide_loose;
                    }
                    if tbtn(ui, "◌", "Contorno", self.outline, true, 62.0).clicked() {
                        self.outline = !self.outline;
                    }
                    if tbtn(ui, "◁", "Ocultar\nprojétil", self.hide_proj, true, 70.0).clicked() {
                        self.hide_proj = !self.hide_proj;
                    }
                    if tbtn(ui, "▯", "Ocultar\nalvo", self.hide_target, true, 62.0).clicked() {
                        self.hide_target = !self.hide_target;
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
        if self.job.is_some() && !snap.done && tbtn(ui, "✕", "Cancelar", false, true, 62.0).clicked() {
            self.cancel_job();
            return;
        }
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
                this.goto(sel);
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
                ui.selectable_value(&mut self.res_tab, ResTab::Files, "Arquivos");
            });
            ui.separator();
            match self.res_tab {
                ResTab::Summary => self.ui_summary(ui, snap),
                ResTab::Charts => self.ui_charts(ui, snap),
                ResTab::Limit => self.ui_limit(ui, ctx),
                ResTab::Files => self.ui_files(ui, snap),
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
                ui.weak("Edite na aba PROJÉTIL: arraste os pontos ou use as setas.");
                let p = &self.cfg.design.projectile;
                ui.label(format!("Pontos: {} · L = {:.1} mm · ⌀ = {:.1} mm (reais)", p.profile.len(), p.length() * 1e3 / self.ammo_scale.max(1e-6), p.max_radius() * 2e3 / self.ammo_scale.max(1e-6)));
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
                    ui.label(format!("{:.1} g", am.mass_g));
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
                ui.label(format!("L = {:.1} mm · ⌀ = {:.1} mm (reais)", p.length() * 1e3 / self.ammo_scale.max(1e-6), p.max_radius() * 2e3 / self.ammo_scale.max(1e-6)));
            }
            Sel::Layer(i) => {
                let n = self.cfg.design.layers.len();
                if i >= n {
                    self.sel = Sel::Layer(0);
                    return;
                }
                ui.heading(format!("Extrusão {}", i + 1));
                if self.ws != Ws::Target && ui.button("✎ Editar na aba ALVO").clicked() {
                    self.goto(Sel::Layer(i));
                }
                let l: &mut Layer = &mut self.cfg.design.layers[i];
                mat_combo(ui, "lm", &mut l.material);
                // medidas REAIS (o modelo guarda × escala)
                let u = self.ammo_scale.max(1e-6);
                let (mut t, mut g, mut w, mut h) = (l.thickness * 1e3 / u, l.gap_before * 1e3 / u, l.width * 1e3 / u, l.height * 1e3 / u);
                ui.add(egui::Slider::new(&mut t, 2.0..=1500.0).logarithmic(true).text("espessura real mm"));
                ui.add(egui::Slider::new(&mut g, 0.0..=300.0).text("folga antes mm"));
                ui.add(egui::Slider::new(&mut w, 20.0..=2000.0).logarithmic(true).text(if l.round { "diâmetro real mm" } else { "largura real mm" }));
                if !l.round {
                    ui.add(egui::Slider::new(&mut h, 20.0..=2000.0).logarithmic(true).text("altura real mm"));
                }
                ui.checkbox(&mut l.round, "Seção circular (disco)");
                l.thickness = t * u / 1e3;
                l.gap_before = g * u / 1e3;
                l.width = w * u / 1e3;
                l.height = h * u / 1e3;
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
                ui.add(egui::Slider::new(&mut self.cfg.obliquity_deg, 0.0..=85.0).text("obliquidade °"));
                ui.add(egui::Slider::new(&mut self.cfg.time_factor, 0.5..=3.0).text("tempo final ×"));
                ui.checkbox(&mut self.cfg.clamp_edges, "Engastar bordas do alvo (apoio fixo)");
                ui.separator();
                ui.label(RichText::new("Realismo e velocidade").strong());
                let mut noise = (self.cfg.noise * 100.0) as f32;
                if ui.add(egui::Slider::new(&mut noise, 0.0..=40.0).text("ruído do material %")).on_hover_text("Variação aleatória da resistência/ruptura por ligação. 0 = material ideal (fratura 'perfeita').").changed() {
                    self.cfg.noise = noise as f64 / 100.0;
                }
                let mut jit = (self.cfg.jitter * 100.0) as f32;
                if ui.add(egui::Slider::new(&mut jit, 0.0..=15.0).text("jitter dos nós %")).on_hover_text("Perturba a posição dos nós (em % do espaçamento) para quebrar o padrão de malha regular.").changed() {
                    self.cfg.jitter = jit as f64 / 100.0;
                }
                ui.checkbox(&mut self.cfg.symmetry, "Simetria (2–4× mais rápido)");
                if self.cfg.symmetry {
                    ui.weak("Com simetria, o padrão de fratura se repete espelhado nos quadrantes (o ruído varia dentro de cada um). Desligue para obter fraturas totalmente assimétricas, a um custo maior.");
                }
                let mut sl = (self.cfg.sleep_ratio * 100.0) as f32;
                if ui.add(egui::Slider::new(&mut sl, 0.0..=8.0).text("região ativa %")).on_hover_text("Nós em repouso longe do impacto não são calculados até serem atingidos por ondas acima de X% da velocidade de impacto. 0 = calcula tudo. Acelera muito alvos grossos.").changed() {
                    self.cfg.sleep_ratio = sl as f64 / 100.0;
                }
                ui.weak("Obliquidade no plano X–Z, medida a partir da normal da placa.");
            }
            Sel::Mesh => {
                self.resolution_panel(ui, ctx);
                return;
            }
        }
        {
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
        ui.heading("Qualidade da malha");
        ui.label("Escolha quantos nós cabem no diâmetro do projétil. Mais nós = fratura e geometria mais fiéis, mas o custo cresce rápido. Todos os valores continuam em medidas reais.");
        ui.add_space(4.0);
        ui.label("Predefinições:");
        ui.horizontal_wrapped(|ui| {
            for (l, n) in RES_PRESETS {
                ui.selectable_value(&mut self.res_nodes, n, format!("{l} · {n:.0}"));
            }
        });
        ui.add(egui::Slider::new(&mut self.res_nodes, 3.0..=48.0).logarithmic(true).text("nós no diâmetro"));
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
        let sc = self.scaled_cfg();
        let bonds = sc.bonds_per_node();
        let mem = sc.estimate_memory_mb();
        let free = pensim_core::usable_memory_mb();
        let base = Preset::Rifle762.config();
        let base_n = 30000.0_f64;
        let rel = (n / base_n) * (self.cfg.horizon_m / base.horizon_m).powi(3) * (self.res_nodes / 8.0);
        let across_d = self.res_nodes;
        let thin = sc.design.layers.iter().map(|l| l.thickness).fold(f64::MAX, f64::min);
        let across_t = thin / sc.dx;
        egui::Grid::new("resgrid").num_columns(2).striped(true).show(ui, |ui| {
            ui.label("Nós estimados");
            ui.label(format!("≈ {:.0}", n));
            ui.end_row();
            ui.label("Ligações por nó");
            ui.label(format!("≈ {:.0}", bonds));
            ui.end_row();
            ui.label("Memória estimada / utilizável");
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
            ui.colored_label(warn, "Projétil com menos de 4 nós de diâmetro: geometria grosseira. Aumente a qualidade.");
        }
        if across_t < 3.0 {
            ui.colored_label(warn, "Camada com menos de 3 nós de espessura: perfuração pouco confiável.");
        }
        if free.map_or(false, |f| mem > 0.85 * f) {
            ui.colored_label(bad, "Não cabe na memória livre: a simulação será recusada. Diminua a qualidade da malha.");
        } else if mem > 1500.0 || n > 120_000.0 {
            ui.colored_label(bad, "Muito pesado: pode levar muito tempo e fechar o app por falta de memória. Você decide.");
        } else if mem > 600.0 || n > 50_000.0 {
            ui.colored_label(warn, "Pesado para celular: a simulação pode levar vários minutos.");
        } else {
            ui.colored_label(Color32::from_rgb(30, 140, 80), "Adequado para celular.");
        }
        ui.weak("A simulação roda internamente em tamanho conveniente; tempos, energias e penetração são convertidos de volta para valores reais.");
        ui.weak("O viewport mostra a malha atual: ajuste e veja os nós mudarem antes de simular.");
        ui.separator();
        ui.checkbox(&mut self.faces, "Mostrar faces (superfície contínua que segue os nós)");
        ui.weak("Só no resultado da simulação. Troca as bolinhas por uma malha de faces sombreada; se a malha for grande demais, volta sozinha para os nós.");
        ui.separator();
        if ui.add_sized([ui.available_width(), 42.0], egui::Button::new(RichText::new("▶ Simular com esta resolução").strong().color(Color32::WHITE)).fill(ACCENT)).clicked() {
            self.start(ctx);
        }
    }

    fn progress_card(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, s: &Snap) {
        let p = s.progress.clamp(0.0, 1.0);
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
        painter.text(card.center() + Vec2::new(0.0, 36.0), Align2::CENTER_TOP, "A reprodução começa sozinha quando terminar.", FontId::proportional(11.0), Color32::from_gray(110));
        let ctx2 = ui.ctx().clone();
        egui::Area::new(egui::Id::new("cancel_card")).fixed_pos(card.center_bottom() + Vec2::new(-60.0, -34.0)).show(&ctx2, |ui| {
            if ui.add(egui::Button::new(RichText::new("✕ Cancelar simulação").strong()).min_size(Vec2::new(120.0, 28.0))).clicked() {
                self.cancel_job();
            }
        });
        let btn = Rect::from_center_size(card.center_bottom() + Vec2::new(0.0, -2.0), Vec2::ZERO);
        let _ = btn;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    /// Aviso de memória recusada com o botão para continuar mesmo assim.
    fn memory_override(&mut self, ui: &mut egui::Ui, err: &str) {
        ui.colored_label(Color32::from_rgb(200, 50, 40), err);
        if err.starts_with("Memória insuficiente") {
            ui.weak("O app pode ficar lento ou ser fechado pelo Android se faltar memória de verdade.");
            if ui.add_sized([ui.available_width(), 38.0], egui::Button::new(RichText::new("Ignorar e continuar mesmo assim").strong())).clicked() {
                let ctx = ui.ctx().clone();
                self.start_with(&ctx, true);
            }
        }
    }

    /// Estimativa do tamanho do arquivo (quadros sempre comprimidos).
    fn estimate_save_bytes(snap: &Snap) -> u64 {
        snap.frames.iter().map(|f| if f.is_compressed() { f.size_bytes() as u64 } else { f.size_bytes() as u64 / 6 }).sum::<u64>() + snap.parts.len() as u64 * 6
    }

    fn save_current(&mut self, snap: &Snap) {
        let (Some(cfg), Some(outcome)) = (self.started_cfg.clone(), snap.live.clone()) else {
            self.save_msg = "Nada para salvar.".into();
            return;
        };
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let title = format!("{} · {:.0} m/s{}", AMMO[self.ammo_idx].name, cfg.velocity, if cfg.obliquity_deg > 0.5 { format!(" · {:.0}°", cfg.obliquity_deg) } else { String::new() });
        let data = pensim_core::save::SaveData {
            title, velocity: cfg.velocity, obliquity_deg: cfg.obliquity_deg, dx: cfg.dx, scale: self.started_scale, design: cfg.design.clone(),
            n_phys: snap.n, dt: snap.dt, steps: snap.steps, wall: snap.wall, t_end: snap.t_end, back_x: snap.back_x, sym: snap.sym, outcome,
            parts: snap.parts.to_vec(), surface: snap.surface.to_vec(), normals: snap.normals.to_vec(), ref_pos: snap.ref_pos.to_vec(), frames: snap.frames.clone(),
        };
        let path = saves_dir().join(format!("sim_{secs}.psim"));
        self.save_msg = match pensim_core::save::write(&path, &data) {
            Ok(sz) => format!("Salvo: {} ({:.1} MB)", path.file_name().and_then(|n| n.to_str()).unwrap_or("?"), mb(sz)),
            Err(e) => format!("Falha ao salvar: {e}"),
        };
        self.saves_dirty = true;
    }

    fn open_save(&mut self, path: &std::path::Path) {
        let Some(d) = pensim_core::save::read(path) else {
            self.save_msg = "Arquivo inválido ou de outra versão.".into();
            return;
        };
        self.job = None;
        self.job_id += 1;
        self.decoded = None;
        self.cfg.design = d.design.clone();
        self.cfg.velocity = d.velocity;
        self.cfg.obliquity_deg = d.obliquity_deg;
        self.cfg.dx = d.dx;
        self.started_cfg = Some(self.cfg.clone());
        self.started_scale = d.scale;
        self.started_obl = d.obliquity_deg;
        self.range = [(0.0, 0.0); 7];
        self.view_idx = 0;
        self.autoplayed = false;
        self.playing = false;
        self.play_pos = 0.0;
        self.save_msg = format!("Aberto: {}", d.title);
        self.loaded = Some(Loaded {
            scale: d.scale,
            title: d.title, frames: d.frames, parts: Arc::new(d.parts), surface: Arc::new(d.surface), normals: Arc::new(d.normals), ref_pos: Arc::new(d.ref_pos),
            sym: d.sym, n: d.n_phys, dt: d.dt, steps: d.steps, wall: d.wall, t_end: d.t_end, back_x: d.back_x, outcome: d.outcome,
        });
        self.ws = Ws::Results;
    }

    fn refresh_saves(&mut self) {
        let mut v: Vec<SaveEntry> = std::fs::read_dir(saves_dir())
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "psim")).filter_map(|p| pensim_core::save::info(&p).map(|i| SaveEntry { path: p, title: i.title, frames: i.frames, size: i.size_bytes })).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| b.path.cmp(&a.path));
        self.saves = v;
        self.saves_dirty = false;
    }

    fn ui_files(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        if self.saves_dirty {
            self.refresh_saves();
        }
        ui.label(RichText::new("Salvar e abrir simulações").strong());
        let ready = snap.done && !snap.frames.is_empty() && self.loaded.is_none();
        let mem: u64 = snap.frames.iter().map(|f| f.size_bytes() as u64).sum();
        if !snap.frames.is_empty() {
            ui.label(format!("Na memória: {:.1} MB ({} quadros{}).", mb(mem), snap.frames.len(), if snap.frames.iter().any(|f| f.is_compressed()) { ", comprimidos" } else { "" }));
            let est = Self::estimate_save_bytes(snap);
            ui.label(format!("Arquivo estimado: ≈ {:.1} MB (sempre comprimido).", mb(est)));
            if mb(mem) >= 150.0 {
                ui.colored_label(Color32::from_rgb(190, 120, 0), "Simulação grande: salve para não perdê-la e liberar memória depois.");
            }
        }
        if ui.add_enabled(ready, egui::Button::new(RichText::new("Salvar esta simulação").strong())).clicked() {
            self.save_current(snap);
        }
        if !self.save_msg.is_empty() {
            ui.weak(self.save_msg.clone());
        }
        ui.separator();
        ui.label(RichText::new("Simulações salvas").strong());
        if self.saves.is_empty() {
            ui.weak("Nenhuma ainda.");
        }
        let mut open: Option<std::path::PathBuf> = None;
        let mut del: Option<std::path::PathBuf> = None;
        for e in &self.saves {
            ui.group(|ui| {
                ui.label(RichText::new(&e.title).strong());
                ui.weak(format!("{} quadros · {:.1} MB · {}", e.frames, mb(e.size), e.path.file_name().and_then(|n| n.to_str()).unwrap_or("")));
                ui.horizontal(|ui| {
                    if ui.button("Abrir").clicked() {
                        open = Some(e.path.clone());
                    }
                    if ui.button("Excluir").clicked() {
                        del = Some(e.path.clone());
                    }
                });
            });
        }
        if let Some(p) = open {
            self.open_save(&p);
        }
        if let Some(p) = del {
            let _ = std::fs::remove_file(&p);
            self.saves_dirty = true;
        }
        let total: u64 = self.saves.iter().map(|e| e.size).sum();
        ui.weak(format!("Total salvo: {:.1} MB", mb(total)));
    }

    fn ui_summary(&mut self, ui: &mut egui::Ui, s: &Snap) {
        if let Some(e) = &s.error {
            self.memory_override(ui, e);
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
            let rs = self.started_scale.max(1e-6);
            row("Penetração máx. da ponta (real)", format!("{:.0} mm", o.penetration * 1e3 / rs));
            row("Duração simulada (real)", format!("{:.0} µs", s.t_end * 1e6 / rs));
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
        // valores REAIS: tempo ÷ escala, energia ÷ escala³ (similaridade geométrica)
        let rs = self.started_scale.max(1e-6);
        let mk = |name: &'static str, color, g: &dyn Fn(&PackedFrame) -> f64| Series { name, color, markers: false, pts: fr.iter().map(|f| (f.time * 1e6 / rs, g(f) / rs.powi(3))).collect() };
        line_plot(ui, 190.0, "Balanço de energia (valores reais)", "t real [µs]", "J", &[
            mk("Cinética proj.", Color32::from_rgb(30, 110, 220), &|f| f.ke_proj),
            mk("Cinética alvo", Color32::from_rgb(30, 160, 90), &|f| f.ke_target),
            mk("Plástica", Color32::from_rgb(240, 130, 20), &|f| f.e_plastic),
            mk("Fratura", Color32::from_rgb(210, 50, 50), &|f| f.e_fracture),
            mk("Elástica", Color32::from_rgb(140, 90, 210), &|f| f.e_elastic),
            mk("Contato", Color32::GRAY, &|f| f.e_contact),
        ]);
        ui.add_space(8.0);
        line_plot(ui, 170.0, "Velocidade do projétil (vx do CM)", "t real [µs]", "m/s", &[Series { name: "vx", color: Color32::from_rgb(30, 110, 220), markers: false, pts: fr.iter().map(|f| (f.time * 1e6 / rs, f.vx_proj)).collect() }]);
    }

    fn ui_limit(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label("Varre velocidades em paralelo (várias simulações 3D simultâneas) e refina por bissecção para estimar o limite balístico V50. Pode levar minutos.");
        ui.add(egui::Slider::new(&mut self.sweep_range.0, 100.0..=2500.0).text("v mín m/s"));
        ui.add(egui::Slider::new(&mut self.sweep_range.1, 200.0..=3000.0).text("v máx m/s"));
        ui.add(egui::Slider::new(&mut self.sweep_n, 3..=8).text("pontos"));
        let busy = self.sweep.as_ref().map_or(false, |s| !s.state.lock().unwrap().finished);
        if ui.add_enabled(!busy, egui::Button::new("◎ Calcular limite balístico")).clicked() && self.sweep_range.0 < self.sweep_range.1 {
            self.sweep = Some(SweepJob::spawn(self.scaled_cfg(), self.sweep_range.0, self.sweep_range.1, self.sweep_n, 2, ctx.clone()));
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
                let p = s.progress.clamp(0.0, 1.0) as f32;
                ui.add(egui::ProgressBar::new(p).desired_width((ui.available_width() - 130.0).max(80.0)).text(format!("calculando… {:.0}%", p * 100.0)));
                ui.label(if self.running(s) { "● em execução" } else { "pausado" });
                if ui.button("✕ Cancelar").clicked() {
                    self.cancel_job();
                }
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
                let t = s.frames[self.view_idx.min(nf - 1)].time * 1e6 / self.started_scale.max(1e-6);
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
            let mut go: Option<Sel> = None;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("LINHA DO TEMPO").small().weak());
                let cur = self.sel;
                let mut chip = |ui: &mut egui::Ui, sel: Sel, t: String| {
                    if ui.add(egui::Button::new(RichText::new(t).small()).selected(cur == sel)).clicked() {
                        go = Some(sel);
                    }
                };
                chip(ui, Sel::Sketch, "✎ Esboço 1".into());
                chip(ui, Sel::Projectile, "⟳ Revolução 1".into());
                for i in 0..self.cfg.design.layers.len() {
                    chip(ui, Sel::Layer(i), format!("▭ Extrusão {}", i + 1));
                }
                chip(ui, Sel::Conditions, "⚙ Impacto".into());
            });
            if let Some(s) = go {
                self.goto(s);
            }
        }
    }

    // --------------------------------------------------------- viewport 3D

    fn viewport(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        let t = std::time::Instant::now();
        self.viewport_inner(ui, snap);
        PROF_VIEW_US.store(t.elapsed().as_micros() as u64, Ordering::Relaxed);
    }

    fn viewport_inner(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        if self.ws == Ws::Target && !self.peek3d {
            let colors: Vec<Color32> = self.cfg.design.layers.iter().map(|l| mat_color(l.material.name)).collect();
            let proj = self.cfg.design.projectile.profile.clone();
            sketch::show_target(ui, &mut self.sketch, &mut self.cfg.design.layers, &proj, &colors, self.cfg.obliquity_deg, self.ammo_scale);
            return;
        }
        if self.ws == Ws::Projectile && !self.peek3d {
            let fill = mat_color(self.cfg.design.projectile.material.name);
            let core = self.cfg.design.projectile.core.as_ref().map(|c| (c.profile.clone(), mat_color(c.material.name)));
            sketch::show(ui, &mut self.sketch, &mut self.cfg.design.projectile.profile, fill, core.as_ref().map(|(p, c)| (p.as_slice(), *c)), self.ammo_scale);
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
        let round_id = if self.round_nodes { self.disc_tex.as_ref().map(|t| t.id()) } else { None };
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
            painter.text(rect.center() - Vec2::new(0.0, 30.0), Align2::CENTER_CENTER, e, FontId::proportional(14.0), Color32::from_rgb(190, 40, 40));
            if e.starts_with("Memória insuficiente") {
                let ctx2 = ui.ctx().clone();
                egui::Area::new(egui::Id::new("mem_override")).anchor(Align2::CENTER_CENTER, Vec2::new(0.0, 10.0)).show(&ctx2, |ui| {
                    if ui.add(egui::Button::new(RichText::new("Ignorar e continuar mesmo assim").strong()).min_size(Vec2::new(260.0, 38.0))).clicked() {
                        self.start_with(&ctx2, true);
                    }
                });
            }
        } else if results {
            // só o quadro exibido é descomprimido (e fica em cache enquanto não muda)
            let key = (self.job_id, self.view_idx);
            let fr: Arc<Frame> = match &self.decoded {
                Some((j, i, fr)) if (*j, *i) == key => fr.clone(),
                _ => {
                    let fr = snap.frames[self.view_idx].unpack();
                    self.decoded = Some((key.0, key.1, fr.clone()));
                    fr
                }
            };
            let f = &*fr;
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
            // a simulação usa o projétil inclinado; na tela giramos tudo para o projétil voar na
            // horizontal e o ALVO aparecer inclinado: R(α) leva d=(cosα,0,−senα) em (1,0,0)
            let (sa, ca) = (self.started_obl.to_radians().sin() as f32, self.started_obl.to_radians().cos() as f32);
            let turn = |q: [f32; 3]| [ca * q[0] - sa * q[2], q[1], -(sa * q[0] + ca * q[2])];
            for &m in &masks {
                ext_pos.extend(f.pos.iter().map(|&q| turn(flip(q, m))));
            }
            let mut idx: Vec<u32> = Vec::new();
            // nós que entram só por estarem na face do corte (sem cor de contorno, se desligado)
            let mut plain = vec![false; n * masks.len()];
            for (mi, &m) in masks.iter().enumerate() {
                for i in 0..n {
                    let ry = flip(snap.ref_pos[i], m)[1];
                    if self.section && !in_section(ry) {
                        continue;
                    }
                    if self.hide_loose && f.damage[i] >= 0.9 {
                        continue;
                    }
                    let is_p = snap.parts[i] < self.cfg.design.n_proj_parts() as u8;
                    if (is_p && self.hide_proj) || (!is_p && self.hide_target) {
                        continue;
                    }
                    let surf = snap.surface[i] || f.damage[i] > 0.4;
                    if surf || (self.section && near_face(ry)) {
                        idx.push((mi * n + i) as u32);
                        if !surf && !self.cut_colored {
                            plain[mi * n + i] = true;
                        }
                    }
                }
            }
            // malha de faces que segue os nós (opcional; cache por quadro)
            let faces_mesh: Option<Arc<crate::mesh::SurfMesh>> = if self.faces {
                let key: MeshKey = (self.job_id, self.view_idx, self.section, keep_pos, cut.to_bits(), self.hide_loose, self.hide_proj, self.hide_target, dx.to_bits(), self.started_obl.to_bits());
                match &self.mesh_cache {
                    Some((k, m)) if *k == key => m.clone(),
                    _ => {
                        let np8 = self.cfg.design.n_proj_parts() as u8;
                        let (hl, hp, ht, sec) = (self.hide_loose, self.hide_proj, self.hide_target, self.section);
                        let vis = |mi: usize, i: usize| -> u8 {
                            if hl && f.damage[i] >= 0.9 {
                                return 0;
                            }
                            let is_p = snap.parts[i] < np8;
                            if (is_p && hp) || (!is_p && ht) {
                                return 0;
                            }
                            if sec && !in_section(flip(snap.ref_pos[i], masks[mi])[1]) {
                                return 2;
                            }
                            1
                        };
                        let (sa, ca) = (self.started_obl.to_radians().sin() as f32, self.started_obl.to_radians().cos() as f32);
                        let turn2 = |q: [f32; 3]| [ca * q[0] - sa * q[2], q[1], -(sa * q[0] + ca * q[2])];
                        let m = crate::mesh::build(n, &masks, &f.pos, &snap.ref_pos, &snap.parts, np8, dx, self.started_obl.to_radians() as f32, 400_000, &flip, &turn2, &vis).map(Arc::new);
                        self.mesh_cache = Some((key, m.clone()));
                        m
                    }
                }
            } else {
                None
            };
            // iluminação: luz direcional (cima-esquerda da câmera) + difusa + brilho suave
            let (cr, cu, ce) = self.cam.basis();
            let ld = {
                let v = [ce[0] * 0.8 + cu[0] * 0.55 - cr[0] * 0.45, ce[1] * 0.8 + cu[1] * 0.55 - cr[1] * 0.45, ce[2] * 0.8 + cu[2] * 0.55 - cr[2] * 0.45];
                let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                [v[0] / m, v[1] / m, v[2] / m]
            };
            let (sa_l, ca_l) = (self.started_obl.to_radians().sin() as f32, self.started_obl.to_radians().cos() as f32);
            let cut_normal_y = if keep_pos { -1.0f32 } else { 1.0 };
            let section_on = self.section;
            let normals = snap.normals.clone();
            let light = move |e: usize| -> f32 {
                let (mi, i) = (e / n, e % n);
                let m = masks[mi];
                let nv = normals[i];
                let nn = if nv == [0.0; 3] {
                    // interior: na face do corte a normal aponta para a câmera; senão, de frente
                    if section_on { [0.0, cut_normal_y, 0.0] } else { ce }
                } else {
                    [nv[0], if m & 1 != 0 { -nv[1] } else { nv[1] }, if m & 2 != 0 { -nv[2] } else { nv[2] }]
                };
                let nn = if nv == [0.0; 3] { nn } else { [ca_l * nn[0] - sa_l * nn[2], nn[1], -(sa_l * nn[0] + ca_l * nn[2])] };
                let dif = (nn[0] * ld[0] + nn[1] * ld[1] + nn[2] * ld[2]).max(0.0);
                // brilho (Blinn) com o vetor de meia-via entre luz e câmera
                let h = [ld[0] + ce[0], ld[1] + ce[1], ld[2] + ce[2]];
                let hm = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt().max(1e-6);
                let sp = ((nn[0] * h[0] + nn[1] * h[1] + nn[2] * h[2]) / hm).max(0.0).powi(24);
                0.34 + 0.80 * dif + 0.22 * sp
            };
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
                let mat_cols: Vec<Color32> = self.cfg.design.materials().iter().map(|m| mat_color(m.name)).collect();
                let part_of = snap.parts.clone();
                if let Some(sm) = &faces_mesh {
                    view3d::render_surf(&painter, &view, sm, ld, &|e, cut| {
                        if cut && !self.cut_colored {
                            mat_cols[(part_of[e % n] as usize).min(mat_cols.len() - 1)]
                        } else {
                            rainbow((data[e % n] * sc_f - lo) / (hi - lo), bands)
                        }
                    });
                } else {
                    view3d::render(&painter, &view, dx, &ext_pos, &idx, false, Some(&light), round_id, &|i| {
                        if plain[i] {
                            mat_cols[(part_of[i % n] as usize).min(mat_cols.len() - 1)]
                        } else {
                            rainbow((data[i % n] * sc_f - lo) / (hi - lo), bands)
                        }
                    });
                }
                legend(&painter, rect, self.field, lo, hi);
                if self.outline && faces_mesh.is_none() {
                    let (parts, np) = (snap.parts.clone(), self.cfg.design.n_proj_parts() as u8);
                    view3d::outline(&painter, &view, dx, &ext_pos, &idx, &|i| if parts[i % n] >= np { 0 } else if f.damage[i % n] >= 0.6 { 2 } else { 1 }, Color32::from_rgb(20, 24, 36));
                }
            } else {
                let parts = snap.parts.clone();
                let mats: Vec<Color32> = self.cfg.design.materials().iter().map(|m| mat_color(m.name)).collect();
                if let Some(sm) = &faces_mesh {
                    view3d::render_surf(&painter, &view, sm, ld, &|e, _| mats[(parts[e % n] as usize).min(mats.len() - 1)]);
                } else {
                    view3d::render(&painter, &view, dx, &ext_pos, &idx, true, Some(&light), round_id, &|i| mats[(parts[i % n] as usize).min(mats.len() - 1)]);
                }
                if self.outline && faces_mesh.is_none() {
                    let np = self.cfg.design.n_proj_parts() as u8;
                    view3d::outline(&painter, &view, dx, &ext_pos, &idx, &|i| if parts[i % n] >= np { 0 } else if f.damage[i % n] >= 0.6 { 2 } else { 1 }, Color32::from_rgb(20, 24, 36));
                }
            }
            painter.text(rect.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, format!("t = {:.0} µs (tempo real equivalente)", f.time * 1e6 / self.started_scale.max(1e-6)), FontId::monospace(15.0), Color32::from_gray(30));
        } else {
            let mats: Vec<Color32> = self.cfg.design.materials().iter().map(|m| mat_color(m.name)).collect();
            let idx: Vec<u32> = if self.section { p.idx.iter().cloned().filter(|&i| in_section(p.pos[i as usize][1])).collect() } else { p.idx.clone() };
            view3d::render(&painter, &view, p.dx, &p.pos, &idx, true, None, round_id, &|i| mats[(p.part[i] as usize).min(mats.len() - 1)]);
            if self.outline {
                let np = self.cfg.design.n_proj_parts() as u8;
                view3d::outline(&painter, &view, p.dx, &p.pos, &idx, &|i| if p.part[i] < np { 1 } else { 0 }, Color32::from_rgb(20, 24, 36));
            }
            let label = p.note.clone().unwrap_or_else(|| format!("Modelo · {} nós", p.pos.len()));
            painter.text(rect.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, label, FontId::proportional(13.0), Color32::from_gray(60));
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
