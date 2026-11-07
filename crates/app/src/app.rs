use crate::plot::{line_plot, Series};
use crate::sketch::{self, SketchEditor, Tool};
use crate::view3d::{self, Camera, View};
use crate::draw::{self, DTool, DrawEditor, Drawing, Paint};
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

type Key = (Design, [f64; 9], [bool; 3]);

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
    /// barra de resultado aberta com todos os detalhes
    res_more: bool,
    /// projétil desenhado à mão (substitui o da biblioteca)
    pdraw: Option<Drawing>,
    ped: DrawEditor,
    /// objetos desenhados do alvo (um desenho por objeto) e o que está em edição
    tdraws: Vec<Drawing>,
    tedit: Option<usize>,
    ted: DrawEditor,
    mesh_cache: Option<(MeshKey, Option<Arc<crate::mesh::SurfMesh>>)>,
    topo_cache: Option<((u64, usize, usize, u32, u64), Arc<crate::mesh::Topo>)>,
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
    if name.contains("capuz") { c(150, 92, 70) }
    else if name.contains("Latão") { c(201, 162, 72) }
    else if name.contains("Enchimento") { c(214, 120, 40) }
    else if name.contains("Urânio") { c(80, 96, 70) }
    else if name.contains("Carboneto") { c(52, 60, 70) }
    else if name.contains("Aço doce") { c(150, 160, 172) }
    else if name.contains("Aço") { c(118, 134, 156) }
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
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
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
            res_more: false,
            pdraw: None,
            ped: DrawEditor::new(Paint::Mat(LIBRARY[11].clone()), 0.5),
            tdraws: vec![],
            tedit: None,
            ted: DrawEditor::new(Paint::Mat(LIBRARY[0].clone()), 1.0),
            mesh_cache: None,
            topo_cache: None,
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
            live_view: true,
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
                "res" => Ws::Sim,
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
        if let Some(k) = std::env::var("PENSIM_AMMO").ok().and_then(|v| v.parse::<usize>().ok()) {
            if k < AMMO.len() {
                me.ammo_idx = k;
                me.load_ammo(k, 1.0);
            }
        }
        if std::env::var("PENSIM_DRAW").is_ok() {
            // desenho de exemplo (captura de tela): bala com camisa e núcleo
            let mut d = Drawing::new("Projétil desenhado", 0.0);
            let o: Vec<usize> = [[6.0, 3.0], [14.0, 4.0], [30.0, 4.0], [30.0, 0.0]].iter().map(|&p| d.add_point(p)).collect();
            d.add_edge(0, o[0]);
            d.add_edge(o[0], o[1]);
            d.add_edge(o[1], o[2]);
            d.add_edge(o[2], o[3]);
            d.add_edge(o[3], 0);
            let i: Vec<usize> = [[6.0, 0.0], [10.0, 2.5], [28.0, 3.0], [28.0, 0.0]].iter().map(|&p| d.add_point(p)).collect();
            for k in 0..4 {
                d.add_edge(i[k], i[(k + 1) % 4]);
            }
            d.bucket(20.0, 3.6, Paint::Mat(LIBRARY[13].clone()));
            d.bucket(20.0, 1.0, Paint::Mat(LIBRARY[6].clone()));
            me.pdraw = Some(d);
            me.ws = Ws::Projectile;
            if std::env::var("PENSIM_FILLET").is_ok() {
                me.ped.tool = DTool::Fillet;
                if let Ok(f) = me.pdraw.as_ref().unwrap().start_fillet(o[0]) {
                    me.ped.fillet = Some(f);
                }
            }
        }
        if let Ok(img) = std::env::var("PENSIM_IMG") {
            if me.pdraw.is_none() {
                me.pdraw = Some(Drawing::new("Projétil desenhado", 0.0));
            }
            me.ws = Ws::Projectile;
            me.ped.pending_load = Some(img);
            me.ped.tool = DTool::Image;
        }
        if std::env::var("PENSIM_TOBJ").is_ok() {
            let mut d = Drawing::new("Objeto 1", 100.0);
            let q: Vec<usize> = [[0.0, -30.0], [20.0, -30.0], [35.0, 30.0], [10.0, 30.0]].iter().map(|&p| d.add_point(p)).collect();
            for k in 0..4 {
                d.add_edge(q[k], q[(k + 1) % 4]);
            }
            d.bucket(15.0, 0.0, Paint::Mat(LIBRARY[0].clone()));
            me.tdraws.push(d);
            me.tedit = Some(0);
            me.ws = Ws::Target;
        }
        if std::env::var("PENSIM_SIDE").is_ok() {
            me.cam = Camera::front();
        }
        if std::env::var("PENSIM_HIDETARGET").is_ok() {
            me.hide_target = true;
        }
        if std::env::var("PENSIM_FACES").is_ok() {
            me.faces = true;
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
        (c.design.clone(), [c.dx, c.horizon_m, c.velocity, c.obliquity_deg, c.time_factor, c.noise, c.jitter, c.sleep_ratio, self.res_nodes], [c.clamp_edges, c.symmetry, c.state_based])
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
        // RHA: dureza pela espessura real da chapa (chapas grossas são mais macias)
        let u = self.ammo_scale.max(1e-6);
        for l in c.design.layers.iter_mut() {
            pensim_core::adjust_rha(&mut l.material, l.thickness / u);
        }
        c.design.projectile.scale(f);
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
        self.clear_sim_cache(); // cancela, espera a thread acabar e libera a memória
        self.job_id += 1;
        self.playing = false;
        self.save_msg = "Simulação cancelada.".into();
        if self.ws == Ws::Results {
            self.ws = Ws::Sim;
        }
    }

    /// `force`: ignora a verificação de memória (o usuário escolheu continuar mesmo assim).
    /// Libera tudo o que a simulação anterior deixou na memória (cálculo, quadros, caches de
    /// desenho). Não mexe nas simulações SALVAS em arquivo.
    fn clear_sim_cache(&mut self) {
        self.job = None; // Drop espera o cálculo antigo acabar e liberar a memória
        self.loaded = None;
        self.decoded = None;
        self.mesh_cache = None;
        self.topo_cache = None;
        self.sweep = None;
        self.range = [(0.0, 0.0); 7];
    }

    fn start_with(&mut self, ctx: &egui::Context, force: bool) {
        self.clear_sim_cache();
        self.job_id += 1;
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
        self.sync_drawings();
        // a malha 3D da prévia só é montada quando aparece (montar a cada mudança do desenho travava a edição)
        let need3d = matches!(self.ws, Ws::Sim | Ws::Resolution | Ws::Results) || self.peek3d;
        if need3d {
            self.refresh_preview();
        }
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
        let wide = avail.x > avail.y * 1.05;
        let is_compact = avail.y < 520.0;
        COMPACT.store(is_compact, Ordering::Relaxed);
        egui::Panel::top("head")
            .frame(egui::Frame::NONE.fill(Color32::WHITE).inner_margin(egui::Margin::symmetric(8, 4)))
            .show_separator_line(true)
            .show(ui, |ui| self.header(ui));
        if self.ws == Ws::Results && (snap.live.is_some() || snap.error.is_some()) {
            egui::Panel::bottom("resbar").frame(egui::Frame::NONE.fill(Color32::from_rgb(250, 251, 253)).inner_margin(egui::Margin::symmetric(10, 6))).show_separator_line(true).show(ui, |ui| self.result_bar(ui, &snap));
        }
        if wide {
            let w = (avail.x * 0.36).clamp(290.0, 420.0);
            egui::Panel::right("panel").exact_size(w).resizable(false).frame(egui::Frame::NONE.fill(PANEL_BG).inner_margin(egui::Margin::same(10))).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.step_panel(ui, &ctx, &snap));
            });
        } else {
            // em pé, o painel não pode comer a vista: no resultado ele começa menor, tem teto de 45%
            // da altura e id próprio (o tamanho de um passo não vaza para o outro)
            let res = self.ws == Ws::Results;
            let ph = if res { avail.y * 0.30 } else { avail.y * 0.46 };
            egui::Panel::bottom(if res { "panel_res" } else { "panel" }).default_size(ph).min_size(60.0).max_size(avail.y * 0.55).resizable(true).frame(egui::Frame::NONE.fill(PANEL_BG).inner_margin(egui::Margin::same(10))).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.step_panel(ui, &ctx, &snap));
            });
        }
        if self.ws == Ws::Results && snap.frames.len() > 1 && (snap.done || self.live_view) {
            egui::Panel::bottom("playbar").frame(egui::Frame::NONE.fill(Color32::WHITE).inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| self.playbar(ui, &snap));
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

// ---------------------------------------------------------------- interface

const PANEL_BG: Color32 = Color32::from_rgb(243, 245, 248);
const GOOD: Color32 = Color32::from_rgb(30, 140, 80);
const WARN: Color32 = Color32::from_rgb(190, 120, 0);
const BAD: Color32 = Color32::from_rgb(200, 50, 40);

/// Cartão branco com título.
fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, Color32::from_rgb(222, 226, 232)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(15.0));
                ui.add_space(2.0);
            }
            add(ui);
        });
    ui.add_space(8.0);
}

