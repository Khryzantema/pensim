//! Editor de esboço 2D do perfil do projétil (revolvido em torno do eixo).
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Stroke};
use pensim_core::Layer;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Move,
    Add,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

pub struct SketchEditor {
    pub tool: Tool,
    pub snap_mm: f64,
    /// ponto selecionado (mostra as setas de movimento)
    pub selected: Option<usize>,
    drag: Option<usize>,
    /// vértice selecionado no editor do alvo: (camada, é o vértice traseiro?)
    pub tsel: Option<(usize, bool)>,
    tdrag: Option<(usize, bool)>,
    /// arraste de seta: eixo, valor inicial (mm) e posição inicial do ponteiro (px)
    gizmo: Option<(Axis, f64, f32)>,
}

impl Default for SketchEditor {
    fn default() -> Self {
        SketchEditor { tool: Tool::Move, snap_mm: 0.5, selected: None, drag: None, tsel: None, tdrag: None, gizmo: None }
    }
}

/// Posições (centro da alça) das setas horizontal e vertical de um ponto, afastadas dele.
fn gizmo_handles(p: Pos2, rect: egui::Rect) -> (Pos2, Pos2) {
    let hx = if p.x + 78.0 > rect.right() { -52.0 } else { 52.0 };
    let vy = if p.y - 78.0 < rect.top() { 52.0 } else { -52.0 };
    (Pos2::new(p.x + hx, p.y), Pos2::new(p.x, p.y + vy))
}

fn arrow(painter: &egui::Painter, c: Pos2, horizontal: bool, col: Color32, active: bool) {
    let (d, n) = if horizontal { (egui::Vec2::new(1.0, 0.0), egui::Vec2::new(0.0, 1.0)) } else { (egui::Vec2::new(0.0, 1.0), egui::Vec2::new(1.0, 0.0)) };
    let half = 20.0;
    painter.circle_filled(c, 19.0, Color32::from_rgba_unmultiplied(255, 255, 255, 215));
    painter.circle_stroke(c, 19.0, Stroke::new(if active { 3.0 } else { 1.5 }, col));
    painter.line_segment([c - d * half * 0.7, c + d * half * 0.7], Stroke::new(2.5, col));
    for sgn in [-1.0f32, 1.0] {
        let tip = c + d * (half * 0.85) * sgn;
        let b = tip - d * 8.0 * sgn;
        painter.add(egui::Shape::convex_polygon(vec![tip, b + n * 5.0, b - n * 5.0], col, Stroke::NONE));
    }
}

