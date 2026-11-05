use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

pub struct Series<'a> {
    pub name: &'a str,
    pub color: Color32,
    pub pts: Vec<(f64, f64)>,
    pub markers: bool,
}

/// Gráfico de linhas simples (sem dependências extras).
pub fn line_plot(ui: &mut Ui, height: f32, title: &str, xl: &str, yl: &str, series: &[Series]) {
    ui.label(egui::RichText::new(title).strong());
    ui.horizontal_wrapped(|ui| {
        for s in series {
            ui.colored_label(s.color, format!("■ {}", s.name));
        }
    });
    let (resp, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), height), Sense::hover());
    let r = resp.rect;
    let vis = ui.visuals();
    painter.rect_filled(r, 4.0, vis.extreme_bg_color);
    let plot = Rect::from_min_max(
        Pos2::new(r.left() + 46.0, r.top() + 8.0),
        Pos2::new(r.right() - 10.0, r.bottom() - 26.0),
    );
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for s in series {
        for &(x, y) in &s.pts {
            if x.is_finite() && y.is_finite() {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    if x0 >= x1 || y0 > y1 {
        painter.text(r.center(), Align2::CENTER_CENTER, "sem dados", FontId::proportional(13.0), vis.weak_text_color());
        return;
    }
    if (y1 - y0).abs() < 1e-12 {
        y1 = y0 + 1.0;
    }
    let pad = (y1 - y0) * 0.05;
    y0 -= pad;
    y1 += pad;
    let tx = |x: f64| plot.left() + ((x - x0) / (x1 - x0)) as f32 * plot.width();
    let ty = |y: f64| plot.bottom() - ((y - y0) / (y1 - y0)) as f32 * plot.height();
    let grid = Stroke::new(1.0, vis.widgets.noninteractive.bg_stroke.color);
    let font = FontId::proportional(11.0);
    for k in 0..=4 {
        let f = k as f64 / 4.0;
        let (gx, gy) = (x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
        painter.line_segment([Pos2::new(tx(gx), plot.top()), Pos2::new(tx(gx), plot.bottom())], grid);
        painter.line_segment([Pos2::new(plot.left(), ty(gy)), Pos2::new(plot.right(), ty(gy))], grid);
        painter.text(Pos2::new(tx(gx), plot.bottom() + 3.0), Align2::CENTER_TOP, fmt(gx), font.clone(), vis.text_color());
        painter.text(Pos2::new(plot.left() - 4.0, ty(gy)), Align2::RIGHT_CENTER, fmt(gy), font.clone(), vis.text_color());
    }
    painter.text(Pos2::new(plot.center().x, r.bottom() - 1.0), Align2::CENTER_BOTTOM, xl, font.clone(), vis.weak_text_color());
    painter.text(Pos2::new(r.left() + 2.0, r.top() + 1.0), Align2::LEFT_TOP, yl, font.clone(), vis.weak_text_color());
    for s in series {
        let pts: Vec<Pos2> = s.pts.iter().map(|&(x, y)| Pos2::new(tx(x), ty(y))).collect();
        if s.markers {
            for p in &pts {
                painter.circle_filled(*p, 4.0, s.color);
            }
        } else {
            painter.add(Shape::line(pts, Stroke::new(2.0, s.color)));
        }
    }
}

fn fmt(v: f64) -> String {
    let a = v.abs();
    if a != 0.0 && !(0.01..10000.0).contains(&a) {
        format!("{v:.1e}")
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}
