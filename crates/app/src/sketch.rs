//! Editor de esboço 2D do perfil do projétil (revolvido em torno do eixo).
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Stroke};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Move,
    Add,
    Delete,
}

pub struct SketchEditor {
    pub tool: Tool,
    pub snap_mm: f64,
    drag: Option<usize>,
}

impl Default for SketchEditor {
    fn default() -> Self {
        SketchEditor { tool: Tool::Move, snap_mm: 0.5, drag: None }
    }
}

/// Retorna `true` se o perfil mudou. `profile` em metros: (a = distância da ponta, r = raio).
pub fn show(ui: &mut egui::Ui, ed: &mut SketchEditor, profile: &mut Vec<[f64; 2]>, fill: Color32) -> bool {
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
    // cotas
    painter.text(to(len / 2.0, -rmax - 2.0), Align2::CENTER_TOP, format!("L = {len:.1} mm   ⌀ máx = {:.1} mm", rmax * 2.0), FontId::proportional(13.0), Color32::from_rgb(40, 60, 100));

    let mut changed = false;
    let hit = |p: Pos2, prof: &Vec<[f64; 2]>| -> Option<usize> {
        prof.iter().enumerate().map(|(i, q)| (i, to(q[0] * 1e3, q[1] * 1e3).distance(p))).filter(|(_, d)| *d < 22.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    };
    for (i, q) in profile.iter().enumerate() {
        let c = if ed.drag == Some(i) { Color32::from_rgb(255, 140, 0) } else { Color32::from_rgb(30, 90, 200) };
        painter.circle_filled(to(q[0] * 1e3, q[1] * 1e3), 6.0, c);
        painter.circle_stroke(to(q[0] * 1e3, q[1] * 1e3), 6.0, Stroke::new(1.5, Color32::WHITE));
    }
    let snap = ed.snap_mm.max(0.05);
    let snapv = |v: f64| (v / snap).round() * snap;
    if ed.tool == Tool::Move {
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                ed.drag = hit(p, profile);
            }
        }
        if let (Some(i), Some(p)) = (ed.drag, resp.interact_pointer_pos()) {
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
                        changed = true;
                    }
                }
                Tool::Delete => {
                    if let Some(i) = hit(p, profile) {
                        if profile.len() > 2 && i != 0 {
                            profile.remove(i);
                            changed = true;
                        }
                    }
                }
                Tool::Move => {}
            }
        }
    }
    painter.text(Pos2::new(rect.left() + 8.0, rect.top() + 6.0), Align2::LEFT_TOP, "ESBOÇO · plano a–r (a = distância da ponta)", FontId::proportional(12.0), Color32::from_rgb(90, 100, 120));
    changed
}
