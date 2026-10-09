use tiny_skia::{Color, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub const GREEN: (u8, u8, u8) = (67, 160, 71);

pub fn color((r, g, b): (u8, u8, u8), alpha: u8) -> Color {
    Color::from_rgba8(r, g, b, alpha)
}

pub fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint
}

pub fn arc(pixmap: &mut Pixmap, x: f32, y: f32, radius: f32, from: f32, to: f32, color: Color) {
    let mut builder = PathBuilder::new();
    let steps = 48;
    for i in 0..=steps {
        let angle = from + (to - from) * i as f32 / steps as f32;
        let point = (x + radius * angle.cos(), y + radius * angle.sin());
        if i == 0 { builder.move_to(point.0, point.1) } else { builder.line_to(point.0, point.1) }
    }
    if let Some(path) = builder.finish() {
        let stroke = Stroke { width: 4.0, line_cap: LineCap::Round, ..Stroke::default() };
        pixmap.stroke_path(&path, &paint(color), &stroke, Transform::identity(), None);
    }
}