/// Botão grande de ação principal.
fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add_sized([ui.available_width(), 44.0], egui::Button::new(RichText::new(text).strong().size(16.0).color(Color32::WHITE)).fill(ACCENT).corner_radius(8.0))
}

/// Botão de alternância (chip).
fn chip(ui: &mut egui::Ui, on: &mut bool, text: &str) {
    if ui.add(egui::Button::new(text).selected(*on).corner_radius(14.0)).clicked() {
        *on = !*on;
    }
}

/// Linha "rótulo ............ valor".
fn kv(ui: &mut egui::Ui, k: &str, v: String) {
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = 18.0;
        ui.label(RichText::new(k).color(Color32::from_gray(90)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(v).strong());
        });
    });
}

impl PenSim {
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(t) = &self.logo_tex {
                ui.image((t.id(), Vec2::splat(26.0)));
            }
            let narrow = ui.available_width() < 560.0;
            if !narrow {
                ui.label(RichText::new("PenSim").strong().size(17.0).color(ACCENT));
                ui.add_space(8.0);
            }
            let has_res = self.job.is_some() || self.loaded.is_some();
            for (k, (w, l)) in [(Ws::Projectile, "Munição"), (Ws::Target, "Alvo"), (Ws::Sim, "Simular"), (Ws::Results, "Resultado")].into_iter().enumerate() {
                let on = self.ws == w;
                let txt = if narrow { l.to_string() } else { format!("{}  {l}", k + 1) };
                let enabled = w != Ws::Results || has_res;
                let b = egui::Button::new(RichText::new(txt).size(14.0).strong().color(if on { Color32::WHITE } else { Color32::from_gray(60) }))
                    .fill(if on { ACCENT } else { Color32::TRANSPARENT })
                    .corner_radius(16.0)
                    .min_size(Vec2::new(0.0, 32.0));
                if ui.add_enabled(enabled, b).clicked() {
                    self.ws = w;
                    self.peek3d = false;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(RichText::new("⋯").size(18.0), |ui| {
                    ui.set_min_width(260.0);
                    ui.label(RichText::new("Tela").strong());
                    ui.checkbox(&mut self.show_fps, "Mostrar FPS e tempos");
                    ui.weak("Se algum botão perto da borda não responder (barra de status, câmera ou navegação do Android), aumente a margem.");
                    ui.add(egui::Slider::new(&mut self.extra_inset[1], 0.0..=80.0).text("margem topo"));
                    ui.add(egui::Slider::new(&mut self.extra_inset[3], 0.0..=80.0).text("margem base"));
                    ui.add(egui::Slider::new(&mut self.extra_inset[0], 0.0..=80.0).text("margem esquerda"));
                    ui.add(egui::Slider::new(&mut self.extra_inset[2], 0.0..=80.0).text("margem direita"));
                    ui.separator();
                    ui.weak(format!("PenSim · peridinâmica 3D · {} threads", pensim_core::threads()));
                });
                let running = self.job.is_some() && !self.snapshot_done();
                if running && self.ws != Ws::Results {
                    if ui.add(egui::Button::new(RichText::new("● calculando").color(ACCENT)).frame(false)).clicked() {
                        self.ws = Ws::Results;
                    }
                }
            });
        });
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
        self.sketch.selected = None;
        self.sketch.tsel = Some((0, true));
        self.pdraw = None;
        self.sync_objects();
    }

    /// Copia os desenhos para o projeto que é simulado.
    fn sync_drawings(&mut self) {
        if let Some(d) = &self.pdraw {
            if let Some(p) = d.to_projectile(1.0) {
                if p != self.cfg.design.projectile {
                    self.cfg.design.projectile = p;
                }
            }
        }
        self.sync_objects();
    }

    fn sync_objects(&mut self) {
        let u = self.ammo_scale.max(1e-6);
        let objs: Vec<pensim_core::TargetObject> = self.tdraws.iter().map(|d| d.to_object(u)).collect();
        if objs != self.cfg.design.objects {
            self.cfg.design.objects = objs;
        }
    }

    fn snapshot_done(&self) -> bool {
        self.job.as_ref().map_or(true, |j| j.shared.lock().unwrap().done)
    }

    fn step_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snap) {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
        match self.ws {
            Ws::Projectile => self.step_ammo(ui),
            Ws::Target => self.step_target(ui),
            Ws::Sim | Ws::Resolution => self.step_sim(ui, ctx),
            Ws::Results => self.step_results(ui, ctx, snap),
        }
    }

    fn next_btn(&mut self, ui: &mut egui::Ui, to: Ws, text: &str) {
        if ui.add_sized([ui.available_width(), 38.0], egui::Button::new(RichText::new(text).strong()).corner_radius(8.0)).clicked() {
            self.ws = to;
            self.peek3d = false;
        }
    }

    // ------------------------------------------------------------ 1 · munição

    fn step_ammo(&mut self, ui: &mut egui::Ui) {
        if self.pdraw.is_some() {
            self.step_ammo_drawn(ui);
            return;
        }
        let u = self.ammo_scale.max(1e-6);
        if ui.add_sized([ui.available_width(), 36.0], egui::Button::new(RichText::new("✎ Desenhar um projétil novo").strong()).corner_radius(8.0)).clicked() {
            self.pdraw = Some(Drawing::new("Projétil desenhado", 0.0));
            self.ped = DrawEditor::new(Paint::Mat(LIBRARY[11].clone()), 0.5);
            self.ammo_scale = 1.0;
            self.peek3d = false;
        }
        ui.add_space(6.0);
        card(ui, "Munição", |ui| {
            let name = AMMO[self.ammo_idx].name;
            egui::ComboBox::from_id_salt("ammo").width(ui.available_width() - 8.0).selected_text(name).show_ui(ui, |ui| {
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
            let am: &Ammo = &AMMO[self.ammo_idx];
            ui.add_space(4.0);
            kv(ui, "Calibre × comprimento", format!("{:.1} × {:.0} mm", am.caliber_mm, am.length_mm));
            kv(ui, "Massa", if am.mass_g >= 1000.0 { format!("{:.2} kg", am.mass_g / 1e3) } else { format!("{:.1} g", am.mass_g) });
            kv(ui, "Velocidade de boca", format!("{:.0} m/s", am.velocity));
            ui.add_space(2.0);
            ui.label(RichText::new(am.note).size(12.5).color(Color32::from_gray(90)));
        });
        self.velocity_card(ui);
        card(ui, "Peças e materiais", |ui| {
            let mats = self.cfg.design.materials();
            ui.horizontal(|ui| {
                ui.colored_label(mat_color(mats[0].name), "■");
                ui.label("Corpo");
            });
            mat_combo(ui, "pm", &mut self.cfg.design.projectile.material);
            let pr = &mut self.cfg.design.projectile;
            if let Some(c) = pr.core.as_mut() {
                ui.horizontal(|ui| {
                    ui.colored_label(mat_color(c.material.name), "■");
                    ui.label("Núcleo");
                });
                mat_combo(ui, "pc", &mut c.material);
            }
            let mut remove = None;
            for (k, g) in pr.regions.iter_mut().enumerate() {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    match &g.fill {
                        pensim_core::Fill::Mat(m) => {
                            ui.colored_label(mat_color(m.name), "■");
                        }
                        _ => {
                            ui.colored_label(Color32::from_gray(200), "□");
                        }
                    }
                    ui.label(&g.name);
                    match &g.fill {
                        pensim_core::Fill::Void => {
                            ui.weak("vazio");
                        }
                        pensim_core::Fill::Body => {
                            ui.weak("material do corpo");
                        }
                        _ => {}
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Remover peça").clicked() {
                            remove = Some(k);
                        }
                    });
                });
                if let pensim_core::Fill::Mat(m) = &mut g.fill {
                    mat_combo(ui, &format!("rg{k}"), m);
                }
            }
            if let Some(k) = remove {
                pr.regions.remove(k);
            }
            if pr.core.is_none() && pr.regions.is_empty() && ui.button("+ Adicionar núcleo").clicked() {
                let (l, r) = (pr.length(), pr.max_radius());
                pr.core = Some(Core { profile: vec![[0.1 * l, 0.55 * r], [0.8 * l, 0.55 * r]], material: LIBRARY[11].clone() });
            }
        });
        card(ui, "Editar o perfil", |ui| {
            ui.label(RichText::new("Arraste os pontos no desenho. As setas movem só na horizontal ou só na vertical.").size(12.5).color(Color32::from_gray(90)));
            ui.horizontal_wrapped(|ui| {
                for (t, l) in [(Tool::Move, "✥ Mover"), (Tool::Add, "+ Ponto"), (Tool::Delete, "✕ Excluir")] {
                    if ui.add(egui::Button::new(l).selected(self.sketch.tool == t).corner_radius(14.0)).clicked() {
                        self.sketch.tool = t;
                    }
                }
            });
            let prof = &mut self.cfg.design.projectile.profile;
            if let Some(i) = self.sketch.selected.filter(|&i| i < prof.len()) {
                let n = prof.len();
                let lo = if i == 0 { 0.0 } else { prof[i - 1][0] * 1e3 / u + 0.05 };
                let hi = if i + 1 == n { f64::MAX } else { prof[i + 1][0] * 1e3 / u - 0.05 };
                let (mut a, mut r) = (prof[i][0] * 1e3 / u, prof[i][1] * 1e3 / u);
                ui.horizontal(|ui| {
                    ui.label(format!("Ponto {}:", i + 1));
                    ui.add_enabled(i != 0, egui::DragValue::new(&mut a).prefix("dist. ").suffix(" mm").speed(0.1).range(lo..=hi.max(lo)).fixed_decimals(1));
                    ui.add(egui::DragValue::new(&mut r).prefix("raio ").suffix(" mm").speed(0.1).range(0.0..=f64::MAX).fixed_decimals(1));
                });
                prof[i] = [a * u / 1e3, r * u / 1e3];
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Grade:");
                for s in [0.5, 1.0, 2.0, 5.0, 10.0] {
                    if ui.selectable_label((self.sketch.snap_mm / u - s).abs() < 1e-6 * s.max(1.0), format!("{s} mm")).clicked() {
                        self.sketch.snap_mm = s * u;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Trocar a ponta:");
                for nz in Nose::ALL {
                    if ui.button(nz.label()).clicked() {
                        let p = &mut self.cfg.design.projectile;
                        let d = (p.max_radius() * 2.0).max(1e-4);
                        let l = p.length().max(1e-4);
                        p.profile = nz.profile(d, l);
                    }
                }
            });
            let p = &self.cfg.design.projectile;
            ui.weak(format!("Comprimento {:.1} mm · diâmetro {:.1} mm", p.length() * 1e3 / u, p.max_radius() * 2e3 / u));
        });
        self.next_btn(ui, Ws::Target, "Próximo: alvo  →");
    }

    fn velocity_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Velocidade de impacto", |ui| {
            ui.add(egui::Slider::new(&mut self.cfg.velocity, 50.0..=3000.0).suffix(" m/s").fixed_decimals(0).clamping(egui::SliderClamping::Never));
            if self.pdraw.is_none() {
                let vb = AMMO[self.ammo_idx].velocity;
                if (self.cfg.velocity - vb).abs() > 0.5 && ui.small_button(format!("usar a de boca ({vb:.0} m/s)")).clicked() {
                    self.cfg.velocity = vb;
                }
            }
        });
    }

    fn step_ammo_drawn(&mut self, ui: &mut egui::Ui) {
        card(ui, "Desenho do projétil", |ui| {
            ui.label(RichText::new("Desenhe metade do corte (acima do eixo). Feche cada contorno e encha com o balde: o contorno mais interno vale (ex.: contorno externo = camisa, interno = núcleo).").size(12.5).color(Color32::from_gray(90)));
            let (d, ed) = (self.pdraw.as_mut().unwrap(), &mut self.ped);
            draw_tools(ui, d, ed, draw::Mode::Projectile);
        });
        let ok = self.pdraw.as_ref().and_then(|d| d.to_projectile(1.0));
        card(ui, "Projétil", |ui| match &ok {
            Some(p) => {
                kv(ui, "Comprimento", format!("{:.1} mm", p.length() * 1e3));
                kv(ui, "Diâmetro", format!("{:.1} mm", p.max_radius() * 2e3));
                let m = proj_mass(p);
                kv(ui, "Massa", if m >= 1.0 { format!("{:.2} kg", m) } else { format!("{:.1} g", m * 1e3) });
                ui.weak("Peças mais finas que a malha podem sumir: aumente a qualidade se precisar.");
            }
            None => {
                ui.colored_label(WARN, "Ainda não há peça pintada: feche um contorno e use o balde.");
            }
        });
        self.velocity_card(ui);
        ui.horizontal(|ui| {
            if ui.button("🗋 Recomeçar o desenho").clicked() {
                if let Some(d) = self.pdraw.as_mut() {
                    self.ped.push(d);
                    *d = Drawing::new("Projétil desenhado", 0.0);
                    self.ped.current = Some(0);
                    self.ped.fit = true;
                }
            }
            if ui.button("← Voltar à biblioteca").clicked() {
                let i = self.ammo_idx;
                self.load_ammo(i, 1.0);
            }
        });
        ui.add_space(6.0);
        self.next_btn(ui, Ws::Target, "Próximo: alvo  →");
    }

    fn objects_section(&mut self, ui: &mut egui::Ui) {
        // objetos desenhados
        let mut remove_obj = None;
        let ntd = self.tdraws.len();
        for k in 0..ntd {
            let editing = self.tedit == Some(k);
            egui::Frame::NONE
                .fill(Color32::WHITE)
                .stroke(Stroke::new(if editing { 2.0 } else { 1.0 }, if editing { ACCENT } else { Color32::from_rgb(222, 226, 232) }))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&self.tdraws[k].name).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text("Remover objeto").clicked() {
                                remove_obj = Some(k);
                            }
                            if ui.button(if editing { "✓ Pronto" } else { "✎ Editar" }).clicked() {
                                self.tedit = if editing { None } else { Some(k) };
                                self.ted = DrawEditor::new(self.ted.paint.clone(), self.ted.snap_mm);
                                self.ted.current = None;
                                self.peek3d = false;
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Largura (profundidade)");
                        ui.add(egui::DragValue::new(&mut self.tdraws[k].width_mm).suffix(" mm").speed(1.0).range(1.0..=f64::MAX).fixed_decimals(0));
                    });
                    if editing {
                        ui.label(RichText::new("Desenhe o objeto visto de lado (o projétil vem da esquerda, pela linha de tiro). Feche os contornos e encha com o balde.").size(12.5).color(Color32::from_gray(90)));
                        draw_tools(ui, &mut self.tdraws[k], &mut self.ted, draw::Mode::Target);
                    }
                });
            ui.add_space(6.0);
        }
        if let Some(k) = remove_obj {
            self.tdraws.remove(k);
            self.tedit = None;
        }
    }

    // ------------------------------------------------------------ 2 · alvo

    fn step_target(&mut self, ui: &mut egui::Ui) {
        let u = self.ammo_scale.max(1e-6);
        if self.tedit.is_some() {
            // editando um objeto: o cartão dele vem primeiro
            self.objects_section(ui);
        }
        card(ui, "Ângulo do alvo", |ui| {
            ui.add(egui::Slider::new(&mut self.cfg.obliquity_deg, 0.0..=85.0).suffix(" °").fixed_decimals(0));
            ui.horizontal_wrapped(|ui| {
                for deg in [0.0, 30.0, 45.0, 60.0, 75.0] {
                    if ui.selectable_label((self.cfg.obliquity_deg - deg).abs() < 0.5, format!("{deg:.0}°")).clicked() {
                        self.cfg.obliquity_deg = deg;
                    }
                }
            });
            ui.weak("Medido a partir da perpendicular da chapa (0° = tiro de frente).");
        });
        let n = self.cfg.design.layers.len();
        let sel = self.sketch.tsel.map(|t| t.0).unwrap_or(0).min(n.saturating_sub(1));
        let mut action: Option<(usize, i32)> = None; // (camada, -1 subir · +1 descer · 0 remover)
        for i in 0..n {
            let open = i == sel;
            let l = &self.cfg.design.layers[i];
            let title = format!("Camada {} · {} · {:.0} mm", i + 1, l.material.name, l.thickness * 1e3 / u);
            egui::Frame::NONE
                .fill(Color32::WHITE)
                .stroke(Stroke::new(if open { 2.0 } else { 1.0 }, if open { ACCENT } else { Color32::from_rgb(222, 226, 232) }))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let hdr = ui.add(egui::Label::new(RichText::new(&title).strong()).sense(Sense::click()));
                    if hdr.clicked() {
                        self.sketch.tsel = Some((i, true));
                    }
                    if !open {
                        return;
                    }
                    let l: &mut Layer = &mut self.cfg.design.layers[i];
                    mat_combo(ui, &format!("lm{i}"), &mut l.material);
                    if l.material.name == LIBRARY[0].name {
                        let bhn = pensim_core::rha_bhn(l.thickness * 1e3 / u);
                        ui.weak(format!("Nesta espessura: ≈ {bhn:.0} BHN (chapas grossas são mais macias)."));
                    }
                    let (mut t, mut g, mut w, mut h) = (l.thickness * 1e3 / u, l.gap_before * 1e3 / u, l.width * 1e3 / u, l.height * 1e3 / u);
                    egui::Grid::new(format!("lg{i}")).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label("Espessura");
                        ui.add(egui::DragValue::new(&mut t).suffix(" mm").speed(1.0).range(0.5..=f64::MAX).fixed_decimals(0));
                        ui.end_row();
                        ui.label(if l.round { "Diâmetro" } else { "Largura" });
                        ui.add(egui::DragValue::new(&mut w).suffix(" mm").speed(2.0).range(1.0..=f64::MAX).fixed_decimals(0));
                        ui.end_row();
                        if !l.round {
                            ui.label("Altura");
                            ui.add(egui::DragValue::new(&mut h).suffix(" mm").speed(2.0).range(1.0..=f64::MAX).fixed_decimals(0));
                            ui.end_row();
                        }
                        if i > 0 {
                            ui.label("Espaço antes");
                            ui.add(egui::DragValue::new(&mut g).suffix(" mm").speed(0.5).range(0.0..=f64::MAX).fixed_decimals(0));
                            ui.end_row();
                        }
                    });
                    ui.checkbox(&mut l.round, "Disco (seção circular)");
                    l.thickness = t.max(0.5) * u / 1e3;
                    l.gap_before = g.max(0.0) * u / 1e3;
                    l.width = w.max(1.0) * u / 1e3;
                    l.height = if l.round { l.width } else { h.max(1.0) * u / 1e3 };
                    ui.horizontal(|ui| {
                        if ui.add_enabled(i > 0, egui::Button::new("▲")).on_hover_text("Mover para a frente").clicked() {
                            action = Some((i, -1));
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("▼")).on_hover_text("Mover para trás").clicked() {
                            action = Some((i, 1));
                        }
                        if ui.add_enabled(n > 1 || !self.tdraws.is_empty(), egui::Button::new("✕ Remover")).clicked() {
                            action = Some((i, 0));
                        }
                    });
                });
            ui.add_space(6.0);
        }
        if let Some((i, d)) = action {
            match d {
                0 => {
                    self.cfg.design.layers.remove(i);
                    self.sketch.tsel = Some((i.saturating_sub(1), true));
                }
                -1 => {
                    self.cfg.design.layers.swap(i, i - 1);
                    self.sketch.tsel = Some((i - 1, true));
                }
                _ => {
                    self.cfg.design.layers.swap(i, i + 1);
                    self.sketch.tsel = Some((i + 1, true));
                }
            }
        }
        if self.tedit.is_none() {
            self.objects_section(ui);
        }
        if ui.add_sized([ui.available_width(), 34.0], egui::Button::new("✎ + Novo objeto (desenhar)").corner_radius(8.0)).clicked() {
            let name = format!("Objeto {}", self.tdraws.len() + 1);
            self.tdraws.push(Drawing::new(&name, 100.0));
            self.tedit = Some(self.tdraws.len() - 1);
            self.ted = DrawEditor::new(Paint::Mat(LIBRARY[0].clone()), 1.0);
            self.peek3d = false;
        }
        if ui.add_sized([ui.available_width(), 34.0], egui::Button::new("+ Adicionar camada").corner_radius(8.0)).clicked() {
            let mut l = self.cfg.design.layers.last().cloned().unwrap_or_else(|| Preset::Rifle762.config().design.layers[0].clone());
            l.thickness = 0.008 * u;
            l.gap_before = 0.0;
            l.material = LIBRARY[2].clone();
            self.cfg.design.layers.push(l);
            self.sketch.tsel = Some((self.cfg.design.layers.len() - 1, true));
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Grade do desenho:");
            for s in [1.0, 5.0, 10.0, 25.0] {
                if ui.selectable_label((self.sketch.snap_mm / u - s).abs() < 1e-6 * s.max(1.0), format!("{s} mm")).clicked() {
                    self.sketch.snap_mm = s * u;
                }
            }
        });
        ui.add_space(6.0);
        self.next_btn(ui, Ws::Sim, "Próximo: simular  →");
    }

    // ------------------------------------------------------------ 3 · simular

    fn step_sim(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, "Impacto", |ui| {
            kv(ui, "Velocidade (aba Munição)", format!("{:.0} m/s", self.cfg.velocity));
            kv(ui, "Ângulo do alvo (aba Alvo)", format!("{:.0}°", self.cfg.obliquity_deg));
        });
        card(ui, "Qualidade", |ui| self.quality_section(ui));
        card(ui, "Modelo físico", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add(egui::Button::new("Preciso").selected(self.cfg.state_based).corner_radius(14.0)).clicked() {
                    self.cfg.state_based = true;
                }
                if ui.add(egui::Button::new("Rápido").selected(!self.cfg.state_based).corner_radius(14.0)).clicked() {
                    self.cfg.state_based = false;
                }
            });
            ui.weak(if self.cfg.state_based {
                "Peridinâmica state-based: Poisson real, metal escoa (plasticidade J2) e rompe por dano dúctil (Johnson-Cook). Mais realista, mais lento."
            } else {
                "Peridinâmica bond-based (modelo antigo): bem mais rápido, menos realista em metais."
            });
        });
        egui::CollapsingHeader::new(RichText::new("Avançado").strong()).id_salt("adv").show(ui, |ui| {
            ui.label("Horizonte (alcance das ligações, × espaçamento)");
            ui.horizontal(|ui| {
                for (h, l) in [(2.0, "2 · rápido"), (2.5, "2,5"), (3.0, "3 · preciso")] {
                    ui.selectable_value(&mut self.cfg.horizon_m, h, l);
                }
            });
            let mut secs = (self.cfg.n_frames as f32 / PLAY_FPS).clamp(1.0, 16.0);
            if ui.add(egui::Slider::new(&mut secs, 1.0..=16.0).text("duração da repetição s")).changed() {
                self.cfg.n_frames = (secs * PLAY_FPS).round() as usize;
            }
            ui.add(egui::Slider::new(&mut self.cfg.time_factor, 0.5..=3.0).text("tempo máximo ×"));
            let mut noise = (self.cfg.noise * 100.0) as f32;
            if ui.add(egui::Slider::new(&mut noise, 0.0..=40.0).text("variação do material %")).on_hover_text("Variação aleatória de resistência por ligação; 0 = material ideal").changed() {
                self.cfg.noise = noise as f64 / 100.0;
            }
            let mut jit = (self.cfg.jitter * 100.0) as f32;
            if ui.add(egui::Slider::new(&mut jit, 0.0..=15.0).text("irregularidade da malha %")).changed() {
                self.cfg.jitter = jit as f64 / 100.0;
            }
            let mut sl = (self.cfg.sleep_ratio * 100.0) as f32;
            if ui.add(egui::Slider::new(&mut sl, 0.0..=8.0).text("região ativa %")).on_hover_text("Nós parados longe do impacto só entram no cálculo quando atingidos por ondas acima de X% da velocidade de impacto. 0 = calcula tudo.").changed() {
                self.cfg.sleep_ratio = sl as f64 / 100.0;
            }
            ui.checkbox(&mut self.cfg.symmetry, "Simetria (2–4× mais rápido)");
            ui.checkbox(&mut self.cfg.clamp_edges, "Bordas do alvo presas");
        });
        ui.add_space(6.0);
        let stale = self.started_key.is_some() && self.started_key.as_ref() != Some(&self.key());
        if primary(ui, "▶  Simular").clicked() {
            self.start(ctx);
        }
        if stale {
            ui.colored_label(WARN, "O modelo mudou desde a última simulação.");
        }
        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new("Limite balístico (V50)").strong()).id_salt("v50").show(ui, |ui| self.ui_limit(ui, ctx));
    }

    fn quality_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Nós no diâmetro do projétil: mais nós = mais detalhe, mas bem mais lento.").size(12.5).color(Color32::from_gray(90)));
        ui.horizontal_wrapped(|ui| {
            for (l, n) in RES_PRESETS {
                if ui.add(egui::Button::new(format!("{l} · {n:.0}")).selected((self.res_nodes - n).abs() < 0.5).corner_radius(14.0)).clicked() {
                    self.res_nodes = n;
                }
            }
        });
        ui.add(egui::Slider::new(&mut self.res_nodes, 3.0..=48.0).logarithmic(true).text("nós").fixed_decimals(0));
        let n = self.estimate_nodes();
        let sc = self.scaled_cfg();
        let mem = sc.estimate_memory_mb();
        let free = pensim_core::usable_memory_mb();
        let thin = sc.design.layers.iter().map(|l| l.thickness).fold(f64::MAX, f64::min) / sc.dx;
        kv(ui, "Nós", format!("≈ {}", fmt_count(n)));
        kv(ui, "Memória", match free {
            Some(f) => format!("≈ {:.0} de {:.0} MB", mem, f),
            None => format!("≈ {:.0} MB", mem),
        });
        kv(ui, "Nós na camada mais fina", format!("{thin:.0}"));
        let (txt, col) = if free.map_or(false, |f| mem > 0.85 * f) {
            ("Não cabe na memória livre: diminua a qualidade.", BAD)
        } else if mem > 1500.0 || n > 120_000.0 {
            ("Muito pesado: pode demorar muito ou faltar memória.", BAD)
        } else if mem > 600.0 || n > 50_000.0 {
            ("Pesado para celular: pode levar vários minutos.", WARN)
        } else {
            ("Leve: adequado para celular.", GOOD)
        };
        ui.colored_label(col, txt);
        if self.res_nodes < 4.0 {
            ui.colored_label(WARN, "Menos de 4 nós no diâmetro: geometria grosseira.");
        }
        if thin < 3.0 {
            ui.colored_label(WARN, "Camada com menos de 3 nós de espessura: resultado pouco confiável.");
        }
    }

    // ------------------------------------------------------------ 4 · resultado

    fn step_results(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &Snap) {
        if let Some(e) = &snap.error {
            card(ui, "Não foi possível simular", |ui| self.memory_override(ui, e));
            return;
        }
        if self.job.is_some() && !snap.done {
            card(ui, "Calculando…", |ui| {
                let p = snap.progress.clamp(0.0, 1.0) as f32;
                let el = self.started_at.elapsed().as_secs_f64();
                let eta = if p > 0.03 { format!(" · faltam ~{}", fmt_secs(el * (1.0 - p as f64) / p as f64)) } else { String::new() };
                ui.add(egui::ProgressBar::new(p).text(format!("{:.0}%", p * 100.0)));
                ui.weak(format!("{} decorridos{eta} · {} nós", fmt_secs(el), fmt_count(snap.n as f64)));
                ui.horizontal(|ui| {
                    if let Some(j) = &self.job {
                        let paused = j.paused.load(Ordering::SeqCst);
                        if ui.button(if paused { "▶ Continuar" } else { "|| Pausar" }).clicked() {
                            j.paused.store(!paused, Ordering::SeqCst);
                        }
                    }
                    if ui.button("✕ Cancelar").clicked() {
                        self.cancel_job();
                    }
                    ui.checkbox(&mut self.live_view, "Ver ao vivo");
                });
                if ui.add_sized([ui.available_width(), 36.0], egui::Button::new(RichText::new("■ Parar aqui e ver o resultado").strong()).corner_radius(8.0)).on_hover_text("Encerra o cálculo agora; o resultado usa o que já foi simulado").clicked() {
                    if let Some(j) = &self.job {
                        j.finish.store(true, Ordering::SeqCst);
                        j.paused.store(false, Ordering::SeqCst);
                    }
                }
                ui.weak("Ao vivo você acompanha a animação; pare quando achar que já viu o suficiente. A reprodução completa começa sozinha no fim.");
            });
        }
        if snap.live.is_some() {
            // o resultado fica na barra da base da tela
        } else if self.job.is_none() && self.loaded.is_none() {
            card(ui, "", |ui| {
                ui.label("Nenhuma simulação ainda.");
            });
            self.next_btn(ui, Ws::Sim, "Ir para simular");
        }
        card(ui, "Visualização", |ui| {
            egui::ComboBox::from_id_salt("field").width(ui.available_width() - 8.0).selected_text(self.field.label()).show_ui(ui, |ui| {
                for f in Field::ALL {
                    ui.selectable_value(&mut self.field, f, f.label());
                }
            });
            ui.horizontal_wrapped(|ui| {
                chip(ui, &mut self.section, "◧ Corte");
                chip(ui, &mut self.faces, "◆ Faces");
                chip(ui, &mut self.bands, "▤ Faixas de cor");
                chip(ui, &mut self.cut_colored, "Corte colorido");
                chip(ui, &mut self.outline, "Contorno");
                chip(ui, &mut self.round_nodes, "Nós redondos");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Ocultar:");
                chip(ui, &mut self.hide_proj, "projétil");
                chip(ui, &mut self.hide_target, "alvo");
                chip(ui, &mut self.hide_loose, "fragmentos");
            });
        });
        egui::CollapsingHeader::new(RichText::new("Gráficos").strong()).id_salt("charts").show(ui, |ui| self.ui_charts(ui, snap));
        egui::CollapsingHeader::new(RichText::new("Salvar e abrir").strong()).id_salt("files").default_open(self.res_tab == ResTab::Files).show(ui, |ui| self.ui_files(ui, snap));
        ui.add_space(6.0);
        if snap.done && ui.add_sized([ui.available_width(), 36.0], egui::Button::new("↺ Simular de novo").corner_radius(8.0)).clicked() {
            self.start(ctx);
        }
    }

    /// Barra de resultado (base da tela): números principais sempre à vista.
    fn result_bar(&mut self, ui: &mut egui::Ui, s: &Snap) {
        if let Some(e) = &s.error {
            ui.colored_label(BAD, e);
            return;
        }
        let Some(o) = s.live.clone() else { return };
        let rs = self.started_scale.max(1e-6);
        let v0 = self.started_cfg.as_ref().map_or(self.cfg.velocity, |c| c.velocity);
        let (txt, col) = if !s.done {
            ("Calculando…", Color32::from_gray(110))
        } else {
            match o.kind {
                OutcomeKind::Perforation => ("Perfurou", BAD),
                OutcomeKind::PartialPenetration => ("Ficou cravado", WARN),
                OutcomeKind::Rejected => (if o.v_residual.abs() > 0.1 * v0 { "Ricocheteou" } else { "Rebateu / parou na frente" }, GOOD),
            }
        };
        let stat = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(k).size(11.0).color(Color32::from_gray(110)));
                ui.label(RichText::new(v).size(15.0).strong());
            });
            ui.add_space(6.0);
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(txt).size(17.0).strong().color(col));
            ui.add_space(8.0);
            stat(ui, "Vel. residual", format!("{:.0} m/s", o.v_residual));
            stat(ui, "Penetração", format!("{:.0} mm", o.penetration * 1e3 / rs));
            stat(ui, "Energia absorvida", format!("{:.0} %", o.energy_absorbed_pct));
            if ui.available_width() > 420.0 {
                stat(ui, "Impacto", format!("{v0:.0} m/s"));
            }
            if ui.add(egui::Button::new(if self.res_more { "▾ menos" } else { "▴ detalhes" }).corner_radius(12.0)).clicked() {
                self.res_more = !self.res_more;
            }
        });
        if self.res_more {
            ui.separator();
            egui::Grid::new("resbar_more").num_columns(4).spacing([16.0, 4.0]).show(ui, |ui| {
                let mut n = 0;
                let mut row = |ui: &mut egui::Ui, k: &str, v: String| {
                    ui.label(RichText::new(k).color(Color32::from_gray(100)));
                    ui.label(RichText::new(v).strong());
                    n += 1;
                    if n % 2 == 0 {
                        ui.end_row();
                    }
                };
                row(ui, "Projétil atrás do alvo", format!("{:.0} %", o.proj_behind_pct));
                row(ui, "Alvo ejetado (tampão)", format!("{:.1} %", o.target_ejected_pct));
                row(ui, "Duração simulada", format!("{:.0} µs", s.t_end * 1e6 / rs));
                row(ui, "Erro de energia", format!("{:+.2} %", o.energy_error_pct));
                row(ui, "Nós", fmt_count(s.n as f64));
                row(ui, "Passos", format!("{}", s.steps));
                row(ui, "Tempo de cálculo", fmt_secs(s.wall));
                row(ui, "Quadros", format!("{}", s.frames.len()));
            });
        }
    }

    fn playbar(&mut self, ui: &mut egui::Ui, s: &Snap) {
        let nf = s.frames.len();
        ui.horizontal(|ui| {
            let can = s.done;
            if ui.add_enabled(can, egui::Button::new(RichText::new(if self.playing { "⏸" } else { "▶" }).size(18.0)).min_size(Vec2::new(40.0, 34.0))).clicked() {
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
            ui.monospace(format!("{t:6.0} µs"));
            egui::ComboBox::from_id_salt("speed").width(58.0).selected_text(format!("{}×", self.speed)).show_ui(ui, |ui| {
                for sp in [0.1f32, 0.25, 0.5, 1.0, 2.0, 4.0] {
                    ui.selectable_value(&mut self.speed, sp, format!("{sp}×"));
                }
            });
            if ui.selectable_label(self.loop_play, "⟲").on_hover_text("Repetir em laço").clicked() {
                self.loop_play = !self.loop_play;
            }
            let mut i = self.view_idx;
            let w = (ui.available_width() - 8.0).max(60.0);
            ui.spacing_mut().slider_width = w;
            if ui.add(egui::Slider::new(&mut i, 0..=nf - 1).show_value(false)).changed() {
                self.view_idx = i;
                self.play_pos = i as f32;
                self.playing = false;
            }
        });
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
        let pname = if self.pdraw.is_some() { "Projétil desenhado" } else { AMMO[self.ammo_idx].name };
        let title = format!("{} · {:.0} m/s{}", pname, cfg.velocity, if cfg.obliquity_deg > 0.5 { format!(" · {:.0}°", cfg.obliquity_deg) } else { String::new() });
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
        self.clear_sim_cache();
        self.job_id += 1;
        self.cfg.design = d.design.clone();
        // desenhos à mão voltam a ser editáveis
        self.tdraws = d.design.objects.iter().map(|o| Drawing::from_polys(&o.name, o.width * 1e3, &o.parts, 1.0)).collect();
        self.tedit = None;
        let pr = &d.design.projectile;
        self.pdraw = if pr.regions.iter().any(|r| !r.poly.is_empty()) {
            let polys: Vec<(Vec<[f64; 2]>, pensim_core::Fill)> = pr.regions.iter().map(|r| (r.poly.clone(), if matches!(r.fill, pensim_core::Fill::Body) { pensim_core::Fill::Mat(pr.material.clone()) } else { r.fill.clone() })).collect();
            Some(Drawing::from_polys("Projétil desenhado", 0.0, &polys, 1.0))
        } else {
            None
        };
        self.ammo_scale = 1.0;
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
        line_plot(ui, 170.0, "Velocidade do projétil (parte que saiu pela traseira)", "t real [µs]", "m/s", &[Series { name: "vx", color: Color32::from_rgb(30, 110, 220), markers: false, pts: fr.iter().map(|f| (f.time * 1e6 / rs, f.vx_proj)).collect() }]);
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

    // --------------------------------------------------------- viewport 3D

    fn viewport(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        let t = std::time::Instant::now();
        self.viewport_inner(ui, snap);
        PROF_VIEW_US.store(t.elapsed().as_micros() as u64, Ordering::Relaxed);
    }

    fn viewport_inner(&mut self, ui: &mut egui::Ui, snap: &Snap) {
        if matches!(self.ws, Ws::Projectile | Ws::Target) {
            // alterna entre o desenho 2D e a malha 3D
            let r = ui.max_rect();
            let ctx = ui.ctx().clone();
            egui::Area::new(egui::Id::new("view2d3d")).order(egui::Order::Foreground).fixed_pos(r.right_top() + Vec2::new(-200.0, if self.peek3d { 40.0 } else { 46.0 })).show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Desenho 2D").strong()).selected(!self.peek3d).corner_radius(14.0)).clicked() {
                        self.peek3d = false;
                    }
                    if ui.add(egui::Button::new(RichText::new("Malha 3D").strong()).selected(self.peek3d).corner_radius(14.0)).clicked() {
                        self.peek3d = true;
                    }
                });
            });
        }
        if self.ws == Ws::Target && !self.peek3d {
            let u = self.ammo_scale.max(1e-6);
            if let Some(k) = self.tedit.filter(|&k| k < self.tdraws.len()) {
                // referência: chapas, projétil e os outros objetos
                let layers = self.cfg.design.layers.clone();
                let prof = self.cfg.design.projectile.profile.clone();
                let others: Vec<Vec<(Vec<[f64; 2]>, Paint)>> = self.tdraws.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, d)| d.sorted_fills()).collect();
                let bg = move |p: &egui::Painter, to: &dyn Fn(f64, f64) -> Pos2| {
                    let mut x = 0.0;
                    for l in &layers {
                        x += l.gap_before * 1e3 / u;
                        let (t, h) = (l.thickness * 1e3 / u, l.height * 1e3 / u);
                        let c = mat_color(l.material.name);
                        p.rect_filled(Rect::from_two_pos(to(x, h / 2.0), to(x + t, -h / 2.0)), 0.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 90));
                        x += t;
                    }
                    let pts: Vec<Pos2> = prof.iter().map(|q| to(-3.0 - q[0] * 1e3 / u, q[1] * 1e3 / u)).chain(prof.iter().rev().map(|q| to(-3.0 - q[0] * 1e3 / u, -q[1] * 1e3 / u))).collect();
                    p.add(egui::Shape::closed_line(pts, Stroke::new(1.0, Color32::from_rgb(120, 130, 150))));
                    for o in &others {
                        for (poly, c) in o {
                            let col = if let Paint::Mat(m) = c { mat_color(m.name) } else { Color32::GRAY };
                            p.add(egui::Shape::closed_line(poly.iter().map(|q| to(q[0], q[1])).collect(), Stroke::new(1.5, col)));
                        }
                    }
                };
                draw::show(ui, &mut self.tdraws[k], &mut self.ted, draw::Mode::Target, &|n| mat_color(n), &bg);
                return;
            }
            let colors: Vec<Color32> = self.cfg.design.layers.iter().map(|l| mat_color(l.material.name)).collect();
            let proj = self.cfg.design.projectile.profile.clone();
            let objs: Vec<Vec<(Vec<[f64; 2]>, Paint)>> = self.tdraws.iter().map(|d| d.sorted_fills()).collect();
            let extra = move |p: &egui::Painter, tor: &dyn Fn(f64, f64) -> Pos2| {
                for o in &objs {
                    for (poly, c) in o {
                        let col = if let Paint::Mat(m) = c { mat_color(m.name) } else { Color32::GRAY };
                        // desenho em mm reais → mm do modelo
                        p.add(egui::Shape::closed_line(poly.iter().map(|q| tor(q[0] * u, q[1] * u)).collect(), Stroke::new(2.0, col)));
                    }
                }
            };
            sketch::show_target(ui, &mut self.sketch, &mut self.cfg.design.layers, &proj, &colors, self.cfg.obliquity_deg, self.ammo_scale, &extra);
            return;
        }
        if self.ws == Ws::Projectile && !self.peek3d && self.pdraw.is_some() {
            draw::show(ui, self.pdraw.as_mut().unwrap(), &mut self.ped, draw::Mode::Projectile, &|n| mat_color(n), &|_, _| {});
            return;
        }
        if self.ws == Ws::Projectile && !self.peek3d {
            let fill = mat_color(self.cfg.design.projectile.material.name);
            let pr = self.cfg.design.projectile.clone();
            let cols: Vec<Color32> = self.cfg.design.materials().iter().map(|m| mat_color(m.name)).collect();
            sketch::show(ui, &mut self.sketch, &mut self.cfg.design.projectile.profile, fill, Some((&pr, &cols)), self.ammo_scale);
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
                        let (sa, ca) = (self.started_obl.to_radians().sin() as f32, self.started_obl.to_radians().cos() as f32);
                        // topologia fixa por simulação (cache)
                        let tkey = (self.job_id, n, masks.len(), dx.to_bits(), self.started_obl.to_bits());
                        let topo = match &self.topo_cache {
                            Some((k, t)) if *k == tkey => t.clone(),
                            _ => {
                                let t = Arc::new(crate::mesh::topo(n, &masks, &snap.ref_pos, &snap.parts, np8, dx, self.started_obl.to_radians() as f32, &flip));
                                self.topo_cache = Some((tkey, t.clone()));
                                t
                            }
                        };
                        let (hl, hp, ht, sec) = (self.hide_loose, self.hide_proj, self.hide_target, self.section);
                        let mut vis = vec![0u8; n * masks.len()];
                        for (mi, &mm) in masks.iter().enumerate() {
                            for i in 0..n {
                                let is_p = snap.parts[i] < np8;
                                vis[mi * n + i] = if (hl && f.damage[i] >= 0.9) || (is_p && hp) || (!is_p && ht) {
                                    0
                                } else if sec && !in_section(flip(snap.ref_pos[i], mm)[1]) {
                                    2
                                } else {
                                    1
                                };
                            }
                        }
                        let turn2 = move |q: [f32; 3]| [ca * q[0] - sa * q[2], q[1], -(sa * q[0] + ca * q[2])];
                        let m = crate::mesh::build(&topo, &f.pos, &vis, dx, 400_000, &flip, &turn2).map(Arc::new);
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
                    let lc = |i: usize| rainbow((data[i % n] * sc_f - lo) / (hi - lo), bands);
                    let loose = view3d::Loose { pos: &ext_pos, idx: &sm.loose, node_size: dx, tex: self.disc_tex.as_ref().map(|t| t.id()), color: &lc };
                    view3d::render_surf(&painter, &view, sm, ld, &|e, cut| {
                        if cut && !self.cut_colored {
                            mat_cols[(part_of[e % n] as usize).min(mat_cols.len() - 1)]
                        } else {
                            rainbow((data[e % n] * sc_f - lo) / (hi - lo), bands)
                        }
                    }, Some(loose));
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
                    let lc = |i: usize| mats[(parts[i % n] as usize).min(mats.len() - 1)];
                    let loose = view3d::Loose { pos: &ext_pos, idx: &sm.loose, node_size: dx, tex: self.disc_tex.as_ref().map(|t| t.id()), color: &lc };
                    view3d::render_surf(&painter, &view, sm, ld, &|e, _| mats[(parts[e % n] as usize).min(mats.len() - 1)], Some(loose));
                } else {
                    view3d::render(&painter, &view, dx, &ext_pos, &idx, true, Some(&light), round_id, &|i| mats[(parts[i % n] as usize).min(mats.len() - 1)]);
                }
                if self.outline && faces_mesh.is_none() {
                    let np = self.cfg.design.n_proj_parts() as u8;
                    view3d::outline(&painter, &view, dx, &ext_pos, &idx, &|i| if parts[i % n] >= np { 0 } else if f.damage[i % n] >= 0.6 { 2 } else { 1 }, Color32::from_rgb(20, 24, 36));
                }
            }
            painter.text(rect.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, format!("t = {:.0} µs", f.time * 1e6 / self.started_scale.max(1e-6)), FontId::monospace(15.0), Color32::from_gray(30));
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
        if self.ws == Ws::Results && self.job.is_some() && !snap.done && snap.error.is_none() {
            // progresso discreto no topo (sem cobrir o 3D)
            let p = snap.progress.clamp(0.0, 1.0) as f32;
            let el = self.started_at.elapsed().as_secs_f64();
            let eta = if p > 0.03 { format!(" · faltam ~{}", fmt_secs(el * (1.0 - p as f64) / p as f64)) } else { String::new() };
            let bar = Rect::from_min_size(rect.left_top(), Vec2::new(rect.width(), 4.0));
            painter.rect_filled(bar, 0.0, Color32::from_gray(220));
            painter.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * p, 4.0)), 0.0, ACCENT);
            painter.text(rect.left_top() + Vec2::new(10.0, 30.0), Align2::LEFT_TOP, format!("Calculando {:.0}%{eta}", p * 100.0), FontId::proportional(13.0), Color32::from_gray(50));
            let ctx3 = ui.ctx().clone();
            egui::Area::new(egui::Id::new("stop_here")).order(egui::Order::Foreground).fixed_pos(rect.left_top() + Vec2::new(10.0, 50.0)).show(&ctx3, |ui| {
                if ui.add(egui::Button::new(RichText::new("■ Parar aqui").strong()).corner_radius(14.0)).on_hover_text("Encerra o cálculo agora e mostra o resultado").clicked() {
                    if let Some(j) = &self.job {
                        j.finish.store(true, Ordering::SeqCst);
                        j.paused.store(false, Ordering::SeqCst);
                    }
                }
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
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

fn paint_combo(ui: &mut egui::Ui, id: &str, p: &mut Paint) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(200.0).selected_text(p.name()).show_ui(ui, |ui| {
        if ui.selectable_label(*p == Paint::Void, "Vazio (oco)").clicked() {
            *p = Paint::Void;
            changed = true;
        }
        for m in LIBRARY {
            if ui.selectable_label(p.name() == m.name, m.name).clicked() {
                *p = Paint::Mat(m.clone());
                changed = true;
            }
        }
    });
    changed
}

/// Ferramentas do editor de desenho livre (painel). Devolve `true` se o desenho mudou.
fn draw_tools(ui: &mut egui::Ui, d: &mut Drawing, ed: &mut DrawEditor, mode: draw::Mode) -> bool {
    let mut changed = false;
    let tname = match ed.tool {
        DTool::Line => "Desenhar",
        DTool::Connect => "Ligar pontos",
        DTool::Point => "Ponto",
        DTool::Move => "Mover",
        DTool::Split => "Dividir linha",
        DTool::Fillet => "Arredondar",
        DTool::Erase => "Apagar",
        DTool::Bucket => "Balde",
        DTool::Image => "Imagem de fundo",
    };
    ui.label(RichText::new(format!("Ferramenta: {tname}")).strong().color(ACCENT));
    ui.weak("Escolha a ferramenta na barra sobre o desenho.");
    match ed.tool {
        DTool::Image => {
            image_panel(ui, d, ed, &mut changed);
        }
        DTool::Bucket => {
            ui.horizontal(|ui| {
                ui.label("Material do balde:");
                paint_combo(ui, "bucket", &mut ed.paint);
            });
        }
        DTool::Fillet => {
            let mut apply = false;
            let mut cancel = false;
            if let Some(f) = ed.fillet.as_mut() {
                ui.add(egui::Slider::new(&mut f.n, 2..=40).text("pontos da curva"));
                ui.horizontal(|ui| {
                    apply = ui.button(RichText::new("✓ Aplicar").strong()).clicked();
                    cancel = ui.button("✕ Cancelar").clicked();
                });
            } else {
                ui.weak("Toque num canto do desenho (um ponto com duas linhas).");
            }
            if cancel {
                ed.fillet = None;
            }
            if apply {
                if let Some(f) = ed.fillet.take() {
                    ed.push(d);
                    d.apply_fillet(&f);
                    changed = true;
                }
            }
        }
        _ => {}
    }
    ui.horizontal(|ui| {
        ui.label("Zoom:");
        if ui.button(" − ").clicked() {
            if let Some(v) = ed.view.as_mut() {
                v[2] /= 1.3;
            }
        }
        if ui.button(" + ").clicked() {
            if let Some(v) = ed.view.as_mut() {
                v[2] *= 1.3;
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Grade:");
        for s in [0.1, 0.5, 1.0, 2.0, 5.0, 10.0] {
            if ui.selectable_label((ed.snap_mm - s).abs() < 1e-9, format!("{s} mm")).clicked() {
                ed.snap_mm = s;
            }
        }
    });
    if let Some(i) = ed.selected.filter(|&i| i > 0 && i < d.pts.len()) {
        let mut q = d.pts[i];
        ui.horizontal(|ui| {
            ui.label(format!("Ponto {i}:"));
            let (l0, l1) = if mode == draw::Mode::Projectile { ("dist. ", "raio ") } else { ("x ", "z ") };
            let r0 = ui.add(egui::DragValue::new(&mut q[0]).prefix(l0).suffix(" mm").speed(0.1).fixed_decimals(2));
            let lo = if mode == draw::Mode::Projectile { 0.0 } else { f64::MIN };
            let r1 = ui.add(egui::DragValue::new(&mut q[1]).prefix(l1).suffix(" mm").speed(0.1).range(lo..=f64::MAX).fixed_decimals(2));
            if r0.drag_started() || r1.drag_started() || r0.gained_focus() || r1.gained_focus() {
                ed.push(d);
            }
        });
        if q != d.pts[i] {
            d.pts[i] = q;
            changed = true;
        }
    }
    if !d.fills.is_empty() {
        ui.separator();
        ui.label(RichText::new("Peças pintadas").strong());
        let mut remove = None;
        for k in 0..d.fills.len() {
            ui.horizontal(|ui| {
                let col = match &d.fills[k].paint {
                    Paint::Mat(m) => mat_color(m.name),
                    Paint::Void => Color32::from_gray(220),
                };
                ui.colored_label(col, "■");
                if paint_combo(ui, &format!("fill{k}"), &mut d.fills[k].paint) {
                    changed = true;
                }
                if ui.small_button("✕").on_hover_text("Tirar a pintura").clicked() {
                    remove = Some(k);
                }
            });
        }
        if let Some(k) = remove {
            ed.push(d);
            d.fills.remove(k);
            changed = true;
        }
    }
    changed
}

/// Pasta de imagens de referência (no Android, a pasta do app visível no gerenciador de arquivos).
fn images_dir() -> std::path::PathBuf {
    #[cfg(target_os = "android")]
    let base = crate::ANDROID_APP.get().and_then(|a| a.external_data_path().or_else(|| a.internal_data_path()));
    #[cfg(not(target_os = "android"))]
    let base = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share/pensim"));
    let dir = base.unwrap_or_else(|| std::path::PathBuf::from(".")).join("imagens");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn image_files() -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = vec![];
    let mut dirs = vec![images_dir()];
    #[cfg(not(target_os = "android"))]
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(&h).join("Pictures"));
        dirs.push(std::path::PathBuf::from(&h).join("Imagens"));
        dirs.push(std::path::PathBuf::from(&h).join("Downloads"));
    }
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                let ext = p.extension().and_then(|x| x.to_str()).map(|x| x.to_lowercase()).unwrap_or_default();
                if matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
                    v.push(p);
                }
            }
        }
    }
    v.sort();
    v.truncate(60);
    v
}

