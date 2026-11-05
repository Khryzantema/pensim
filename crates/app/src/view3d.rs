//! Viewport 3D leve (projeção ortográfica, órbita, painter's algorithm).
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Response, Stroke, Vec2};

#[derive(Clone, Copy)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan: Vec2,
}

impl Camera {
    pub fn iso() -> Self {
        Camera { yaw: -60f32.to_radians(), pitch: 24f32.to_radians(), zoom: 1.0, pan: Vec2::ZERO }
    }
    pub fn front() -> Self {
        Camera { yaw: -90f32.to_radians(), pitch: 0.0, zoom: 1.0, pan: Vec2::ZERO }
    }
    pub fn top() -> Self {
        Camera { yaw: -90f32.to_radians(), pitch: 89f32.to_radians(), zoom: 1.0, pan: Vec2::ZERO }
    }
    pub fn right() -> Self {
        Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, pan: Vec2::ZERO }
    }
    pub fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (sy, cy, sp, cp) = (self.yaw.sin(), self.yaw.cos(), self.pitch.sin(), self.pitch.cos());
        let eye = [cp * cy, cp * sy, sp];
        let right = [-sy, cy, 0.0];
        let up = [-sp * cy, -sp * sy, cp];
        (right, up, eye)
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub struct View {
    pub cam: Camera,
    pub rect: Rect,
    pub center: [f32; 3],
    pub radius: f32,
}

impl View {
    pub fn scale(&self) -> f32 {
        self.rect.width().min(self.rect.height()) / (2.3 * self.radius.max(1e-6)) * self.cam.zoom
    }
    pub fn project(&self, p: [f32; 3]) -> (Pos2, f32) {
        let (r, u, e) = self.cam.basis();
        let d = [p[0] - self.center[0], p[1] - self.center[1], p[2] - self.center[2]];
        let s = self.scale();
        (
            self.rect.center() + self.cam.pan + Vec2::new(dot(d, r) * s, -dot(d, u) * s),
            dot(d, e),
        )
    }
}

pub fn background(painter: &egui::Painter, rect: Rect, dark: bool) {
    let (top, bot) = if dark {
        (Color32::from_rgb(38, 44, 54), Color32::from_rgb(18, 20, 26))
    } else {
        (Color32::from_rgb(196, 214, 236), Color32::from_rgb(246, 248, 252))
    };
    let mut m = egui::Mesh::default();
    m.colored_vertex(rect.left_top(), top);
    m.colored_vertex(rect.right_top(), top);
    m.colored_vertex(rect.right_bottom(), bot);
    m.colored_vertex(rect.left_bottom(), bot);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(m));
}

/// Desenha os nós em ordem de profundidade (longe → perto).
/// Textura de um disco suave com leve gradiente esférico (para nós arredondados).
pub fn disc_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let n = 48usize;
    let mut px = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let (u, w) = ((x as f32 + 0.5) / n as f32 * 2.0 - 1.0, (y as f32 + 0.5) / n as f32 * 2.0 - 1.0);
            let r = (u * u + w * w).sqrt();
            let a = ((1.0 - r) / 0.18).clamp(0.0, 1.0); // borda suave
            let hl = ((u + 0.30).powi(2) + (w + 0.30).powi(2)).sqrt(); // realce no canto superior esquerdo
            let sh = (1.05 - 0.38 * hl).clamp(0.62, 1.0);
            let g = (sh * 255.0) as u8;
            px.push(Color32::from_rgba_unmultiplied(g, g, g, (a * 255.0) as u8));
        }
    }
    ctx.load_texture("disc", egui::ColorImage { size: [n, n], source_size: egui::Vec2::new(n as f32, n as f32), pixels: px }, egui::TextureOptions::LINEAR)
}

pub fn render(painter: &egui::Painter, v: &View, node_size: f32, pos: &[[f32; 3]], idx: &[u32], shade: bool, light: Option<&dyn Fn(usize) -> f32>, round: Option<egui::TextureId>, color: &dyn Fn(usize) -> Color32) {
    let mut items: Vec<(f32, u32, Pos2)> = idx
        .iter()
        .map(|&i| {
            let (p, d) = v.project(pos[i as usize]);
            (d, i, p)
        })
        .collect();
    items.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let (dmin, dmax) = items.first().map_or((0.0, 1.0), |f| (f.0, items.last().unwrap().0));
    let half = (node_size * v.scale() * if round.is_some() { 0.74 } else { 0.62 }).max(0.6);
    let clip = v.rect.expand(half);
    let mut mesh = match round {
        Some(t) => egui::Mesh::with_texture(t),
        None => egui::Mesh::default(),
    };
    let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
    mesh.reserve_vertices(items.len() * 4);
    for (d, i, p) in items {
        if !clip.contains(p) {
            continue;
        }
        let mut c = color(i as usize);
        // profundidade (mais escuro ao fundo) × iluminação por normal, quando fornecida
        let depth_t = 0.72 + 0.28 * ((d - dmin) / (dmax - dmin).max(1e-9));
        let t = match light {
            Some(l) => depth_t * l(i as usize),
            None if shade => 0.68 + 0.32 * ((d - dmin) / (dmax - dmin).max(1e-9)),
            None => 1.0,
        };
        if t != 1.0 {
            let m = |ch: u8| ((ch as f32 * t).clamp(0.0, 255.0)) as u8;
            c = Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()));
        }
        if round.is_some() {
            mesh.add_rect_with_uv(Rect::from_center_size(p, Vec2::splat(half * 2.0)), uv, c);
        } else {
            mesh.add_colored_rect(Rect::from_center_size(p, Vec2::splat(half * 2.0)), c);
        }
    }
    painter.with_clip_rect(v.rect).add(egui::Shape::mesh(mesh));
}

