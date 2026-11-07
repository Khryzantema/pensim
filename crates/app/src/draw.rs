//! Editor de desenho livre: pontos, linhas e contornos fechados preenchidos com o balde.
//!
//! Usado para criar projéteis (perfil no plano a–r, revolvido no eixo) e objetos do alvo
//! (vista lateral x–z, extrudada na largura). Coordenadas em milímetros reais.
//! Regra das peças: o contorno preenchido mais interno (menor área) que contém o ponto vale.
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
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

/// Imagem de referência colocada atrás do desenho (posição, tamanho e giro ajustáveis).
#[derive(Clone, Debug, PartialEq)]
pub struct BgSpec {
    pub path: String,
    /// centro no plano do desenho [mm]
    pub center: [f64; 2],
    /// largura [mm]; a altura vem da proporção da imagem
    pub width_mm: f64,
    pub rot_deg: f64,
    pub alpha: f32,
    pub flip: bool,
    /// altura ÷ largura da imagem
    pub aspect: f64,
}

#[derive(Clone, Debug)]
pub struct Drawing {
    pub name: String,
    pub bg: Option<BgSpec>,
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
    Connect,
    /// ajustar a imagem de fundo
    Image,
}

/// O que está sendo arrastado.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Grab {
    None,
    Pan,
    Point(usize),
    PointH(usize),
    PointV(usize),
    FilletStart,
    FilletEnd,
    FilletMid,
    FilletMidH,
    FilletMidV,
    MultiH,
    MultiV,
    ImgMove,
    ImgHandle,
    /// retângulo de seleção (canto inicial na tela)
    BoxSel(Pos2),
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
    /// textura da imagem de fundo carregada (caminho, textura)
    pub bg_tex: Option<(String, egui::TextureHandle)>,
    /// pedido de carregar uma imagem (tratado no próximo quadro do editor)
    pub pending_load: Option<String>,
    pub img_err: String,
    grab: Grab,
    /// posição do dedo e valor (u, v) no início do arraste
    g0: (Pos2, [f64; 2]),
    /// seleção de vários pontos (Mover) e suas posições no início do arraste
    pub multi: Vec<usize>,
    m0: Vec<[f64; 2]>,
}

impl DrawEditor {
    pub fn new(paint: Paint, snap_mm: f64) -> Self {
        DrawEditor { tool: DTool::Line, current: Some(0), selected: None, paint, undo: vec![], snap_mm, view: None, fit: true, msg: String::new(), fillet: None, bg_tex: None, pending_load: None, img_err: String::new(), grab: Grab::None, g0: (Pos2::ZERO, [0.0; 2]), multi: vec![], m0: vec![] }
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
            self.multi.clear();
            true
        } else {
            false
        }
    }
}

impl Drawing {
    pub fn new(name: &str, width_mm: f64) -> Self {
        Drawing { name: name.into(), bg: None, pts: vec![[0.0, 0.0]], edges: vec![], fills: vec![], width_mm }
    }

    /// Texto simples e estável para guardar o desenho (a imagem de fundo não entra).
    pub fn to_text(&self) -> String {
        let mut o = String::from("PENSIM-DESENHO 1\n");
        o += &format!("N {}\nW {}\n", self.name.replace('\n', " "), self.width_mm);
        for p in &self.pts {
            o += &format!("P {} {}\n", p[0], p[1]);
        }
        for e in &self.edges {
            o += &format!("E {} {}\n", e[0], e[1]);
        }
        for f in &self.fills {
            let c: Vec<String> = f.cycle.iter().map(|i| i.to_string()).collect();
            o += &format!("F {}|{}\n", f.paint.name(), c.join(" "));
        }
        o
    }