/// Retorna `true` se o perfil mudou. `profile` em metros: (a = distância da ponta, r = raio).
pub fn show(ui: &mut egui::Ui, ed: &mut SketchEditor, profile: &mut Vec<[f64; 2]>, fill: Color32, core: Option<(&[[f64; 2]], Color32)>) -> bool {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, Color32::from_rgb(250, 251, 253));
    let len = profile.last().map_or(0.03, |p| p[0]).max(0.005) * 1e3;
    let rmax = profile.iter().map(|p| p[1]).fold(0.004, f64::max) * 1e3;
    let scale = ((rect.width() - 80.0) as f64 / (len + 8.0)).min((rect.height() * 0.5 - 40.0) as f64 / (rmax + 3.0)) as f32;
    let origin = Pos2::new(rect.right() - 40.0, rect.center().y); // ponta à direita (a=0)
    let to = |a_mm: f64, r_mm: f64| Pos2::new(origin.x - a_mm as f32 * scale, origin.y - r_mm as f32 * scale);
    let from = |p: Pos2| ((origin.x - p.x) as f64 / scale as f64, (origin.y - p.y) as f64 / scale as f64);

    // grade
    let minor = Stroke::new(1.0, Color32::from_rgb(232, 236, 242));
    let major = Stroke::new(1.0, Color32::from_rgb(205, 212, 224));
    for k in -(rmax as i32 + 6)..=(rmax as i32 + 6) {
        let y = origin.y - k as f32 * scale;
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], if k % 5 == 0 { major } else { minor });
    }
    for k in -8..=(len as i32 + 8) {
        let x = origin.x - k as f32 * scale;
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], if k % 5 == 0 { major } else { minor });
    }
    // eixo de revolução
    painter.line_segment([Pos2::new(rect.left(), origin.y), Pos2::new(rect.right(), origin.y)], Stroke::new(1.5, Color32::from_rgb(200, 60, 60)));
    painter.text(Pos2::new(rect.left() + 6.0, origin.y - 4.0), Align2::LEFT_BOTTOM, "eixo de revolução", FontId::proportional(11.0), Color32::from_rgb(200, 60, 60));

    // sólido: faixas entre o perfil e seu espelho
    if profile.len() >= 2 {
        let c = Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 170);
        let mut mesh = egui::Mesh::default();
        for w in profile.windows(2) {
            let q = [to(w[0][0] * 1e3, w[0][1] * 1e3), to(w[1][0] * 1e3, w[1][1] * 1e3), to(w[1][0] * 1e3, -w[1][1] * 1e3), to(w[0][0] * 1e3, -w[0][1] * 1e3)];
            let base = mesh.vertices.len() as u32;
            for p in q {
                mesh.colored_vertex(p, c);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        painter.add(egui::Shape::mesh(mesh));
        let mut poly: Vec<Pos2> = profile.iter().map(|p| to(p[0] * 1e3, p[1] * 1e3)).collect();
        let mut back: Vec<Pos2> = profile.iter().rev().map(|p| to(p[0] * 1e3, -p[1] * 1e3)).collect();
        poly.append(&mut back);
        painter.add(egui::Shape::closed_line(poly, Stroke::new(2.0, Color32::from_rgb(30, 60, 120))));
    }
    // núcleo (somente exibição)
    if let Some((cp, cc)) = core {
        if cp.len() >= 2 {
            let c = Color32::from_rgba_unmultiplied(cc.r(), cc.g(), cc.b(), 210);
            let mut mesh = egui::Mesh::default();
            for w in cp.windows(2) {
                let q = [to(w[0][0] * 1e3, w[0][1] * 1e3), to(w[1][0] * 1e3, w[1][1] * 1e3), to(w[1][0] * 1e3, -w[1][1] * 1e3), to(w[0][0] * 1e3, -w[0][1] * 1e3)];
                let base = mesh.vertices.len() as u32;
                for p in q {
                    mesh.colored_vertex(p, c);
                }
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
    // cotas
    painter.text(to(len / 2.0, -rmax - 2.0), Align2::CENTER_TOP, format!("L = {len:.1} mm   ⌀ máx = {:.1} mm", rmax * 2.0), FontId::proportional(13.0), Color32::from_rgb(40, 60, 100));

    let mut changed = false;
    let hit = |p: Pos2, prof: &Vec<[f64; 2]>| -> Option<usize> {
        prof.iter().enumerate().map(|(i, q)| (i, to(q[0] * 1e3, q[1] * 1e3).distance(p))).filter(|(_, d)| *d < 22.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    };
    if ed.selected.map_or(false, |i| i >= profile.len()) {
        ed.selected = None;
    }
    let snap = ed.snap_mm.max(0.05);
    let snapv = |v: f64| (v / snap).round() * snap;
    let red = Color32::from_rgb(210, 60, 50);
    let green = Color32::from_rgb(40, 150, 70);

    if ed.tool == Tool::Move {
        let handles = ed.selected.map(|i| gizmo_handles(to(profile[i][0] * 1e3, profile[i][1] * 1e3), rect));
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                ed.gizmo = None;
                ed.drag = None;
                if let (Some(i), Some((hh, hv))) = (ed.selected, handles) {
                    if i != 0 && hh.distance(p) < 26.0 {
                        ed.gizmo = Some((Axis::Horizontal, profile[i][0] * 1e3, p.x));
                    } else if hv.distance(p) < 26.0 {
                        ed.gizmo = Some((Axis::Vertical, profile[i][1] * 1e3, p.y));
                    }
                }
                if ed.gizmo.is_none() {
                    ed.drag = hit(p, profile);
                    if ed.drag.is_some() {
                        ed.selected = ed.drag;
                    }
                }
            }
        }
        if let (Some((axis, start, p0)), Some(p), Some(i)) = (ed.gizmo, resp.interact_pointer_pos(), ed.selected) {
            let n = profile.len();
            match axis {
                Axis::Horizontal => {
                    let lo = if i == 0 { 0.0 } else { profile[i - 1][0] * 1e3 + snap };
                    let hi = if i + 1 == n { 400.0 } else { profile[i + 1][0] * 1e3 - snap };
                    let a = snapv(start - (p.x - p0) as f64 / scale as f64).clamp(lo, hi.max(lo));
                    if (profile[i][0] * 1e3 - a).abs() > 1e-9 {
                        profile[i][0] = a / 1e3;
                        changed = true;
                    }
                }
                Axis::Vertical => {
                    let r = snapv(start - (p.y - p0) as f64 / scale as f64).clamp(0.0, 40.0);
                    if (profile[i][1] * 1e3 - r).abs() > 1e-9 {
                        profile[i][1] = r / 1e3;
                        changed = true;
                    }
                }
            }
        } else if let (Some(i), Some(p)) = (ed.drag, resp.interact_pointer_pos()) {
            let (a, r) = from(p);
            let n = profile.len();
            let lo = if i == 0 { 0.0 } else { profile[i - 1][0] * 1e3 + snap };
            let hi = if i + 1 == n { 400.0 } else { profile[i + 1][0] * 1e3 - snap };
            let a = if i == 0 { 0.0 } else { snapv(a).clamp(lo, hi.max(lo)) };
            let r = snapv(r).clamp(0.0, 40.0);
            if (profile[i][0] * 1e3 - a).abs() > 1e-9 || (profile[i][1] * 1e3 - r).abs() > 1e-9 {
                profile[i] = [a / 1e3, r / 1e3];
                changed = true;
            }
        }
        if resp.drag_stopped() {
            ed.drag = None;
            ed.gizmo = None;
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                ed.selected = hit(p, profile).or_else(|| {
                    // toque numa seta não deve desselecionar
                    ed.selected.filter(|&i| {
                        let (hh, hv) = gizmo_handles(to(profile[i][0] * 1e3, profile[i][1] * 1e3), rect);
                        hh.distance(p) < 26.0 || hv.distance(p) < 26.0
                    })
                });
            }
        }
    } else if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            match ed.tool {
                Tool::Add => {
                    let (a, r) = from(p);
                    let (a, r) = (snapv(a).max(0.0), snapv(r).clamp(0.0, 40.0));
                    let pos = profile.iter().position(|q| q[0] * 1e3 > a).unwrap_or(profile.len());
                    if pos > 0 && (profile[pos - 1][0] * 1e3 - a).abs() > 1e-9 {
                        profile.insert(pos, [a / 1e3, r / 1e3]);
                        ed.selected = Some(pos);
                        changed = true;
                    }
                }
                Tool::Delete => {
                    if let Some(i) = hit(p, profile) {
                        if profile.len() > 2 && i != 0 {
                            profile.remove(i);
                            ed.selected = None;
                            changed = true;
                        }
                    }
                }
                Tool::Move => {}
            }
        }
    }

    // pontos
    for (i, q) in profile.iter().enumerate() {
        let sel = ed.selected == Some(i);
        let c = if sel { Color32::from_rgb(255, 140, 0) } else { Color32::from_rgb(30, 90, 200) };
        let pp = to(q[0] * 1e3, q[1] * 1e3);
        painter.circle_filled(pp, if sel { 7.5 } else { 6.0 }, c);
        painter.circle_stroke(pp, if sel { 7.5 } else { 6.0 }, Stroke::new(1.5, Color32::WHITE));
    }
    // setas do ponto selecionado (afastadas para o ponto continuar visível)
    if let (Some(i), Tool::Move) = (ed.selected, ed.tool) {
        let pp = to(profile[i][0] * 1e3, profile[i][1] * 1e3);
        let (hh, hv) = gizmo_handles(pp, rect);
        let dim = Stroke::new(1.0, Color32::from_rgba_unmultiplied(90, 100, 120, 120));
        painter.line_segment([pp, hh], dim);
        painter.line_segment([pp, hv], dim);
        let locked = i == 0;
        arrow(&painter, hh, true, if locked { Color32::GRAY } else { red }, matches!(ed.gizmo, Some((Axis::Horizontal, _, _))));
        arrow(&painter, hv, false, green, matches!(ed.gizmo, Some((Axis::Vertical, _, _))));
        let (off, al) = if pp.x + 190.0 > rect.right() { (egui::Vec2::new(-10.0, 12.0), Align2::RIGHT_TOP) } else { (egui::Vec2::new(10.0, 12.0), Align2::LEFT_TOP) };
        painter.text(pp + off, al, format!("a = {:.1}  r = {:.1} mm", profile[i][0] * 1e3, profile[i][1] * 1e3), FontId::proportional(12.0), Color32::from_rgb(40, 60, 100));
    }
    painter.text(Pos2::new(rect.left() + 8.0, rect.top() + 6.0), Align2::LEFT_TOP, "ESBOÇO · plano a–r (a = distância da ponta)", FontId::proportional(12.0), Color32::from_rgb(90, 100, 120));
    changed
}

