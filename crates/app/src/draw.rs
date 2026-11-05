//! Editor de desenho livre: pontos, linhas e contornos fechados preenchidos com o balde.
//!
//! Usado para criar projéteis (perfil no plano a–r, revolvido no eixo) e objetos do alvo
//! (vista lateral x–z, extrudada na largura). Coordenadas em milímetros reais.
//! Regra das peças: o contorno preenchido mais interno (menor área) que contém o ponto vale.
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use pensim_core::{point_in_poly, poly_area, Fill, Material, Projectile, Region, TargetObject};

#[derive(Clone, PartialEq, Debug)]
pub enum Paint {
    Void,
    Mat(Material),
}

impl Paint {
    pub fn name(&self) -> &str {
        match self {
            Paint::Void => "Vazio",
            Paint::Mat(m) => m.name,
        }
    }
    fn fill(&self) -> Fill {
        match self {
            Paint::Void => Fill::Void,
            Paint::Mat(m) => Fill::Mat(m.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FillRec {
    /// contorno fechado (índices dos pontos, em ordem)
    pub cycle: Vec<usize>,
    pub paint: Paint,
}

#[derive(Clone, Debug)]
pub struct Drawing {
    pub name: String,
    pub pts: Vec<[f64; 2]>,
    pub edges: Vec<[usize; 2]>,
    pub fills: Vec<FillRec>,
    /// largura da extrusão (objetos do alvo) [mm]
    pub width_mm: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Projectile,
    Target,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DTool {
    Line,
    Point,
    Move,
    Split,
    Erase,
    Bucket,
    Fillet,
}

/// Arredondamento de um canto: o canto `p` entre as linhas p–a e p–b vira uma curva de Bézier
/// que começa a `d1` de p sobre p–a, termina a `d2` sobre p–b, com ponto de controle `q`.
#[derive(Clone, Debug)]
pub struct Fillet {
    pub p: usize,
    pub a: usize,
    pub b: usize,
    pub d1: f64,
    pub d2: f64,
    pub q: [f64; 2],
    /// pontos criados (incluindo as duas pontas)
    pub n: usize,
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn len(a: [f64; 2]) -> f64 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

impl Fillet {
    fn ends(&self, d: &Drawing) -> ([f64; 2], [f64; 2]) {
        let (p, a, b) = (d.pts[self.p], d.pts[self.a], d.pts[self.b]);
        let (ua, ub) = (sub(a, p), sub(b, p));
        let (la, lb) = (len(ua).max(1e-12), len(ub).max(1e-12));
        ([p[0] + ua[0] / la * self.d1, p[1] + ua[1] / la * self.d1], [p[0] + ub[0] / lb * self.d2, p[1] + ub[1] / lb * self.d2])
    }
    /// pontos da curva, de S (lado a) até E (lado b)
    pub fn curve(&self, d: &Drawing) -> Vec<[f64; 2]> {
        let (s0, e0) = self.ends(d);
        let n = self.n.max(2);
        (0..n)
            .map(|i| {
                let t = i as f64 / (n - 1) as f64;
                let (w0, w1, w2) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
                [w0 * s0[0] + w1 * self.q[0] + w2 * e0[0], w0 * s0[1] + w1 * self.q[1] + w2 * e0[1]]
            })
            .collect()
    }
}

pub struct DrawEditor {
    pub tool: DTool,
    pub current: Option<usize>,
    pub selected: Option<usize>,
    pub paint: Paint,
    pub undo: Vec<Drawing>,
    pub snap_mm: f64,
    /// centro (u, v) [mm] e escala [px/mm]
    pub view: Option<[f64; 3]>,
    pub fit: bool,
    pub msg: String,
    pub fillet: Option<Fillet>,
    drag: Option<usize>,
    /// alça do arredondamento em arraste: 1 = início, 2 = fim, 3 = meio
    fdrag: u8,
    panning: bool,
}

impl DrawEditor {
    pub fn new(paint: Paint, snap_mm: f64) -> Self {
        DrawEditor { tool: DTool::Line, current: Some(0), selected: None, paint, undo: vec![], snap_mm, view: None, fit: true, msg: String::new(), fillet: None, drag: None, fdrag: 0, panning: false }
    }
    pub fn push(&mut self, d: &Drawing) {
        self.undo.push(d.clone());
        if self.undo.len() > 80 {
            self.undo.remove(0);
        }
    }
    pub fn undo(&mut self, d: &mut Drawing) -> bool {
        if let Some(prev) = self.undo.pop() {
            *d = prev;
            self.current = None;
            self.selected = None;
            true
        } else {
            false
        }
    }
}

impl Drawing {
    pub fn new(name: &str, width_mm: f64) -> Self {
        Drawing { name: name.into(), pts: vec![[0.0, 0.0]], edges: vec![], fills: vec![], width_mm }
    }

    pub fn add_point(&mut self, p: [f64; 2]) -> usize {
        self.pts.push(p);
        self.pts.len() - 1
    }

    pub fn add_edge(&mut self, u: usize, v: usize) {
        if u != v && !self.edges.iter().any(|e| (e[0] == u && e[1] == v) || (e[0] == v && e[1] == u)) {
            self.edges.push([u, v]);
        }
    }

    fn uses_edge(c: &[usize], u: usize, v: usize) -> bool {
        let n = c.len();
        (0..n).any(|i| {
            let (a, b) = (c[i], c[(i + 1) % n]);
            (a == u && b == v) || (a == v && b == u)
        })
    }

    pub fn remove_point(&mut self, i: usize) {
        if i == 0 || i >= self.pts.len() {
            return;
        }
        self.edges.retain(|e| e[0] != i && e[1] != i);
        self.fills.retain(|f| !f.cycle.contains(&i));
        self.pts.remove(i);
        let fix = |k: usize| if k > i { k - 1 } else { k };
        for e in &mut self.edges {
            e[0] = fix(e[0]);
            e[1] = fix(e[1]);
        }
        for f in &mut self.fills {
            for k in &mut f.cycle {
                *k = fix(*k);
            }
        }
    }

    pub fn remove_edge(&mut self, k: usize) {
        let [u, v] = self.edges.remove(k);
        self.fills.retain(|f| !Self::uses_edge(&f.cycle, u, v));
    }

    /// Divide a linha `k` com um ponto no meio; devolve o novo ponto.
    pub fn split_edge(&mut self, k: usize) -> usize {
        let [u, v] = self.edges[k];
        let (a, b) = (self.pts[u], self.pts[v]);
        let m = self.add_point([0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]);
        self.edges[k] = [u, m];
        self.edges.push([m, v]);
        for f in &mut self.fills {
            let n = f.cycle.len();
            let mut out = Vec::with_capacity(n + 1);
            for i in 0..n {
                let (x, y) = (f.cycle[i], f.cycle[(i + 1) % n]);
                out.push(x);
                if (x == u && y == v) || (x == v && y == u) {
                    out.push(m);
                }
            }
            f.cycle = out;
        }
        m
    }

    pub fn poly(&self, c: &[usize]) -> Vec<[f64; 2]> {
        c.iter().map(|&i| self.pts[i]).collect()
    }

    /// Faces limitadas do grafo plano (contornos fechados), percorridas no sentido anti-horário.
    pub fn faces(&self) -> Vec<Vec<usize>> {
        let n = self.pts.len();
        let mut nb: Vec<Vec<usize>> = vec![vec![]; n];
        for e in &self.edges {
            nb[e[0]].push(e[1]);
            nb[e[1]].push(e[0]);
        }
        let ang = |p: [f64; 2], q: [f64; 2]| (q[1] - p[1]).atan2(q[0] - p[0]);
        for v in 0..n {
            let p = self.pts[v];
            let pts = &self.pts;
            nb[v].sort_by(|&a, &b| ang(p, pts[a]).total_cmp(&ang(p, pts[b])));
        }
        let mut used = std::collections::HashSet::new();
        let mut out = vec![];
        for u in 0..n {
            for &v in &nb[u] {
                if used.contains(&(u, v)) {
                    continue;
                }
                let (mut a, mut b) = (u, v);
                let mut cyc = vec![];
                for _ in 0..(2 * self.edges.len() + 2) {
                    used.insert((a, b));
                    cyc.push(a);
                    let m = nb[b].len();
                    let k = nb[b].iter().position(|&x| x == a).unwrap_or(0);
                    let w = nb[b][(k + m - 1) % m];
                    a = b;
                    b = w;
                    if (a, b) == (u, v) {
                        break;
                    }
                }
                if poly_area(&self.poly(&cyc)) > 1e-9 {
                    out.push(cyc);
                }
            }
        }
        out
    }

    /// Balde: pinta o menor contorno fechado que contém o ponto. `false` se não houver.
    pub fn bucket(&mut self, u: f64, v: f64, paint: Paint) -> bool {
        let mut best: Option<(f64, Vec<usize>)> = None;
        for f in self.faces() {
            let poly = self.poly(&f);
            let a = poly_area(&poly).abs();
            if point_in_poly(&poly, u, v) && best.as_ref().map_or(true, |b| a < b.0) {
                best = Some((a, f));
            }
        }
        let Some((_, cyc)) = best else { return false };
        let mut key = cyc.clone();
        key.sort_unstable();
        key.dedup();
        for f in &mut self.fills {
            let mut k2 = f.cycle.clone();
            k2.sort_unstable();
            k2.dedup();
            if k2 == key {
                f.paint = paint;
                return true;
            }
        }
        self.fills.push(FillRec { cycle: cyc, paint });
        true
    }

    /// Contornos preenchidos em ordem de área decrescente (o último que contém o ponto vale).
    pub fn sorted_fills(&self) -> Vec<(Vec<[f64; 2]>, Paint)> {
        let mut v: Vec<(f64, Vec<[f64; 2]>, Paint)> = self.fills.iter().map(|f| {
            let p = self.poly(&f.cycle);
            (poly_area(&p).abs(), p, f.paint.clone())
        }).collect();
        v.sort_by(|a, b| b.0.total_cmp(&a.0));
        v.into_iter().map(|(_, p, c)| (p, c)).collect()
    }

    pub fn paint_at(&self, sorted: &[(Vec<[f64; 2]>, Paint)], u: f64, v: f64) -> Option<Paint> {
        let mut hit = None;
        for (p, c) in sorted {
            if point_in_poly(p, u, v) {
                hit = Some(c.clone());
            }
        }
        hit
    }

    /// Reconstrói um desenho a partir de contornos já convertidos (metros × `u`).
    pub fn from_polys(name: &str, width_mm: f64, polys: &[(Vec<[f64; 2]>, Fill)], u: f64) -> Self {
        let mut d = Drawing::new(name, width_mm);
        let k = 1e3 / u.max(1e-12);
        for (poly, fill) in polys {
            let ids: Vec<usize> = poly.iter().map(|q| {
                let p = [q[0] * k, q[1] * k];
                match d.pts.iter().position(|e| (e[0] - p[0]).abs() < 1e-6 && (e[1] - p[1]).abs() < 1e-6) {
                    Some(i) => i,
                    None => d.add_point(p),
                }
            }).collect();
            for i in 0..ids.len() {
                d.add_edge(ids[i], ids[(i + 1) % ids.len()]);
            }
            let paint = match fill {
                Fill::Mat(m) => Paint::Mat(m.clone()),
                _ => Paint::Void,
            };
            d.fills.push(FillRec { cycle: ids, paint });
        }
        d
    }

    /// Projétil (perfil no plano a–r) em metros × `u`. A ponta (menor a) vira a = 0.
    pub fn to_projectile(&self, u: f64) -> Option<Projectile> {
        let fills = self.sorted_fills();
        let first_mat = fills.iter().position(|f| matches!(f.1, Paint::Mat(_)))?;
        let amin = fills.iter().flat_map(|f| f.0.iter()).map(|p| p[0]).fold(f64::MAX, f64::min);
        let k = u * 1e-3;
        let polys: Vec<(Vec<[f64; 2]>, Paint)> = fills.into_iter().map(|(p, c)| (p.into_iter().map(|q| [(q[0] - amin) * k, q[1].max(0.0) * k]).collect(), c)).collect();
        let len = polys.iter().flat_map(|f| f.0.iter()).map(|p| p[0]).fold(0.0, f64::max);
        if len <= 1e-9 {
            return None;
        }
        // perfil externo (envelope) para a malha: maior raio com material em cada a
        let mut samples: Vec<f64> = (0..=120).map(|i| len * i as f64 / 120.0).collect();
        for (p, c) in &polys {
            if matches!(c, Paint::Mat(_)) {
                samples.extend(p.iter().map(|q| q[0]));
            }
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        samples.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let env = |a: f64| -> f64 {
            let mut r: f64 = 0.0;
            for (p, c) in &polys {
                if !matches!(c, Paint::Mat(_)) {
                    continue;
                }
                let n = p.len();
                for i in 0..n {
                    let (x, y) = (p[i], p[(i + 1) % n]);
                    let (lo, hi) = (x[0].min(y[0]), x[0].max(y[0]));
                    if a >= lo - 1e-12 && a <= hi + 1e-12 {
                        let rr = if (y[0] - x[0]).abs() < 1e-15 { x[1].max(y[1]) } else { x[1] + (a - x[0]) / (y[0] - x[0]) * (y[1] - x[1]) };
                        r = r.max(rr);
                    }
                }
            }
            r
        };
        let profile: Vec<[f64; 2]> = samples.iter().map(|&a| [a, env(a) + 1e-9]).collect();
        let Paint::Mat(body) = polys[first_mat].1.clone() else { return None };
        let regions = polys
            .into_iter()
            .enumerate()
            .map(|(i, (poly, c))| Region { name: c.name().to_string(), outer: vec![], inner: vec![], fill: if i == first_mat { Fill::Body } else { c.fill() }, poly })
            .collect();
        Some(Projectile { profile, material: body, core: None, regions })
    }

    /// Objeto do alvo (vista lateral x–z) em metros × `u`.
    pub fn to_object(&self, u: f64) -> TargetObject {
        let k = u * 1e-3;
        let parts = self.sorted_fills().into_iter().map(|(p, c)| (p.into_iter().map(|q| [q[0] * k, q[1] * k]).collect(), c.fill())).collect();
        TargetObject { name: self.name.clone(), parts, width: self.width_mm * k }
    }

    /// Começa um arredondamento no ponto `p` (precisa ter exatamente duas linhas).
    pub fn start_fillet(&self, p: usize) -> Result<Fillet, &'static str> {
        if p == 0 {
            return Err("O ponto de origem não pode ser arredondado.");
        }
        let nb: Vec<usize> = self.edges.iter().filter_map(|e| if e[0] == p { Some(e[1]) } else if e[1] == p { Some(e[0]) } else { None }).collect();
        if nb.len() != 2 {
            return Err("Escolha um canto com exatamente duas linhas.");
        }
        let (a, b) = (nb[0], nb[1]);
        let m = len(sub(self.pts[a], self.pts[p])).min(len(sub(self.pts[b], self.pts[p])));
        Ok(Fillet { p, a, b, d1: 0.4 * m, d2: 0.4 * m, q: self.pts[p], n: 7 })
    }

    pub fn apply_fillet(&mut self, f: &Fillet) {
        let pts = f.curve(self);
        let ids: Vec<usize> = pts.iter().map(|&q| self.add_point(q)).collect();
        let (p, a, b) = (f.p, f.a, f.b);
        self.edges.retain(|e| !((e[0] == p && (e[1] == a || e[1] == b)) || (e[1] == p && (e[0] == a || e[0] == b))));
        self.add_edge(a, ids[0]);
        for w in ids.windows(2) {
            self.add_edge(w[0], w[1]);
        }
        self.add_edge(*ids.last().unwrap(), b);
        // contornos pintados: troca o canto pela curva, na mesma ordem de percurso
        for fl in &mut self.fills {
            let n = fl.cycle.len();
            if let Some(i) = fl.cycle.iter().position(|&k| k == p) {
                let (prev, next) = (fl.cycle[(i + n - 1) % n], fl.cycle[(i + 1) % n]);
                let seq: Vec<usize> = if prev == a && next == b { ids.clone() } else if prev == b && next == a { ids.iter().rev().cloned().collect() } else { continue };
                fl.cycle.splice(i..=i, seq);
            }
        }
        if !self.edges.iter().any(|e| e[0] == p || e[1] == p) {
            self.remove_point(p);
        }
    }

    fn bounds(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
        for p in &self.pts {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].max(p[0]);
            b[2] = b[2].min(p[1]);
            b[3] = b[3].max(p[1]);
        }
        b
    }
}

/// Triangulação por "orelhas" de um polígono simples (côncavo ou convexo). Pontos repetidos
/// e "espinhos" (linhas soltas percorridas ida e volta) são ignorados.
pub fn triangulate(poly: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return vec![];
    }
    let ccw = poly_area(poly) > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    // remove repetidos consecutivos
    idx.dedup_by(|a, b| (poly[*a][0] - poly[*b][0]).abs() < 1e-12 && (poly[*a][1] - poly[*b][1]).abs() < 1e-12);
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let inside = |p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
        let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(neg && pos)
    };
    let mut out = vec![];
    let mut guard = 0;
    while idx.len() > 3 && guard < 10 * n * n + 100 {
        guard += 1;
        let m = idx.len();
        let mut cut = false;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (poly[ia], poly[ib], poly[ic]);
            let cr = cross(a, b, c);
            let convex = if ccw { cr > 1e-14 } else { cr < -1e-14 };
            if cr.abs() <= 1e-14 {
                // colinear ou espinho: tira o vértice do meio sem gerar triângulo
                idx.remove(k);
                cut = true;
                break;
            }
            if !convex {
                continue;
            }
            let blocked = idx.iter().any(|&j| j != ia && j != ib && j != ic && {
                let p = poly[j];
                (p != a && p != b && p != c) && inside(p, a, b, c)
            });
            if blocked {
                continue;
            }
            out.push([ia, ib, ic]);
            idx.remove(k);
            cut = true;
            break;
        }
        if !cut {
            break;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    } else if idx.len() > 3 {
        // polígono que se cruza: leque como último recurso
        for k in 1..idx.len() - 1 {
            out.push([idx[0], idx[k], idx[k + 1]]);
        }
    }
    out
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Desenha e edita. `bg` desenha a referência (chapas, projétil) com a mesma transformação.
/// Devolve `true` se o desenho mudou.
pub fn show(
    ui: &mut egui::Ui,
    d: &mut Drawing,
    ed: &mut DrawEditor,
    mode: Mode,
    mat_color: &dyn Fn(&str) -> Color32,
    bg: &dyn Fn(&egui::Painter, &dyn Fn(f64, f64) -> Pos2),
) -> bool {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, Color32::from_rgb(250, 251, 253));
    let sx: f64 = if mode == Mode::Projectile { -1.0 } else { 1.0 };
    if ed.view.is_none() || ed.fit {
        let mut b = d.bounds();
        let (minw, minh) = if mode == Mode::Projectile { (40.0, 14.0) } else { (60.0, 60.0) };
        if b[1] - b[0] < minw {
            let c = if mode == Mode::Projectile { b[0] + minw * 0.5 - 4.0 } else { 0.5 * (b[0] + b[1]) };
            b[0] = c - minw * 0.5;
            b[1] = c + minw * 0.5;
        }
        if mode == Mode::Projectile {
            b[2] = -b[3].max(minh * 0.5);
            b[3] = b[3].max(minh * 0.5);
        } else if b[3] - b[2] < minh {
            let c = 0.5 * (b[2] + b[3]);
            b[2] = c - minh * 0.5;
            b[3] = c + minh * 0.5;
        }
        let s = ((rect.width() as f64 - 60.0) / (b[1] - b[0])).min((rect.height() as f64 - 70.0) / (b[3] - b[2])).max(0.05);
        ed.view = Some([0.5 * (b[0] + b[1]), 0.5 * (b[2] + b[3]), s]);
        ed.fit = false;
    }
    let [mut cu, mut cv, mut s] = ed.view.unwrap();
    // zoom: pinça / ctrl+roda / roda
    if resp.hovered() {
        let (z, sc) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y));
        let z = z as f64 * (1.0 + sc as f64 * 0.002).clamp(0.8, 1.25);
        if (z - 1.0).abs() > 1e-4 {
            s = (s * z).clamp(0.05, 400.0);
        }
    }
    let c = rect.center();
    let to = |u: f64, v: f64| Pos2::new(c.x + (sx * (u - cu) * s) as f32, c.y - ((v - cv) * s) as f32);
    let from = |p: Pos2| (cu + sx * (p.x - c.x) as f64 / s, cv - (p.y - c.y) as f64 / s);

    // grade
    let mut step = ed.snap_mm.max(0.1);
    while step * s < 9.0 {
        step *= 5.0;
    }
    let (u0, v1) = from(rect.left_top());
    let (u1, v0) = from(rect.right_bottom());
    let (ua, ub) = (u0.min(u1), u0.max(u1));
    let minor = Stroke::new(1.0, Color32::from_rgb(234, 237, 243));
    let major = Stroke::new(1.0, Color32::from_rgb(212, 218, 228));
    let mut k = (ua / step).floor() as i64;
    while (k as f64) * step <= ub && k < (ua / step).floor() as i64 + 2000 {
        let x = to(k as f64 * step, 0.0).x;
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], if k % 5 == 0 { major } else { minor });
        k += 1;
    }
    let mut k = (v0 / step).floor() as i64;
    while (k as f64) * step <= v1 && k < (v0 / step).floor() as i64 + 2000 {
        let y = to(0.0, k as f64 * step).y;
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], if k % 5 == 0 { major } else { minor });
        k += 1;
    }
    let axis_y = to(0.0, 0.0).y;
    painter.line_segment([Pos2::new(rect.left(), axis_y), Pos2::new(rect.right(), axis_y)], Stroke::new(1.5, Color32::from_rgb(200, 60, 60)));
    painter.text(Pos2::new(rect.left() + 6.0, axis_y - 3.0), Align2::LEFT_BOTTOM, if mode == Mode::Projectile { "eixo de revolução" } else { "linha de tiro" }, FontId::proportional(11.0), Color32::from_rgb(200, 60, 60));

    bg(&painter, &to);

    // preenchimentos em vetor: cada contorno triangulado, do maior para o menor
    // (o mais interno é pintado por cima, que é a mesma regra da simulação)
    let fills = d.sorted_fills();
    {
        let mut mesh = egui::Mesh::default();
        let mut add_poly = |poly: &[[f64; 2]], col: Color32, mirror: bool| {
            let tris = triangulate(poly);
            let base = mesh.vertices.len() as u32;
            for q in poly {
                let pp = if mirror { to(q[0], -q[1]) } else { to(q[0], q[1]) };
                mesh.colored_vertex(pp, col);
            }
            for t in tris {
                mesh.add_triangle(base + t[0] as u32, base + t[1] as u32, base + t[2] as u32);
            }
        };
        let colors: Vec<Color32> = fills
            .iter()
            .map(|(_, pnt)| match pnt {
                Paint::Void => Color32::from_rgb(250, 251, 253),
                Paint::Mat(m) => mat_color(m.name),
            })
            .collect();
        if mode == Mode::Projectile {
            // metade espelhada, mais clara, só para dar a ideia do sólido
            for ((poly, _), c) in fills.iter().zip(&colors) {
                let l = |x: u8| ((x as f32) * 0.5 + 127.0) as u8;
                add_poly(poly, Color32::from_rgb(l(c.r()), l(c.g()), l(c.b())), true);
            }
        }
        for ((poly, _), c) in fills.iter().zip(&colors) {
            add_poly(poly, *c, false);
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    // interação
    let ptr = resp.interact_pointer_pos().or(resp.hover_pos());
    let hit_point = |d: &Drawing, p: Pos2| -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, q) in d.pts.iter().enumerate() {
            let dd = to(q[0], q[1]).distance(p);
            if dd < 20.0 && best.map_or(true, |b| dd < b.1) {
                best = Some((i, dd));
            }
        }
        best.map(|b| b.0)
    };
    let hit_edge = |d: &Drawing, p: Pos2| -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (k, e) in d.edges.iter().enumerate() {
            let (a, b) = (d.pts[e[0]], d.pts[e[1]]);
            let dd = seg_dist(p, to(a[0], a[1]), to(b[0], b[1]));
            if dd < 14.0 && best.map_or(true, |bb| dd < bb.1) {
                best = Some((k, dd));
            }
        }
        best.map(|b| b.0)
    };
    let snap_mm = ed.snap_mm;
    let snap = |p: Pos2| -> [f64; 2] {
        let (u, v) = from(p);
        let g = snap_mm.max(0.01);
        let mut q = [(u / g).round() * g, (v / g).round() * g];
        if (v * s).abs() < 10.0 {
            q[1] = 0.0;
        }
        if mode == Mode::Projectile {
            q[1] = q[1].max(0.0);
        }
        q
    };
    let mut changed = false;
    let push = |ed: &mut DrawEditor, d: &Drawing| ed.push(d);
    if resp.clicked() {
        if let Some(p) = ptr {
            ed.msg.clear();
            let hp = hit_point(d, p);
            let he = if hp.is_none() { hit_edge(d, p) } else { None };
            match ed.tool {
                DTool::Line => match (hp, ed.current) {
                    (Some(i), None) => ed.current = Some(i),
                    (Some(i), Some(cur)) if cur == i => ed.current = None,
                    (Some(i), Some(cur)) => {
                        push(ed, d);
                        d.add_edge(cur, i);
                        ed.current = Some(i);
                        changed = true;
                    }
                    (None, cur) => {
                        push(ed, d);
                        let id = d.add_point(snap(p));
                        if let Some(cu) = cur {
                            d.add_edge(cu, id);
                        }
                        ed.current = Some(id);
                        changed = true;
                    }
                },
                DTool::Point => match hp {
                    Some(i) => ed.selected = Some(i),
                    None => {
                        push(ed, d);
                        let id = d.add_point(snap(p));
                        ed.selected = Some(id);
                        changed = true;
                    }
                },
                DTool::Move => ed.selected = hp,
                DTool::Split => {
                    if let Some(k) = he {
                        push(ed, d);
                        let m = d.split_edge(k);
                        ed.selected = Some(m);
                        changed = true;
                    }
                }
                DTool::Erase => {
                    if let Some(i) = hp {
                        if i == 0 {
                            ed.msg = "O ponto de origem não pode ser apagado.".into();
                        } else {
                            push(ed, d);
                            d.remove_point(i);
                            ed.current = None;
                            ed.selected = None;
                            changed = true;
                        }
                    } else if let Some(k) = he {
                        push(ed, d);
                        d.remove_edge(k);
                        changed = true;
                    }
                }
                DTool::Fillet => {
                    if let Some(i) = hp {
                        match d.start_fillet(i) {
                            Ok(f) => ed.fillet = Some(f),
                            Err(e) => ed.msg = e.into(),
                        }
                    }
                }
                DTool::Bucket => {
                    let (u, v) = from(p);
                    let v = if mode == Mode::Projectile { v.abs() } else { v };
                    let before = d.clone();
                    if d.bucket(u, v, ed.paint.clone()) {
                        ed.push(&before);
                        changed = true;
                    } else {
                        ed.msg = "Toque dentro de um contorno fechado (feche o desenho com linhas).".into();
                    }
                }
            }
        }
    }
    // alças do arredondamento (posições na tela)
    let fh = ed.fillet.as_ref().filter(|f| f.p < d.pts.len() && f.a < d.pts.len() && f.b < d.pts.len()).map(|f| {
        let (s0, e0) = f.ends(d);
        (to(s0[0], s0[1]), to(e0[0], e0[1]), to(f.q[0], f.q[1]))
    });
    if resp.drag_started() {
        let pp = resp.interact_pointer_pos();
        if let (Some((hs, he, hq)), Some(p)) = (fh, pp) {
            ed.fdrag = if hq.distance(p) < 24.0 { 3 } else if hs.distance(p) < 24.0 { 1 } else if he.distance(p) < 24.0 { 2 } else { 0 };
        }
    }
    if ed.fdrag != 0 {
        if let (Some(f), Some(p)) = (ed.fillet.as_mut(), resp.interact_pointer_pos()) {
            let (u, v) = from(p);
            let pc = d.pts[f.p];
            match ed.fdrag {
                3 => f.q = [u, v],
                k => {
                    // projeta o dedo sobre a linha do canto até a outra ponta
                    let other = d.pts[if k == 1 { f.a } else { f.b }];
                    let dir = sub(other, pc);
                    let l = len(dir).max(1e-12);
                    let t = ((u - pc[0]) * dir[0] + (v - pc[1]) * dir[1]) / l;
                    let t = t.clamp(0.0, 0.95 * l);
                    if k == 1 {
                        f.d1 = t;
                    } else {
                        f.d2 = t;
                    }
                }
            }
        }
        if resp.drag_stopped() {
            ed.fdrag = 0;
        }
    } else if resp.drag_started() {
        let hp = resp.interact_pointer_pos().and_then(|p| hit_point(d, p));
        match (ed.tool, hp) {
            (DTool::Move, Some(i)) if i != 0 => {
                ed.push(d);
                ed.drag = Some(i);
                ed.selected = Some(i);
            }
            (DTool::Move, Some(0)) => ed.msg = "O ponto de origem fica fixo.".into(),
            _ => ed.panning = true,
        }
    }
    if resp.dragged() && ed.fdrag == 0 {
        if let (Some(i), Some(p)) = (ed.drag, resp.interact_pointer_pos()) {
            let q = snap(p);
            if d.pts[i] != q {
                d.pts[i] = q;
                changed = true;
            }
        } else if ed.panning {
            let dl = resp.drag_delta();
            cu -= sx * dl.x as f64 / s;
            cv += dl.y as f64 / s;
        }
    }
    if resp.drag_stopped() {
        ed.drag = None;
        ed.panning = false;
    }
    ed.view = Some([cu, cv, s]);
    let to = |u: f64, v: f64| Pos2::new(c.x + (sx * (u - cu) * s) as f32, c.y - ((v - cv) * s) as f32);

    // linhas e pontos
    for e in &d.edges {
        let (a, b) = (d.pts[e[0]], d.pts[e[1]]);
        painter.line_segment([to(a[0], a[1]), to(b[0], b[1])], Stroke::new(2.0, Color32::from_rgb(30, 60, 120)));
    }
    if ed.tool == DTool::Line {
        if let (Some(cur), Some(p)) = (ed.current, resp.hover_pos()) {
            let a = d.pts[cur];
            painter.line_segment([to(a[0], a[1]), p], Stroke::new(1.0, Color32::from_rgba_unmultiplied(30, 60, 120, 110)));
        }
    }
    for (i, q) in d.pts.iter().enumerate() {
        let pp = to(q[0], q[1]);
        let active = ed.current == Some(i) && ed.tool == DTool::Line || ed.selected == Some(i);
        let (r, col) = if i == 0 { (8.0, Color32::from_rgb(210, 50, 40)) } else if active { (8.0, Color32::from_rgb(255, 140, 0)) } else { (6.0, Color32::from_rgb(30, 90, 200)) };
        painter.circle_filled(pp, r, col);
        painter.circle_stroke(pp, r, Stroke::new(1.5, Color32::WHITE));
    }
    if let Some(f) = ed.fillet.as_ref().filter(|f| f.p < d.pts.len() && f.a < d.pts.len() && f.b < d.pts.len()) {
        let curve: Vec<Pos2> = f.curve(d).iter().map(|q| to(q[0], q[1])).collect();
        painter.add(egui::Shape::line(curve.clone(), Stroke::new(2.5, Color32::from_rgb(255, 140, 0))));
        for q in &curve {
            painter.circle_filled(*q, 3.5, Color32::from_rgb(255, 140, 0));
        }
        let (s0, e0) = f.ends(d);
        let pc = d.pts[f.p];
        let (hs, he, hq) = (to(s0[0], s0[1]), to(e0[0], e0[1]), to(f.q[0], f.q[1]));
        let dim = Stroke::new(1.0, Color32::from_rgba_unmultiplied(90, 100, 120, 140));
        painter.line_segment([hs, hq], dim);
        painter.line_segment([he, hq], dim);
        // setas nas pontas (deslizam ao longo das linhas) e alça do meio (forma da curva)
        for (h, other) in [(hs, d.pts[f.a]), (he, d.pts[f.b])] {
            let dir = (to(other[0], other[1]) - to(pc[0], pc[1])).normalized();
            let n = Vec2::new(-dir.y, dir.x);
            painter.circle_filled(h, 15.0, Color32::from_rgba_unmultiplied(255, 255, 255, 220));
            painter.circle_stroke(h, 15.0, Stroke::new(1.5, Color32::from_rgb(210, 60, 50)));
            for sg in [-1.0f32, 1.0] {
                let tip = h + dir * 11.0 * sg;
                let bb = tip - dir * 6.0 * sg;
                painter.add(egui::Shape::convex_polygon(vec![tip, bb + n * 4.5, bb - n * 4.5], Color32::from_rgb(210, 60, 50), Stroke::NONE));
            }
        }
        painter.circle_filled(hq, 12.0, Color32::from_rgba_unmultiplied(255, 255, 255, 220));
        painter.circle_stroke(hq, 12.0, Stroke::new(2.0, Color32::from_rgb(40, 150, 70)));
        painter.circle_filled(hq, 4.0, Color32::from_rgb(40, 150, 70));
    }
    let hint = match ed.tool {
        DTool::Line => "Linha: toque num ponto e depois na tela para criar pontos ligados; toque num ponto existente para fechar o contorno",
        DTool::Point => "Ponto: toque na tela para criar um ponto solto",
        DTool::Move => "Mover: arraste os pontos; arraste o fundo para mover a vista",
        DTool::Split => "Dividir: toque numa linha para pôr um ponto no meio",
        DTool::Erase => "Apagar: toque num ponto ou numa linha",
        DTool::Bucket => "Balde: toque dentro de um contorno fechado para encher com o material",
        DTool::Fillet => "Arredondar: toque num canto; arraste as setas (início/fim) e o círculo verde (forma) e toque em Aplicar",
    };
    let g = painter.layout(hint.to_string(), FontId::proportional(12.0), Color32::from_rgb(80, 90, 110), rect.width() - 16.0);
    painter.galley(rect.left_top() + Vec2::new(8.0, 6.0), g, Color32::from_rgb(80, 90, 110));
    if !ed.msg.is_empty() {
        painter.text(rect.left_bottom() + Vec2::new(8.0, -8.0), Align2::LEFT_BOTTOM, &ed.msg, FontId::proportional(13.0), Color32::from_rgb(190, 110, 0));
    }
    if let Some(i) = ed.selected.filter(|&i| i < d.pts.len()) {
        let q = d.pts[i];
        let txt = if mode == Mode::Projectile { format!("ponto {}: a = {:.1} mm · r = {:.1} mm", i, q[0], q[1]) } else { format!("ponto {}: x = {:.1} mm · z = {:.1} mm", i, q[0], q[1]) };
        painter.text(rect.right_bottom() + Vec2::new(-8.0, -8.0), Align2::RIGHT_BOTTOM, txt, FontId::proportional(12.0), Color32::from_rgb(40, 60, 100));
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(d: &mut Drawing, x0: f64, y0: f64, w: f64) -> Vec<usize> {
        let a = d.add_point([x0, y0]);
        let b = d.add_point([x0 + w, y0]);
        let c = d.add_point([x0 + w, y0 + w]);
        let e = d.add_point([x0, y0 + w]);
        d.add_edge(a, b);
        d.add_edge(b, c);
        d.add_edge(c, e);
        d.add_edge(e, a);
        vec![a, b, c, e]
    }

    #[test]
    fn triangulation_covers_concave() {
        // "L" côncavo: área dos triângulos = área do polígono
        let p = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 1.0], [1.0, 1.0], [1.0, 3.0], [0.0, 3.0]];
        let t = triangulate(&p);
        let a: f64 = t.iter().map(|t| poly_area(&[p[t[0]], p[t[1]], p[t[2]]]).abs()).sum();
        assert!((a - poly_area(&p).abs()).abs() < 1e-9, "área {a}");
    }

    #[test]
    fn faces_and_bucket() {
        let mut d = Drawing::new("t", 10.0);
        square(&mut d, 0.0, 0.0, 10.0);
        square(&mut d, 3.0, 3.0, 4.0);
        let lead = Paint::Mat(pensim_core::LIBRARY[6].clone());
        let cu = Paint::Mat(pensim_core::LIBRARY[5].clone());
        assert!(d.bucket(1.0, 1.0, cu.clone()));
        assert!(d.bucket(5.0, 5.0, lead.clone()));
        assert!(!d.bucket(20.0, 20.0, lead.clone()));
        let f = d.sorted_fills();
        assert_eq!(f.len(), 2);
        assert_eq!(d.paint_at(&f, 1.0, 1.0), Some(cu.clone()));
        assert_eq!(d.paint_at(&f, 5.0, 5.0), Some(lead.clone()));
        // dividir uma linha mantém o preenchimento
        let k = d.edges.iter().position(|e| e == &[1, 2]).unwrap();
        d.split_edge(k);
        let f = d.sorted_fills();
        assert_eq!(d.paint_at(&f, 1.0, 1.0), Some(cu));
        // projétil e objeto
        let p = d.to_projectile(1.0).unwrap();
        assert!(p.length() > 0.009 && p.max_radius() > 0.009);
        assert_eq!(p.part_at(0.005, 0.005, 0.0), Some(1));
        // arredondar o canto (10,10) do quadrado de fora mantém o preenchimento
        let c = d.pts.iter().position(|q| *q == [10.0, 10.0]).unwrap();
        let f = d.start_fillet(c).unwrap();
        d.apply_fillet(&f);
        assert!(!d.pts.iter().any(|q| *q == [10.0, 10.0]));
        let fl = d.sorted_fills();
        assert_eq!(d.paint_at(&fl, 1.0, 1.0).map(|p| p.name().to_string()), Some(pensim_core::LIBRARY[5].name.to_string()));
        assert_eq!(d.paint_at(&fl, 9.9, 9.9), None);
        let o = d.to_object(1.0);
        assert_eq!(o.part_at(0.001, 0.001), Some(0));
        assert_eq!(o.part_at(0.005, 0.005), Some(1));
    }
}