    pub fn from_text(t: &str) -> Option<Drawing> {
        let mut l = t.lines();
        if !l.next()?.starts_with("PENSIM-DESENHO") {
            return None;
        }
        let mut d = Drawing { name: String::new(), bg: None, pts: vec![], edges: vec![], fills: vec![], width_mm: 0.0 };
        for ln in l {
            let (k, r) = ln.split_at(ln.find(' ').unwrap_or(ln.len()));
            let r = r.trim_start();
            match k {
                "N" => d.name = r.into(),
                "W" => d.width_mm = r.parse().ok()?,
                "P" => {
                    let v: Vec<f64> = r.split(' ').filter_map(|x| x.parse().ok()).collect();
                    d.pts.push([*v.first()?, *v.get(1)?]);
                }
                "E" => {
                    let v: Vec<usize> = r.split(' ').filter_map(|x| x.parse().ok()).collect();
                    d.edges.push([*v.first()?, *v.get(1)?]);
                }
                "F" => {
                    let (nm, c) = r.split_once('|')?;
                    let paint = match pensim_core::LIBRARY.iter().find(|m| m.name == nm) {
                        Some(m) => Paint::Mat(m.clone()),
                        None => Paint::Void,
                    };
                    let cycle: Vec<usize> = c.split(' ').filter_map(|x| x.parse().ok()).collect();
                    d.fills.push(FillRec { cycle, paint });
                }
                _ => {}
            }
        }
        let n = d.pts.len();
        if n == 0 || d.edges.iter().any(|e| e[0] >= n || e[1] >= n) || d.fills.iter().any(|f| f.cycle.iter().any(|&i| i >= n)) {
            return None;
        }
        Some(d)
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

fn centroid(d: &Drawing, ids: &[usize]) -> [f64; 2] {
    let n = ids.len().max(1) as f64;
    let (mut x, mut y) = (0.0, 0.0);
    for &i in ids {
        x += d.pts[i][0];
        y += d.pts[i][1];
    }
    [x / n, y / n]
}

/// Lê uma imagem (PNG/JPEG), reduz se for enorme e cria a textura. Devolve (textura, proporção h/w).
pub fn load_image(ctx: &egui::Context, path: &str) -> Result<(egui::TextureHandle, f64), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Não consegui ler o arquivo: {e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("Imagem inválida (use PNG ou JPEG): {e}"))?;
    let (w, h) = (img.width().max(1), img.height().max(1));
    let img = if w.max(h) > 2048 {
        let f = 2048.0 / w.max(h) as f32;
        img.resize(((w as f32 * f) as u32).max(1), ((h as f32 * f) as u32).max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (iw, ih) = (rgba.width() as usize, rgba.height() as usize);
    let ci = egui::ColorImage::from_rgba_unmultiplied([iw, ih], rgba.as_raw());
    let tex = ctx.load_texture(format!("bg:{path}"), ci, egui::TextureOptions::LINEAR);
    Ok((tex, ih as f64 / iw as f64))
}

impl BgSpec {
    /// cantos (no plano do desenho) na ordem: sup-esq, sup-dir, inf-dir, inf-esq (da imagem)
    /// `sx` = −1 quando o eixo horizontal do desenho está espelhado na tela (editor do projétil):
    /// a imagem e o giro seguem a ORIENTAÇÃO DA TELA, como na foto original.
    pub fn corners(&self, sx: f64) -> [[f64; 2]; 4] {
        let (hw, hh) = (self.width_mm * 0.5, self.width_mm * self.aspect * 0.5);
        let (sn, cs) = self.rot_deg.to_radians().sin_cos();
        let pts = [[-hw, hh], [hw, hh], [hw, -hh], [-hw, -hh]];
        pts.map(|q| [self.center[0] + sx * (cs * q[0] - sn * q[1]), self.center[1] + sn * q[0] + cs * q[1]])
    }
    /// alça de escala/giro (meio do lado direito)
    pub fn handle(&self, sx: f64) -> [f64; 2] {
        let (sn, cs) = self.rot_deg.to_radians().sin_cos();
        let hw = self.width_mm * 0.5;
        [self.center[0] + sx * cs * hw, self.center[1] + sn * hw]
    }
}

/// Setas de arraste horizontal e vertical, afastadas do ponto para o dedo não cobri-lo.
fn gizmo(p: Pos2, rect: Rect) -> (Pos2, Pos2) {
    let hx = if p.x + 84.0 > rect.right() { -58.0 } else { 58.0 };
    let vy = if p.y - 84.0 < rect.top() { 58.0 } else { -58.0 };
    (Pos2::new(p.x + hx, p.y), Pos2::new(p.x, p.y + vy))
}

fn arrow_glyph(painter: &egui::Painter, c: Pos2, d: Vec2, col: Color32, active: bool) {
    let n = Vec2::new(-d.y, d.x);
    painter.circle_filled(c, 17.0, Color32::from_rgba_unmultiplied(255, 255, 255, 225));
    painter.circle_stroke(c, 17.0, Stroke::new(if active { 3.0 } else { 1.5 }, col));
    painter.line_segment([c - d * 11.0, c + d * 11.0], Stroke::new(2.5, col));
    for sg in [-1.0f32, 1.0] {
        let tip = c + d * 13.0 * sg;
        let b = tip - d * 7.0 * sg;
        painter.add(egui::Shape::convex_polygon(vec![tip, b + n * 5.0, b - n * 5.0], col, Stroke::NONE));
    }
}

fn draw_gizmo(painter: &egui::Painter, p: Pos2, h: Pos2, v: Pos2, ah: bool, av: bool) {
    let dim = Stroke::new(1.0, Color32::from_rgba_unmultiplied(90, 100, 120, 130));
    painter.line_segment([p, h], dim);
    painter.line_segment([p, v], dim);
    arrow_glyph(painter, h, Vec2::new(1.0, 0.0), Color32::from_rgb(210, 60, 50), ah);
    arrow_glyph(painter, v, Vec2::new(0.0, 1.0), Color32::from_rgb(40, 150, 70), av);
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
        // espaço livre: dica no topo e faixa de ferramentas na base
        let s = ((rect.width() as f64 - 60.0) / (b[1] - b[0])).min((rect.height() as f64 - 140.0) / (b[3] - b[2])).max(0.05);
        ed.view = Some([0.5 * (b[0] + b[1]), 0.5 * (b[2] + b[3]) - 10.0 / s, s]);
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

    // imagem de fundo: carrega sob demanda (pedido novo, ou o desenho aponta para outra imagem)
    if let Some(path) = ed.pending_load.take() {
        match load_image(ui.ctx(), &path) {
            Ok((tex, aspect)) => {
                let w = if d.bg.is_none() { ((rect.width() as f64 / s) * 0.6).max(1.0) } else { d.bg.as_ref().unwrap().width_mm };
                let center = d.bg.as_ref().map_or([cu, cv], |b| b.center);
                let (rot, alpha, flip) = d.bg.as_ref().map_or((0.0, 0.55, false), |b| (b.rot_deg, b.alpha, b.flip));
                if d.bg.is_none() {
                    ed.push(d);
                }
                d.bg = Some(BgSpec { path: path.clone(), center, width_mm: w, rot_deg: rot, alpha, flip, aspect });
                ed.bg_tex = Some((path, tex));
                ed.img_err.clear();
                ed.tool = DTool::Image;
            }
            Err(e) => ed.img_err = e,
        }
    }
    if let Some(b) = &d.bg {
        if ed.bg_tex.as_ref().map(|t| t.0.as_str()) != Some(b.path.as_str()) {
            ed.pending_load = Some(b.path.clone());
        }
    }
    if let (Some(b), Some((_, tex))) = (&d.bg, &ed.bg_tex) {
        let cn = b.corners(sx);
        let uvs = if b.flip { [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]] } else { [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] };
        let tint = Color32::from_white_alpha((b.alpha.clamp(0.05, 1.0) * 255.0) as u8);
        let mut mesh = egui::Mesh::with_texture(tex.id());
        for (q, uv) in cn.iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex { pos: Pos2::new(c.x + (sx * (q[0] - cu) * s) as f32, c.y - ((q[1] - cv) * s) as f32), uv: Pos2::new(uv[0], uv[1]), color: tint });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(egui::Shape::mesh(mesh));
    }

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
            // toque nas setas do ponto selecionado não troca a seleção
            let gizmo_hit = ed.selected.filter(|&i| i < d.pts.len()).map_or(false, |i| {
                let (h, v) = gizmo(to(d.pts[i][0], d.pts[i][1]), rect);
                h.distance(p) < 24.0 || v.distance(p) < 24.0
            }) || (ed.multi.len() >= 2 && {
                let c = centroid(d, &ed.multi);
                let (h, v) = gizmo(to(c[0], c[1]), rect);
                h.distance(p) < 24.0 || v.distance(p) < 24.0
            });
            match ed.tool {
                DTool::Line => match (hp, ed.current) {
                    (Some(i), None) => ed.current = Some(i),
                    (Some(i), Some(cur)) if cur == i => {}
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
                DTool::Connect => match (hp, ed.current) {
                    (Some(i), None) => ed.current = Some(i),
                    (Some(i), Some(cur)) if cur == i => ed.current = None,
                    (Some(i), Some(cur)) => {
                        push(ed, d);
                        d.add_edge(cur, i);
                        ed.current = None;
                        changed = true;
                    }
                    (None, _) => ed.current = None,
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
                DTool::Move => {
                    let shift = ui.input(|i| i.modifiers.shift);
                    if let (true, Some(i)) = (shift, hp) {
                        if let Some(k) = ed.multi.iter().position(|&x| x == i) {
                            ed.multi.remove(k);
                        } else if i != 0 {
                            ed.multi.push(i);
                        }
                    } else if hp.is_some() || !gizmo_hit {
                        ed.selected = hp;
                        ed.multi.clear();
                    }
                }
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
                DTool::Image => {}
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
    // segurar sobre um ponto (Mover): entra/sai da seleção múltipla
    if ed.tool == DTool::Move && resp.long_touched() {
        if let Some(p) = resp.interact_pointer_pos() {
            if let Some(i) = hit_point(d, p).filter(|&i| i != 0) {
                if ed.multi.is_empty() {
                    if let Some(s0) = ed.selected.filter(|&s0| s0 != 0 && s0 != i) {
                        ed.multi.push(s0);
                    }
                }
                if let Some(k) = ed.multi.iter().position(|&x| x == i) {
                    ed.multi.remove(k);
                } else {
                    ed.multi.push(i);
                }
                ed.selected = None;
            }
        }
    }
    // alças do arredondamento e setas do ponto selecionado (posições na tela)
    let fh = ed.fillet.as_ref().filter(|f| f.p < d.pts.len() && f.a < d.pts.len() && f.b < d.pts.len()).map(|f| {
        let (s0, e0) = f.ends(d);
        let hq = to(f.q[0], f.q[1]);
        let (gh, gv) = gizmo(hq, rect);
        (to(s0[0], s0[1]), to(e0[0], e0[1]), hq, gh, gv)
    });
    ed.multi.retain(|&i| i > 0 && i < d.pts.len());
    let multi_on = ed.multi.len() >= 2 && ed.tool == DTool::Move;
    let mg = if multi_on {
        let c = centroid(d, &ed.multi);
        Some(gizmo(to(c[0], c[1]), rect))
    } else {
        None
    };
    let sel = ed.selected.filter(|&i| i > 0 && i < d.pts.len() && matches!(ed.tool, DTool::Move | DTool::Point) && !multi_on);
    let pg = sel.map(|i| gizmo(to(d.pts[i][0], d.pts[i][1]), rect));
    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            let near = |q: Pos2| q.distance(p) < 24.0;
            ed.grab = Grab::Pan;
            // ferramenta Imagem: arrastar o fundo move a imagem; a alça verde escala e gira
            if ed.tool == DTool::Image {
                if let Some(b) = &d.bg {
                    let h = b.handle(sx);
                    let hp = Pos2::new(c.x + (sx * (h[0] - cu) * s) as f32, c.y - ((h[1] - cv) * s) as f32);
                    ed.grab = if hp.distance(p) < 28.0 { Grab::ImgHandle } else { Grab::ImgMove };
                    ed.push(d);
                }
            }
            if let Some((hs, he, _hq, gh, gv)) = fh {
                ed.grab = if near(gh) { Grab::FilletMidH } else if near(gv) { Grab::FilletMidV } else if near(hs) { Grab::FilletStart } else if near(he) { Grab::FilletEnd } else { Grab::Pan };
            }
            if ed.grab == Grab::Pan {
                if let Some((gh, gv)) = mg {
                    if near(gh) {
                        ed.grab = Grab::MultiH;
                    } else if near(gv) {
                        ed.grab = Grab::MultiV;
                    }
                }
            }
            if ed.grab == Grab::Pan {
                if let (Some(i), Some((gh, gv))) = (sel, pg) {
                    if near(gh) {
                        ed.grab = Grab::PointH(i);
                    } else if near(gv) {
                        ed.grab = Grab::PointV(i);
                    }
                }
            }
            // pontos só se movem pelas setas (arrastar o ponto direto move a vista), para evitar acidentes
            if ed.grab == Grab::Pan && ed.tool == DTool::Move && hit_point(d, p) == Some(0) {
                ed.msg = "O ponto de origem fica fixo.".into();
            }
            // segurar antes de arrastar (Mover): retângulo de seleção em vez de mover a vista
            if ed.grab == Grab::Pan && ed.tool == DTool::Move {
                let held = ui.input(|i| i.pointer.press_start_time().map_or(0.0, |t0| i.time - t0));
                let shift = ui.input(|i| i.modifiers.shift);
                if held > 0.35 || shift {
                    let start = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
                    ed.grab = Grab::BoxSel(start);
                }
            }
            if matches!(ed.grab, Grab::MultiH | Grab::MultiV) {
                ed.push(d);
                ed.m0 = ed.multi.iter().map(|&i| d.pts[i]).collect();
            }
            let v0 = match ed.grab {
                Grab::Point(i) | Grab::PointH(i) | Grab::PointV(i) => d.pts[i],
                Grab::FilletMid | Grab::FilletMidH | Grab::FilletMidV => ed.fillet.as_ref().map_or([0.0; 2], |f| f.q),
                _ => [0.0; 2],
            };
            if matches!(ed.grab, Grab::Point(_) | Grab::PointH(_) | Grab::PointV(_)) {
                ed.push(d);
            }
            ed.g0 = (p, v0);
        }
    }
    if resp.dragged() {
        if let Some(p) = resp.interact_pointer_pos() {
            let (p0, v0) = ed.g0;
            let g = snap_mm.max(0.01);
            let sn = |x: f64| (x / g).round() * g;
            let du = sx * (p.x - p0.x) as f64 / s;
            let dv = -((p.y - p0.y) as f64) / s;
            let vmin = if mode == Mode::Projectile { 0.0 } else { f64::MIN };
            match ed.grab {
                Grab::Pan => {
                    let dl = resp.drag_delta();
                    cu -= sx * dl.x as f64 / s;
                    cv += dl.y as f64 / s;
                }
                Grab::Point(i) => {
                    let q = snap(p);
                    if d.pts[i] != q {
                        d.pts[i] = q;
                        changed = true;
                    }
                }
                Grab::PointH(i) => {
                    let q = [sn(v0[0] + du), d.pts[i][1]];
                    if d.pts[i] != q {
                        d.pts[i] = q;
                        changed = true;
                    }
                }
                Grab::PointV(i) => {
                    let q = [d.pts[i][0], sn(v0[1] + dv).max(vmin)];
                    if d.pts[i] != q {
                        d.pts[i] = q;
                        changed = true;
                    }
                }
                Grab::FilletMid | Grab::FilletMidH | Grab::FilletMidV => {
                    if let Some(f) = ed.fillet.as_mut() {
                        f.q = match ed.grab {
                            Grab::FilletMidH => [v0[0] + du, v0[1]],
                            Grab::FilletMidV => [v0[0], v0[1] + dv],
                            _ => {
                                let (u, v) = from(p);
                                [u, v]
                            }
                        };
                    }
                }
                Grab::FilletStart | Grab::FilletEnd => {
                    if let Some(f) = ed.fillet.as_mut() {
                        // projeta o dedo sobre a linha do canto até a outra ponta
                        let (u, v) = from(p);
                        let pc = d.pts[f.p];
                        let other = d.pts[if ed.grab == Grab::FilletStart { f.a } else { f.b }];
                        let dir = sub(other, pc);
                        let l = len(dir).max(1e-12);
                        let t = (((u - pc[0]) * dir[0] + (v - pc[1]) * dir[1]) / l).clamp(0.0, 0.95 * l);
                        if ed.grab == Grab::FilletStart {
                            f.d1 = t;
                        } else {
                            f.d2 = t;
                        }
                    }
                }
                Grab::ImgMove => {
                    let dl = resp.drag_delta();
                    if let Some(b) = d.bg.as_mut() {
                        b.center[0] += sx * dl.x as f64 / s;
                        b.center[1] -= dl.y as f64 / s;
                    }
                }
                Grab::ImgHandle => {
                    let (u, v) = from(p);
                    if let Some(b) = d.bg.as_mut() {
                        let (dx, dy) = (u - b.center[0], v - b.center[1]);
                        b.width_mm = (2.0 * (dx * dx + dy * dy).sqrt()).max(1.0);
                        b.rot_deg = dy.atan2(sx * dx).to_degrees();
                    }
                }
                Grab::MultiH | Grab::MultiV => {
                    let horiz = ed.grab == Grab::MultiH;
                    for (k, &i) in ed.multi.iter().enumerate() {
                        let q0 = ed.m0[k];
                        let q = if horiz { [sn(q0[0] + du), q0[1]] } else { [q0[0], sn(q0[1] + dv).max(vmin)] };
                        if d.pts[i] != q {
                            d.pts[i] = q;
                            changed = true;
                        }
                    }
                }
                Grab::BoxSel(_) | Grab::None => {}
            }
        }
    }
    if resp.drag_stopped() {
        if let (Grab::BoxSel(a), Some(b)) = (ed.grab, resp.interact_pointer_pos().or(resp.hover_pos())) {
            let r = Rect::from_two_pos(a, b);
            let add = ui.input(|i| i.modifiers.shift);
            if !add {
                ed.multi.clear();
            }
            for (i, q) in d.pts.iter().enumerate() {
                let sp = Pos2::new(c.x + (sx * (q[0] - cu) * s) as f32, c.y - ((q[1] - cv) * s) as f32);
                if i != 0 && r.contains(sp) && !ed.multi.contains(&i) {
                    ed.multi.push(i);
                }
            }
            ed.selected = if ed.multi.len() == 1 { Some(ed.multi[0]) } else { None };
            if ed.multi.len() == 1 {
                ed.multi.clear();
            }
        }
        ed.grab = Grab::None;
    }
    let boxsel = if let Grab::BoxSel(a) = ed.grab { resp.interact_pointer_pos().map(|b| Rect::from_two_pos(a, b)) } else { None };
    ed.view = Some([cu, cv, s]);
    let to = |u: f64, v: f64| Pos2::new(c.x + (sx * (u - cu) * s) as f32, c.y - ((v - cv) * s) as f32);

    // linhas e pontos
    for e in &d.edges {
        let (a, b) = (d.pts[e[0]], d.pts[e[1]]);
        painter.line_segment([to(a[0], a[1]), to(b[0], b[1])], Stroke::new(2.0, Color32::from_rgb(30, 60, 120)));
    }
    if matches!(ed.tool, DTool::Line | DTool::Connect) {
        if let (Some(cur), Some(p)) = (ed.current, resp.hover_pos()) {
            let a = d.pts[cur];
            painter.line_segment([to(a[0], a[1]), p], Stroke::new(1.0, Color32::from_rgba_unmultiplied(30, 60, 120, 110)));
        }
    }
    for (i, q) in d.pts.iter().enumerate() {
        let pp = to(q[0], q[1]);
        let active = ed.current == Some(i) && matches!(ed.tool, DTool::Line | DTool::Connect) || ed.selected == Some(i);
        let (r, col) = if i == 0 { (8.0, Color32::from_rgb(210, 50, 40)) } else if active { (8.0, Color32::from_rgb(255, 140, 0)) } else { (6.0, Color32::from_rgb(30, 90, 200)) };
        painter.circle_filled(pp, r, col);
        painter.circle_stroke(pp, r, Stroke::new(1.5, Color32::WHITE));
        if ed.current == Some(i) && matches!(ed.tool, DTool::Line | DTool::Connect) {
            painter.circle_stroke(pp, r + 5.0, Stroke::new(2.5, Color32::from_rgb(255, 140, 0)));
        }
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
        let (gh, gv) = gizmo(hq, rect);
        draw_gizmo(&painter, hq, gh, gv, matches!(ed.grab, Grab::FilletMidH), matches!(ed.grab, Grab::FilletMidV));
    }
    if let (DTool::Image, Some(b)) = (ed.tool, &d.bg) {
        let cn: Vec<Pos2> = b.corners(sx).iter().map(|q| Pos2::new(c.x + (sx * (q[0] - cu) * s) as f32, c.y - ((q[1] - cv) * s) as f32)).collect();
        painter.add(egui::Shape::closed_line(cn, Stroke::new(1.5, Color32::from_rgb(0, 112, 192))));
        let h = b.handle(sx);
        let hp = Pos2::new(c.x + (sx * (h[0] - cu) * s) as f32, c.y - ((h[1] - cv) * s) as f32);
        let cp = Pos2::new(c.x + (sx * (b.center[0] - cu) * s) as f32, c.y - ((b.center[1] - cv) * s) as f32);
        painter.line_segment([cp, hp], Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 112, 192, 140)));
        painter.circle_filled(cp, 5.0, Color32::from_rgb(0, 112, 192));
        painter.circle_filled(hp, 13.0, Color32::from_rgba_unmultiplied(255, 255, 255, 235));
        painter.circle_stroke(hp, 13.0, Stroke::new(2.0, Color32::from_rgb(40, 150, 70)));
        painter.text(hp, Align2::CENTER_CENTER, "⟲", FontId::proportional(14.0), Color32::from_rgb(40, 150, 70));
    }
    if let Some(r) = boxsel {
        painter.rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(0, 112, 192, 30));
        painter.rect_stroke(r, 2.0, Stroke::new(1.5, Color32::from_rgb(0, 112, 192)), egui::StrokeKind::Inside);
    }
    for &i in &ed.multi {
        if i < d.pts.len() {
            let pp = to(d.pts[i][0], d.pts[i][1]);
            painter.circle_stroke(pp, 11.0, Stroke::new(2.5, Color32::from_rgb(255, 140, 0)));
        }
    }
    if ed.multi.len() >= 2 && ed.tool == DTool::Move {
        let cc = centroid(d, &ed.multi);
        let pp = to(cc[0], cc[1]);
        painter.circle_filled(pp, 5.0, Color32::from_rgb(255, 140, 0));
        let (gh, gv) = gizmo(pp, rect);
        draw_gizmo(&painter, pp, gh, gv, ed.grab == Grab::MultiH, ed.grab == Grab::MultiV);
    } else if let Some(i) = ed.selected.filter(|&i| i > 0 && i < d.pts.len() && matches!(ed.tool, DTool::Move | DTool::Point)) {
        let pp = to(d.pts[i][0], d.pts[i][1]);
        let (gh, gv) = gizmo(pp, rect);
        draw_gizmo(&painter, pp, gh, gv, matches!(ed.grab, Grab::PointH(_)), matches!(ed.grab, Grab::PointV(_)));
    }
    let hint = match ed.tool {
        DTool::Line => "Desenhar: a linha sai do ponto com anel laranja (no começo, a origem vermelha). Toque na tela para criar o próximo ponto; toque num ponto existente para ligar até ele",
        DTool::Point => "Ponto: toque na tela para criar um ponto solto",
        DTool::Move => "Mover: toque num ponto e arraste as setas (vermelha = horizontal, verde = vertical). Vários pontos: segure e arraste para fazer um retângulo, ou segure sobre cada ponto",
        DTool::Split => "Dividir: toque numa linha para pôr um ponto no meio",
        DTool::Erase => "Apagar: toque num ponto ou numa linha",
        DTool::Bucket => "Balde: toque dentro de um contorno fechado para encher com o material",
        DTool::Fillet => "Arredondar: toque num canto; as setas nas linhas ajustam início e fim, as setas do círculo verde ajustam a forma; depois toque em Aplicar",
        DTool::Connect => "Ligar: toque em dois pontos para fazer uma linha entre eles",
        DTool::Image => "Imagem: arraste o fundo para mover a imagem; arraste a bolinha verde para mudar o tamanho e girar",
    };
    let g = painter.layout(hint.to_string(), FontId::proportional(12.0), Color32::from_rgb(80, 90, 110), rect.width() - 16.0);
    painter.galley(rect.left_top() + Vec2::new(8.0, 6.0), g, Color32::from_rgb(80, 90, 110));
    if !ed.msg.is_empty() {
        painter.text(rect.left_bottom() + Vec2::new(8.0, -58.0), Align2::LEFT_BOTTOM, &ed.msg, FontId::proportional(13.0), Color32::from_rgb(190, 110, 0));
    }
    // faixa de ferramentas na base do desenho (ícone em cima, nome embaixo)
    let ctx = ui.ctx().clone();
    let list: [(DTool, &str, &str); 9] = [
        (DTool::Line, "✎", "Desenhar"),
        (DTool::Connect, "↔", "Ligar"),
        (DTool::Point, "•", "Ponto"),
        (DTool::Move, "✥", "Mover"),
        (DTool::Split, "✂", "Dividir"),
        (DTool::Fillet, "◠", "Arredondar"),
        (DTool::Erase, "⌫", "Apagar"),
        (DTool::Bucket, "▧", "Balde"),
        (DTool::Image, "▣", "Imagem"),
    ];
    let strip = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 50.0), rect.right_bottom());
    painter.rect_filled(strip, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 240));
    painter.line_segment([strip.left_top(), strip.right_top()], Stroke::new(1.0, Color32::from_rgb(215, 220, 228)));
    egui::Area::new(egui::Id::new(("draw_tools", mode == Mode::Projectile))).order(egui::Order::Foreground).fixed_pos(strip.left_top() + Vec2::new(4.0, 3.0)).show(&ctx, |ui| {
        ui.set_max_width(strip.width() - 8.0);
        egui::ScrollArea::horizontal().id_salt(("tools_scroll", mode == Mode::Projectile)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(2.0, 0.0);
                ui.spacing_mut().button_padding = Vec2::new(2.0, 2.0);
                for (t, ic, l) in list {
                    let on = ed.tool == t;
                    let fg = if on { Color32::WHITE } else { Color32::from_gray(45) };
                    let b = egui::Button::new(RichText::new(format!("{ic}\n{l}")).size(10.5).color(fg))
                        .fill(if on { Color32::from_rgb(0, 112, 192) } else { Color32::TRANSPARENT })
                        .corner_radius(8.0)
                        .min_size(Vec2::new(40.0, 42.0));
                    if ui.add(b).clicked() && !on {
                        ed.tool = t;
                        ed.msg.clear();
                        if t != DTool::Fillet {
                            ed.fillet = None;
                        }
                        if t == DTool::Connect {
                            ed.current = None;
                        }
                    }
                }
            });
        });
    });
    // desfazer / enquadrar / terminar linha (canto superior direito, abaixo da troca 2D/3D)
    egui::Area::new(egui::Id::new(("draw_actions", mode == Mode::Projectile))).order(egui::Order::Foreground).pivot(Align2::RIGHT_TOP).fixed_pos(rect.right_top() + Vec2::new(-8.0, 84.0)).show(&ctx, |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);
            if ui.add_enabled(!ed.undo.is_empty(), egui::Button::new("↶ Desfazer").corner_radius(14.0).min_size(Vec2::new(110.0, 28.0))).clicked() {
                changed |= ed.undo(d);
            }
            if ui.add(egui::Button::new("Enquadrar").corner_radius(14.0).min_size(Vec2::new(110.0, 28.0))).clicked() {
                ed.fit = true;
            }
            if ed.tool == DTool::Line && ed.current.is_some() && ui.add(egui::Button::new(RichText::new("✓ Terminar linha").strong()).corner_radius(14.0).min_size(Vec2::new(110.0, 28.0))).clicked() {
                ed.current = None;
            }
        });
    });
    // barra do arredondamento (logo acima da faixa de ferramentas)
    if ed.fillet.is_some() {
        egui::Area::new(egui::Id::new(("fillet_bar", mode == Mode::Projectile))).order(egui::Order::Foreground).fixed_pos(rect.left_bottom() + Vec2::new(6.0, -94.0)).show(&ctx, |ui| {
            let (mut apply, mut cancel) = (false, false);
            if let Some(f) = ed.fillet.as_mut() {
                egui::Frame::NONE.fill(Color32::from_rgba_unmultiplied(255, 255, 255, 245)).stroke(Stroke::new(1.5, Color32::from_rgb(255, 140, 0))).corner_radius(12.0).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button(" − ").clicked() && f.n > 2 {
                            f.n -= 1;
                        }
                        ui.label(RichText::new(format!("{} pontos", f.n)).size(13.0));
                        if ui.button(" + ").clicked() && f.n < 40 {
                            f.n += 1;
                        }
                        if ui.add(egui::Button::new(RichText::new("✓ Aplicar").strong().color(Color32::WHITE)).fill(Color32::from_rgb(0, 112, 192)).corner_radius(14.0).min_size(Vec2::new(90.0, 30.0))).clicked() {
                            apply = true;
                        }
                        if ui.add(egui::Button::new("✕").corner_radius(14.0).min_size(Vec2::new(32.0, 30.0))).on_hover_text("Cancelar").clicked() {
                            cancel = true;
                        }
                    });
                });
            }
            if cancel {
                ed.msg.clear();
                ed.fillet = None;
            }
            if apply {
                if let Some(f) = ed.fillet.take() {
                    ed.push(d);
                    d.apply_fillet(&f);
                    changed = true;
                }
            }
        });
    }
    if let Some(i) = ed.selected.filter(|&i| i < d.pts.len()) {
        let q = d.pts[i];
        let txt = if mode == Mode::Projectile { format!("ponto {}: a = {:.1} mm · r = {:.1} mm", i, q[0], q[1]) } else { format!("ponto {}: x = {:.1} mm · z = {:.1} mm", i, q[0], q[1]) };
        painter.text(rect.right_bottom() + Vec2::new(-8.0, -58.0), Align2::RIGHT_BOTTOM, txt, FontId::proportional(12.0), Color32::from_rgb(40, 60, 100));
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
