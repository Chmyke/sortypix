//! A slender contoured silhouette with inset, matching left and right button curves.
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke};
/// Left and right button meanings, shared by the tooltip, the swap notice and the empty screen.
pub fn legend(inverted: bool) -> [&'static str; 2] {
    if inverted {
        ["Left click: keep", "Right click: reject"]
    } else {
        ["Left click: reject", "Right click: keep"]
    }
}
pub fn visual_size(height: f32) -> egui::Vec2 {
    egui::vec2(30. * height / 46., height)
}
pub fn paint(painter: &egui::Painter, rect: Rect, inverted: bool) {
    let mut body = vec![egui::pos2(17., 1.)];
    curve(&mut body, [(8., 1.), (2., 6.), (2., 14.)]);
    curve(&mut body, [(1., 19.), (1., 23.), (1., 29.)]);
    curve(&mut body, [(1., 40.), (8., 45.), (17., 45.)]);
    curve(&mut body, [(26., 45.), (33., 40.), (33., 29.)]);
    curve(&mut body, [(33., 23.), (33., 19.), (32., 14.)]);
    curve(&mut body, [(32., 6.), (26., 1.), (17., 1.)]);
    polygon(painter, rect, body, theme::CARD2, theme::DIM);
    let mut button = vec![egui::pos2(15.5, 3.)];
    curve(&mut button, [(8., 3.), (4., 7.), (4., 14.)]);
    curve(&mut button, [(3.5, 17.), (3., 19.), (3., 21.)]);
    curve(&mut button, [(6., 23.), (12., 24.), (15.5, 23.)]);
    let right = button.iter().map(|p| egui::pos2(34. - p.x, p.y)).collect();
    let (left_color, right_color) = if inverted {
        (theme::GREEN, theme::RED)
    } else {
        (theme::RED, theme::GREEN)
    };
    polygon(
        painter,
        rect,
        button,
        left_color.linear_multiply(0.45),
        left_color,
    );
    polygon(
        painter,
        rect,
        right,
        right_color.linear_multiply(0.45),
        right_color,
    );
    let wheel = Rect::from_min_max(
        point(rect, egui::pos2(15.8, 9.)),
        point(rect, egui::pos2(18.2, 18.)),
    );
    painter.rect_filled(wheel, 1, theme::AMBER);
    painter.line_segment(
        [
            point(rect, egui::pos2(17., 26.)),
            point(rect, egui::pos2(17., 32.)),
        ],
        Stroke::new(0.7, theme::LINE),
    );
}
fn point(rect: Rect, p: Pos2) -> Pos2 {
    rect.min + egui::vec2(p.x * rect.width() / 34., p.y * rect.height() / 46.)
}
fn polygon(painter: &egui::Painter, rect: Rect, points: Vec<Pos2>, fill: Color32, stroke: Color32) {
    painter.add(Shape::convex_polygon(
        points.into_iter().map(|p| point(rect, p)).collect(),
        fill,
        Stroke::new(0.7, stroke),
    ));
}
fn curve(points: &mut Vec<Pos2>, control: [(f32, f32); 3]) {
    let start = *points.last().expect("Curve starts on the outline");
    let [a, b, end] = control.map(|(x, y)| egui::pos2(x, y));
    for step in 1..=10 {
        let t = step as f32 / 10.;
        let u = 1. - t;
        points.push(
            (start.to_vec2() * u.powi(3)
                + a.to_vec2() * (3. * u * u * t)
                + b.to_vec2() * (3. * u * t * t)
                + end.to_vec2() * t.powi(3))
            .to_pos2(),
        );
    }
}