/// Painel da ferramenta Imagem: carregar, ajustar e remover a imagem de fundo.
fn image_panel(ui: &mut egui::Ui, d: &mut Drawing, ed: &mut DrawEditor, changed: &mut bool) {
    ui.label(RichText::new("A imagem fica atrás de tudo, só como referência para você desenhar por cima.").size(12.5).color(Color32::from_gray(90)));
    // caminho digitado (útil no computador)
    let id = egui::Id::new("img_path");
    let mut path: String = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut path).hint_text("caminho da imagem (PNG ou JPEG)").desired_width(ui.available_width() - 84.0));
        if ui.button("Carregar").clicked() && !path.trim().is_empty() {
            ed.pending_load = Some(path.trim().trim_matches('"').to_string());
        }
    });
    ui.data_mut(|d| d.insert_temp(id, path));
    if !ed.img_err.is_empty() {
        ui.colored_label(BAD, &ed.img_err);
    }
    let files = image_files();
    egui::CollapsingHeader::new(format!("Imagens encontradas ({})", files.len())).id_salt("img_list").show(ui, |ui| {
        if files.is_empty() {
            ui.weak("Nenhuma imagem. Coloque arquivos PNG ou JPEG na pasta abaixo.");
        }
        for f in &files {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            if ui.button(name).clicked() {
                ed.pending_load = Some(f.to_string_lossy().to_string());
            }
        }
        ui.weak(format!("Pasta do app: {}", images_dir().display()));
    });
    if let Some(b) = d.bg.as_mut() {
        ui.separator();
        let before = b.clone();
        ui.add(egui::Slider::new(&mut b.width_mm, 1.0..=2000.0).logarithmic(true).clamping(egui::SliderClamping::Never).text("largura mm"));
        ui.add(egui::Slider::new(&mut b.rot_deg, -180.0..=180.0).suffix("°").text("giro"));
        ui.add(egui::Slider::new(&mut b.alpha, 0.1..=1.0).text("opacidade"));
        ui.horizontal(|ui| {
            ui.label("Posição:");
            ui.add(egui::DragValue::new(&mut b.center[0]).prefix("x ").suffix(" mm").speed(0.5));
            ui.add(egui::DragValue::new(&mut b.center[1]).prefix("y ").suffix(" mm").speed(0.5));
        });
        ui.horizontal(|ui| {
            if ui.button("↔ Espelhar").clicked() {
                b.flip = !b.flip;
            }
            if ui.button("Girar 90°").clicked() {
                b.rot_deg = (b.rot_deg + 90.0 + 180.0).rem_euclid(360.0) - 180.0;
            }
            if ui.button("✕ Remover imagem").clicked() {
                *changed = true;
                ed.bg_tex = None;
            }
        });
        if *b != before {
            *changed = true;
        }
        if ed.bg_tex.is_none() {
            d.bg = None;
        }
    }
}