/// Triedro de eixos (X voo, Y lateral, Z vertical) no canto.
pub fn triad(painter: &egui::Painter, v: &View) {
    let o = Pos2::new(v.rect.left() + 46.0, v.rect.bottom() - 46.0);
    let (r, u, _) = v.cam.basis();
    for (axis, col, name) in [
        ([1.0, 0.0, 0.0], Color32::from_rgb(220, 60, 50), "X"),
        ([0.0, 1.0, 0.0], Color32::from_rgb(60, 170, 70), "Y"),
        ([0.0, 0.0, 1.0], Color32::from_rgb(50, 100, 230), "Z"),
    ] {
        let a = [axis[0], axis[1], axis[2]];
        let p = o + Vec2::new(dot(a, r), -dot(a, u)) * 30.0;
        painter.line_segment([o, p], Stroke::new(2.5, col));
        painter.text(p + (p - o) * 0.25, Align2::CENTER_CENTER, name, FontId::proportional(12.0), col);
    }
}

/// Órbita (1 dedo / botão esquerdo), pan (2 dedos / botão direito), zoom (pinça / roda).
pub fn input(ui: &egui::Ui, resp: &Response, cam: &mut Camera) {
    let multi = ui.input(|i| i.multi_touch());
    if let Some(mt) = multi {
        cam.pan += mt.translation_delta;
        cam.zoom = (cam.zoom * mt.zoom_delta).clamp(0.2, 30.0);
        return;
    }
    if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
        cam.pan += resp.drag_delta();
    } else if resp.dragged_by(egui::PointerButton::Primary) {
        let d = resp.drag_delta();
        cam.yaw -= d.x * 0.008;
        cam.pitch = (cam.pitch + d.y * 0.008).clamp(-1.5, 1.5);
    }
    if resp.hovered() {
        let s = ui.input(|i| i.smooth_scroll_delta.y);
        let z = ui.input(|i| i.zoom_delta());
        cam.zoom = (cam.zoom * z * (1.0 + s * 0.002)).clamp(0.2, 30.0);
    }
}

/// Contorno externo (1 px) das partículas do projétil: mapa de ocupação em tela + profundidade do
/// alvo. O traço envolve cada partícula/silhueta sem cobrir sua cor, aparece só onde o projétil
/// está visível (não sobre o alvo que o encobre) e separa o projétil do alvo e do fundo.
/// `class(i)`: 0 = alvo (oclui), 1 = corpo do projétil (recebe contorno), 2 = ignorar (partícula solta).
pub fn outline(painter: &egui::Painter, v: &View, node_size: f32, pos: &[[f32; 3]], idx: &[u32], class: &dyn Fn(usize) -> u8, color: Color32) {
    const CELL: f32 = 1.0; // pontos de tela por célula
    let r = v.rect;
    let (gw, gh) = (((r.width() / CELL).ceil() as usize).max(3), ((r.height() / CELL).ceil() as usize).max(3));
    let radius = ((node_size * v.scale() * 0.74) / CELL).max(1.0);
    let rc = radius.ceil() as i32;
    let r2 = radius * radius + 0.5;
    let mut zt = vec![f32::NEG_INFINITY; gw * gh]; // profundidade do alvo mais próxima (maior = mais perto)
    let mut proj: Vec<(i32, i32, f32)> = Vec::new();
    for &i in idx {
        let (p, d) = v.project(pos[i as usize]);
        let (cx, cy) = (((p.x - r.left()) / CELL) as i32, ((p.y - r.top()) / CELL) as i32);
        let cl = class(i as usize);
        if cl == 2 {
            continue;
        }
        if cl == 1 {
            proj.push((cx, cy, d));
        } else {
            for dy in -rc..=rc {
                for dx in -rc..=rc {
                    let (x, y) = (cx + dx, cy + dy);
                    if x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh && (dx * dx + dy * dy) as f32 <= r2 {
                        let z = &mut zt[y as usize * gw + x as usize];
                        if d > *z {
                            *z = d;
                        }
                    }
                }
            }
        }
    }
    let eps = node_size * 0.5;
    let mut zp = vec![f32::NEG_INFINITY; gw * gh]; // profundidade do projétil visível (NEG_INFINITY = vazio)
    for (cx, cy, d) in proj {
        for dy in -rc..=rc {
            for dx in -rc..=rc {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh && (dx * dx + dy * dy) as f32 <= r2 {
                    let k = y as usize * gw + x as usize;
                    if d >= zt[k] - eps && d > zp[k] {
                        zp[k] = d;
                    }
                }
            }
        }
    }
    // anel externo: célula vazia vizinha (4-conexa) de célula ocupada; some onde o alvo está na frente
    let mut mesh = egui::Mesh::default();
    let occ = |k: usize| zp[k] > f32::NEG_INFINITY;
    for y in 1..gh - 1 {
        for x in 1..gw - 1 {
            let k = y * gw + x;
            if occ(k) {
                continue;
            }
            let mut zn = f32::NEG_INFINITY;
            for kk in [k - 1, k + 1, k - gw, k + gw] {
                if occ(kk) && zp[kk] > zn {
                    zn = zp[kk];
                }
            }
            if zn > f32::NEG_INFINITY && zt[k] <= zn + eps {
                let tl = Pos2::new(r.left() + x as f32 * CELL, r.top() + y as f32 * CELL);
                mesh.add_colored_rect(Rect::from_min_size(tl, Vec2::splat(CELL)), color);
            }
        }
    }
    painter.with_clip_rect(r).add(egui::Shape::mesh(mesh));
}