/// Posições (x0, x1) em mm de cada camada ao longo do eixo de voo.
pub fn layer_spans(layers: &[Layer]) -> Vec<(f64, f64)> {
    let mut cur = 0.0;
    layers
        .iter()
        .map(|l| {
            cur += l.gap_before * 1e3;
            let x0 = cur;
            cur += l.thickness * 1e3;
            (x0, cur)
        })
        .collect()
}

/// Editor 2D (vista lateral x–z) do alvo: vértices frontal (folga) e traseiro (espessura)
/// de cada camada; a seta vertical altera a altura. Retorna `true` se algo mudou.
pub fn show_target(ui: &mut egui::Ui, ed: &mut SketchEditor, layers: &mut Vec<Layer>, proj: &[[f64; 2]], colors: &[Color32], angle_deg: f64) -> bool {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, Color32::from_rgb(250, 251, 253));
    let spans = layer_spans(layers);
    let total = spans.last().map_or(10.0, |s| s.1);
    let plen = proj.last().map_or(0.0, |p| p[0]) * 1e3;
    let hmax = layers.iter().map(|l| l.height * 1e3).fold(20.0, f64::max);
    let xmin = -(plen + 8.0);
    let xmax = total + 14.0;
    let scale = (((rect.width() - 60.0) as f64 / (xmax - xmin)).min((rect.height() - 60.0) as f64 / (hmax + 10.0))) as f32;
    let origin = Pos2::new(rect.left() + 30.0 - xmin as f32 * scale, rect.center().y);
    let to = |x: f64, y: f64| Pos2::new(origin.x + x as f32 * scale, origin.y - y as f32 * scale);
    let from = |p: Pos2| ((p.x - origin.x) as f64 / scale as f64, (origin.y - p.y) as f64 / scale as f64);

    let minor = Stroke::new(1.0, Color32::from_rgb(232, 236, 242));
    let major = Stroke::new(1.0, Color32::from_rgb(205, 212, 224));
    let fine = scale >= 7.0; // linhas de 1 mm só quando há espaço
    for k in (xmin.floor() as i32)..=(xmax.ceil() as i32) {
        if k % 5 != 0 && !fine {
            continue;
        }
        let x = origin.x + k as f32 * scale;
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], if k % 5 == 0 { major } else { minor });
    }
    for k in -(hmax as i32 / 2 + 8)..=(hmax as i32 / 2 + 8) {
        if k % 5 != 0 && !fine {
            continue;
        }
        let y = origin.y - k as f32 * scale;
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], if k % 5 == 0 { major } else { minor });
    }
    painter.line_segment([Pos2::new(rect.left(), origin.y), Pos2::new(rect.right(), origin.y)], Stroke::new(1.5, Color32::from_rgb(200, 60, 60)));
    painter.text(Pos2::new(rect.left() + 6.0, origin.y - 4.0), Align2::LEFT_BOTTOM, "eixo de voo", FontId::proportional(11.0), Color32::from_rgb(200, 60, 60));

    // projétil de referência na direção de impacto (ângulo medido a partir da normal da placa)
    if proj.len() >= 2 {
        let al = angle_deg.to_radians();
        let d = (al.cos(), -al.sin()); // direção de voo no plano x–z (desce quando α > 0)
        let v = (al.sin(), al.cos());
        let tip = (-0.6 - 0.0, 0.0);
        let world = |a: f64, r: f64| to(tip.0 - d.0 * a + v.0 * r, tip.1 - d.1 * a + v.1 * r);
        let mut poly: Vec<Pos2> = proj.iter().map(|p| world(p[0] * 1e3, p[1] * 1e3)).collect();
        let mut back: Vec<Pos2> = proj.iter().rev().map(|p| world(p[0] * 1e3, -p[1] * 1e3)).collect();
        poly.append(&mut back);
        painter.add(egui::Shape::convex_polygon(poly, Color32::from_rgba_unmultiplied(120, 130, 150, 90), Stroke::new(1.0, Color32::from_rgb(100, 110, 130))));
        // seta da trajetória e rótulo do ângulo
        let len = (plen + 12.0).max(25.0);
        let a0 = world(len, 0.0);
        let a1 = world(0.0, 0.0);
        painter.line_segment([a0, a1], Stroke::new(1.5, Color32::from_rgb(200, 60, 60)));
        painter.text(a0 + egui::Vec2::new(6.0, -6.0), Align2::LEFT_BOTTOM, format!("α = {angle_deg:.0}°"), FontId::proportional(13.0), Color32::from_rgb(200, 60, 60));
    }

    // camadas
    for (i, l) in layers.iter().enumerate() {
        let (x0, x1) = spans[i];
        let h = l.height * 1e3 / 2.0;
        let r = egui::Rect::from_min_max(to(x0, h), to(x1, -h));
        let c = colors.get(i).copied().unwrap_or(Color32::GRAY);
        painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 200));
        painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(30, 60, 120)), egui::StrokeKind::Inside);
        painter.text(r.center(), Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(14.0), Color32::from_gray(30));
    }

    let snap = ed.snap_mm.max(0.1);
    let snapv = |v: f64| (v / snap).round() * snap;
    let corner = |ls: &Vec<Layer>, i: usize, back: bool| -> Pos2 {
        let sp = layer_spans(ls);
        let x = if back { sp[i].1 } else { sp[i].0 };
        to(x, ls[i].height * 1e3 / 2.0)
    };
    let mut changed = false;
    let hit = |ls: &Vec<Layer>, p: Pos2| -> Option<(usize, bool)> {
        let mut best: Option<((usize, bool), f32)> = None;
        for i in 0..ls.len() {
            for back in [false, true] {
                let d = corner(ls, i, back).distance(p);
                if d < 22.0 && best.map_or(true, |b| d < b.1) {
                    best = Some(((i, back), d));
                }
            }
        }
        best.map(|b| b.0)
    };
    if ed.tsel.map_or(false, |(i, _)| i >= layers.len()) {
        ed.tsel = None;
    }
    let red = Color32::from_rgb(210, 60, 50);
    let green = Color32::from_rgb(40, 150, 70);

    let handles = ed.tsel.map(|(i, b)| gizmo_handles(corner(layers, i, b), rect));
    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            ed.gizmo = None;
            ed.tdrag = None;
            if let (Some((i, back)), Some((hh, hv))) = (ed.tsel, handles) {
                let sp = layer_spans(layers);
                let locked_h = !back && i == 0;
                if !locked_h && hh.distance(p) < 26.0 {
                    ed.gizmo = Some((Axis::Horizontal, if back { sp[i].1 } else { sp[i].0 }, p.x));
                } else if hv.distance(p) < 26.0 {
                    ed.gizmo = Some((Axis::Vertical, layers[i].height * 1e3 / 2.0, p.y));
                }
            }
            if ed.gizmo.is_none() {
                ed.tdrag = hit(layers, p);
                if ed.tdrag.is_some() {
                    ed.tsel = ed.tdrag;
                }
            }
        }
    }
    let apply_x = |ls: &mut Vec<Layer>, i: usize, back: bool, x: f64| -> bool {
        let sp = layer_spans(ls);
        let x = snapv(x);
        if back {
            let t = (x - sp[i].0).max(1.0) / 1e3;
            if (ls[i].thickness - t).abs() > 1e-9 {
                ls[i].thickness = t;
                return true;
            }
        } else if i > 0 {
            let prev_end = sp[i - 1].1;
            let g = (x - prev_end).max(0.0) / 1e3;
            if (ls[i].gap_before - g).abs() > 1e-9 {
                ls[i].gap_before = g;
                return true;
            }
        }
        false
    };
    let apply_y = |ls: &mut Vec<Layer>, i: usize, y: f64| -> bool {
        let h = (2.0 * snapv(y)).clamp(20.0, 500.0) / 1e3;
        let mut ch = false;
        if (ls[i].height - h).abs() > 1e-9 {
            ls[i].height = h;
            ch = true;
        }
        if ls[i].round && (ls[i].width - h).abs() > 1e-9 {
            ls[i].width = h;
            ch = true;
        }
        ch
    };
    if let (Some((axis, start, p0)), Some(p), Some((i, back))) = (ed.gizmo, resp.interact_pointer_pos(), ed.tsel) {
        changed |= match axis {
            Axis::Horizontal => apply_x(layers, i, back, start + (p.x - p0) as f64 / scale as f64),
            Axis::Vertical => apply_y(layers, i, start - (p.y - p0) as f64 / scale as f64),
        };
    } else if let (Some((i, back)), Some(p)) = (ed.tdrag, resp.interact_pointer_pos()) {
        let (x, y) = from(p);
        changed |= apply_x(layers, i, back, x);
        changed |= apply_y(layers, i, y);
    }
    if resp.drag_stopped() {
        ed.tdrag = None;
        ed.gizmo = None;
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            ed.tsel = hit(layers, p).or_else(|| {
                ed.tsel.filter(|&(i, b)| {
                    let (hh, hv) = gizmo_handles(corner(layers, i, b), rect);
                    hh.distance(p) < 26.0 || hv.distance(p) < 26.0
                })
            });
        }
    }

    // vértices e setas
    for i in 0..layers.len() {
        for back in [false, true] {
            let sel = ed.tsel == Some((i, back));
            let c = if sel { Color32::from_rgb(255, 140, 0) } else { Color32::from_rgb(30, 90, 200) };
            let pp = corner(layers, i, back);
            painter.circle_filled(pp, if sel { 7.5 } else { 6.0 }, c);
            painter.circle_stroke(pp, if sel { 7.5 } else { 6.0 }, Stroke::new(1.5, Color32::WHITE));
        }
    }
    if let Some((i, back)) = ed.tsel {
        let pp = corner(layers, i, back);
        let (hh, hv) = gizmo_handles(pp, rect);
        let dim = Stroke::new(1.0, Color32::from_rgba_unmultiplied(90, 100, 120, 120));
        painter.line_segment([pp, hh], dim);
        painter.line_segment([pp, hv], dim);
        let locked = !back && i == 0;
        arrow(&painter, hh, true, if locked { Color32::GRAY } else { red }, matches!(ed.gizmo, Some((Axis::Horizontal, _, _))));
        arrow(&painter, hv, false, green, matches!(ed.gizmo, Some((Axis::Vertical, _, _))));
        let sp = layer_spans(layers);
        let txt = if back { format!("espessura {:.1} mm · altura {:.0} mm", layers[i].thickness * 1e3, layers[i].height * 1e3) } else { format!("folga {:.1} mm · altura {:.0} mm", layers[i].gap_before * 1e3, layers[i].height * 1e3) };
        let _ = sp;
        let (off, al) = if pp.x + 230.0 > rect.right() { (egui::Vec2::new(-10.0, 14.0), Align2::RIGHT_TOP) } else { (egui::Vec2::new(10.0, 14.0), Align2::LEFT_TOP) };
        painter.text(pp + off, al, txt, FontId::proportional(12.0), Color32::from_rgb(40, 60, 100));
    }
    painter.text(Pos2::new(rect.left() + 8.0, rect.top() + 6.0), Align2::LEFT_TOP, "ALVO · vista lateral (x = espessura, vertical = altura)", FontId::proportional(12.0), Color32::from_rgb(90, 100, 120));
    changed
}