/// Massa do projétil (kg) por amostragem do corte revolvido.
fn proj_mass(p: &pensim_core::Projectile) -> f64 {
    let (l, r) = (p.length(), p.max_radius());
    if l <= 0.0 || r <= 0.0 {
        return 0.0;
    }
    let mats: Vec<f64> = {
        let mut v = vec![p.material.rho];
        if let Some(c) = &p.core {
            v.push(c.material.rho);
        }
        for g in &p.regions {
            if let pensim_core::Fill::Mat(m) = &g.fill {
                v.push(m.rho);
            }
        }
        v
    };
    let (na, nr) = (240usize, 90usize);
    let (da, dr) = (l / na as f64, r / nr as f64);
    let mut m = 0.0;
    for i in 0..na {
        let a = (i as f64 + 0.5) * da;
        let ro = p.radius_at(a);
        for j in 0..nr {
            let rr = (j as f64 + 0.5) * dr;
            if rr > ro {
                break;
            }
            if let Some(k) = p.part_at(a, rr, 0.0) {
                m += mats.get(k as usize).copied().unwrap_or(0.0) * 2.0 * std::f64::consts::PI * rr * da * dr;
            }
        }
    }
    m
}

fn fmt_count(n: f64) -> String {
    if n >= 1e6 { format!("{:.1} mi", n / 1e6) } else if n >= 1e4 { format!("{:.0} mil", n / 1e3) } else { format!("{n:.0}") }
}

fn fmt_secs(s: f64) -> String {
    if s >= 3600.0 { format!("{:.0} h {:.0} min", (s / 3600.0).floor(), (s % 3600.0) / 60.0) } else if s >= 90.0 { format!("{:.0} min", s / 60.0) } else { format!("{s:.0} s") }
}

fn fmt_v(v: f32) -> String {
    if v.abs() >= 100.0 { format!("{v:.0}") } else if v.abs() >= 1.0 { format!("{v:.1}") } else { format!("{v:.2}") }
}
