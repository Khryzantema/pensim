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

/// Fragmentos soltos desenhados junto com as faces (mesma ordenação por profundidade).
pub struct Loose<'a> {
    pub pos: &'a [[f32; 3]],
    pub idx: &'a [u32],
    pub node_size: f32,
    pub tex: Option<egui::TextureId>,
    pub color: &'a dyn Fn(usize) -> Color32,
}

/// Desenha a malha de faces e os fragmentos soltos numa só passada ordenada por profundidade
/// (vértices compartilhados com Gouraud por canto, descarte de faces de costas).
pub fn render_surf(painter: &egui::Painter, v: &View, m: &crate::mesh::SurfMesh, ld: [f32; 3], color: &dyn Fn(usize, bool) -> Color32, loose: Option<Loose>) {
    let (_, _, e) = v.cam.basis();
    let proj: Vec<(Pos2, f32)> = m.verts.iter().map(|&p| v.project(p)).collect();
    // itens: (profundidade, índice) — índice < nq = face, senão fragmento
    let nq = m.quads.len() as u32;
    let mut order: Vec<(f32, u32)> = Vec::with_capacity(m.quads.len());
    let clip = v.rect.expand(40.0);
    for (qi, q) in m.quads.iter().enumerate() {
        let mut nn = [0.0f32; 3];
        let mut dep = 0.0;
        let mut inside = false;
        for &vi in &q.v {
            let n = m.normals[vi as usize];
            nn = [nn[0] + n[0], nn[1] + n[1], nn[2] + n[2]];
            let (p, d) = proj[vi as usize];
            dep += d;
            inside |= clip.contains(p);
        }
        let _ = nn;
        if !inside {
            continue; // fora da tela
        }
        order.push((dep * 0.25, qi as u32));
    }
    let mut lproj: Vec<(Pos2, u32)> = Vec::new();
    // Profundidade das faces numa grade de baixa resolução: a ordem por profundidade (pintor) usa
    // o centro de cada face, e um fragmento ATRÁS de uma face grande mas mais perto que o centro
    // dela seria pintado por cima ("atravessando" o objeto). Só desenha o fragmento se nenhuma
    // face estiver na frente dele. Maior d = mais perto da câmera.
    let zgrid = loose.as_ref().filter(|l| !l.idx.is_empty()).map(|l| {
        let cell = ((l.node_size * v.scale()) as f32).clamp(2.0, 6.0);
        let (gw, gh) = (((clip.width() / cell).ceil() as usize).max(1), ((clip.height() / cell).ceil() as usize).max(1));
        let mut z = vec![f32::NEG_INFINITY; gw * gh];
        let o = clip.min;
        let tri = |z: &mut Vec<f32>, a: (Pos2, f32), b: (Pos2, f32), c: (Pos2, f32)| {
            let (x0, x1) = (a.0.x.min(b.0.x).min(c.0.x), a.0.x.max(b.0.x).max(c.0.x));
            let (y0, y1) = (a.0.y.min(b.0.y).min(c.0.y), a.0.y.max(b.0.y).max(c.0.y));
            let den = (b.0.y - c.0.y) * (a.0.x - c.0.x) + (c.0.x - b.0.x) * (a.0.y - c.0.y);
            if den.abs() < 1e-6 {
                return;
            }
            let i0 = (((x0 - o.x) / cell).floor().max(0.0)) as usize;
            let i1 = (((x1 - o.x) / cell).ceil() as usize).min(gw);
            let j0 = (((y0 - o.y) / cell).floor().max(0.0)) as usize;
            let j1 = (((y1 - o.y) / cell).ceil() as usize).min(gh);
            for j in j0..j1 {
                let py = o.y + (j as f32 + 0.5) * cell;
                for i in i0..i1 {
                    let px = o.x + (i as f32 + 0.5) * cell;
                    let w0 = ((b.0.y - c.0.y) * (px - c.0.x) + (c.0.x - b.0.x) * (py - c.0.y)) / den;
                    let w1 = ((c.0.y - a.0.y) * (px - c.0.x) + (a.0.x - c.0.x) * (py - c.0.y)) / den;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let d = w0 * a.1 + w1 * b.1 + w2 * c.1;
                    let k = j * gw + i;
                    if d > z[k] {
                        z[k] = d;
                    }
                }
            }
        };
        for &(_, qi) in &order {
            let q = &m.quads[qi as usize].v;
            let p = q.map(|vi| proj[vi as usize]);
            tri(&mut z, p[0], p[1], p[2]);
            tri(&mut z, p[0], p[2], p[3]);
        }
        (z, gw, gh, cell, o, l.node_size as f32)
    });
    if let Some(l) = &loose {
        for &i in l.idx {
            let (p, d) = v.project(l.pos[i as usize]);
            if let Some((z, gw, gh, cell, o, r)) = &zgrid {
                let (gi, gj) = (((p.x - o.x) / cell).floor(), ((p.y - o.y) / cell).floor());
                if gi >= 0.0 && gj >= 0.0 && (gi as usize) < *gw && (gj as usize) < *gh && z[gj as usize * gw + gi as usize] > d + r {
                    continue; // escondido atrás de uma face
                }
            }
            if clip.contains(p) {
                order.push((d, nq + lproj.len() as u32));
                lproj.push((p, i));
            }
        }
    }
    order.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let (dmin, dmax) = order.first().map_or((0.0, 1.0), |f| (f.0, order.last().unwrap().0));
    let h = [ld[0] + e[0], ld[1] + e[1], ld[2] + e[2]];
    let hm = dot(h, h).sqrt().max(1e-6);
    let tex = loose.as_ref().and_then(|l| l.tex);
    // com a textura do disco, as faces usam um texel branco opaco dela
    let uv = if tex.is_some() { Pos2::new(0.35, 0.35) } else { egui::epaint::WHITE_UV };
    let mut mesh = match tex {
        Some(t) => egui::Mesh::with_texture(t),
        None => egui::Mesh::default(),
    };
    mesh.vertices = m
        .verts
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let n = m.normals[i];
            let dif = dot(n, ld).abs();
            let sp = (dot(n, h) / hm).max(0.0).powi(24);
            let depth_t = 0.78 + 0.22 * ((proj[i].1 - dmin) / (dmax - dmin).max(1e-9)).clamp(0.0, 1.0);
            let t = depth_t * (0.34 + 0.80 * dif + 0.22 * sp);
            let (mut ar, mut ag, mut ab, mut cn) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for &c in &m.vcells[i] {
                if c != u32::MAX {
                    let k = color(c as usize, m.vcut[i]);
                    ar += k.r() as f32;
                    ag += k.g() as f32;
                    ab += k.b() as f32;
                    cn += 1.0;
                }
            }
            let c0 = if cn > 0.0 { Color32::from_rgb((ar / cn) as u8, (ag / cn) as u8, (ab / cn) as u8) } else { color(m.vcell[i] as usize, m.vcut[i]) };
            let f = |ch: u8| ((ch as f32 * t).clamp(0.0, 255.0)) as u8;
            egui::epaint::Vertex { pos: proj[i].0, uv, color: Color32::from_rgb(f(c0.r()), f(c0.g()), f(c0.b())) }
        })
        .collect();
    mesh.indices.reserve(order.len() * 6);
    let half = loose.as_ref().map_or(1.0, |l| (l.node_size * v.scale() * if l.tex.is_some() { 0.95 } else { 0.7 }).max(0.6));
    let full_uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
    for (d, k) in order {
        if k < nq {
            let q = &m.quads[k as usize].v;
            mesh.indices.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
        } else if let Some(l) = &loose {
            let (p, i) = lproj[(k - nq) as usize];
            let depth_t = 0.72 + 0.28 * ((d - dmin) / (dmax - dmin).max(1e-9)).clamp(0.0, 1.0);
            let c = (l.color)(i as usize);
            let f = |ch: u8| ((ch as f32 * depth_t).clamp(0.0, 255.0)) as u8;
            let col = Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()));
            let r = Rect::from_center_size(p, Vec2::splat(half * 2.0));
            if l.tex.is_some() {
                mesh.add_rect_with_uv(r, full_uv, col);
            } else {
                let w = Rect::from_min_max(uv, uv);
                mesh.add_rect_with_uv(r, w, col);
            }
        }
    }
    painter.with_clip_rect(v.rect).add(egui::Shape::mesh(mesh));
}
