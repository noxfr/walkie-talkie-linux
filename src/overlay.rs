use std::f32::consts::TAU;
use std::thread;
use std::time::{Duration, Instant};

use tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
use x11rb::connection::Connection;
use x11rb::protocol::shape::SK;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{
    ColormapAlloc, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, Gcontext, ImageFormat, StackMode,
    VisualClass, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

const SIZE: u16 = 44;
const OFFSET: i16 = 16;
const RED: (u8, u8, u8) = (229, 57, 53);
const ORANGE: (u8, u8, u8) = (251, 140, 0);
const GREEN: (u8, u8, u8) = (67, 160, 71);

pub enum Look {
    Hidden,
    Listening { level: f32, attachments: usize },
    Transcribing,
    Holding { remaining: f32 },
}

pub fn run(look: impl Fn() -> Look) {
    let Some(mut overlay) = Overlay::new() else { return };
    let started = Instant::now();
    loop {
        overlay.show(&look(), started.elapsed().as_secs_f32());
        thread::sleep(Duration::from_millis(33));
    }
}

struct Overlay {
    conn: RustConnection,
    root: Window,
    window: Window,
    gc: Gcontext,
    mapped: bool,
}

impl Overlay {
    fn new() -> Option<Self> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen].root;
        let visual = conn.setup().roots[screen]
            .allowed_depths
            .iter()
            .filter(|depth| depth.depth == 32)
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.class == VisualClass::TRUE_COLOR)?
            .visual_id;
        let colormap = conn.generate_id().ok()?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual).ok()?;
        let window = conn.generate_id().ok()?;
        let aux = CreateWindowAux::new().background_pixel(0).border_pixel(0).override_redirect(1).colormap(colormap);
        conn.create_window(32, window, root, 0, 0, SIZE, SIZE, 0, WindowClass::INPUT_OUTPUT, visual, &aux).ok()?;
        conn.xfixes_query_version(5, 0).ok()?.reply().ok()?;
        let region = conn.generate_id().ok()?;
        conn.xfixes_create_region(region, &[]).ok()?;
        conn.xfixes_set_window_shape_region(window, SK::INPUT, 0, 0, region).ok()?;
        let gc = conn.generate_id().ok()?;
        conn.create_gc(gc, window, &CreateGCAux::new()).ok()?;
        conn.flush().ok()?;
        Some(Self { conn, root, window, gc, mapped: false })
    }

    fn show(&mut self, look: &Look, time: f32) -> Option<()> {
        let Some(pixmap) = draw(look, time) else {
            if self.mapped {
                self.conn.unmap_window(self.window).ok()?;
                self.mapped = false;
            }
            return self.conn.flush().ok();
        };
        let pointer = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        let position = ConfigureWindowAux::new()
            .x(i32::from(pointer.root_x + OFFSET))
            .y(i32::from(pointer.root_y + OFFSET))
            .stack_mode(StackMode::ABOVE);
        self.conn.configure_window(self.window, &position).ok()?;
        if !self.mapped {
            self.conn.map_window(self.window).ok()?;
            self.mapped = true;
        }
        let bgra: Vec<u8> = pixmap.data().chunks_exact(4).flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]]).collect();
        self.conn.put_image(ImageFormat::Z_PIXMAP, self.window, self.gc, SIZE, SIZE, 0, 0, 0, 32, &bgra).ok()?;
        self.conn.flush().ok()
    }
}

fn draw(look: &Look, time: f32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(SIZE.into(), SIZE.into())?;
    let center = f32::from(SIZE) / 2.0;
    match *look {
        Look::Hidden => return None,
        Look::Listening { level, attachments } => {
            circle(&mut pixmap, center, center, 8.0 + level * 12.0, color(RED, 70));
            circle(&mut pixmap, center, center, 7.0, color(RED, 230));
            for i in 0..attachments.min(5) {
                let x = 8.0 + i as f32 * 7.0;
                circle(&mut pixmap, x, f32::from(SIZE) - 4.0, 3.5, color((40, 40, 40), 200));
                circle(&mut pixmap, x, f32::from(SIZE) - 4.0, 2.5, Color::WHITE);
            }
        }
        Look::Transcribing => {
            let start = time * TAU;
            arc(&mut pixmap, center, 10.0, start, start + TAU * 0.7, color(ORANGE, 230));
        }
        Look::Holding { remaining } => {
            arc(&mut pixmap, center, 10.0, -TAU / 4.0, -TAU / 4.0 + TAU * remaining, color(GREEN, 230));
            circle(&mut pixmap, center, center, 4.0, color(GREEN, 230));
        }
    }
    Some(pixmap)
}

fn color((r, g, b): (u8, u8, u8), alpha: u8) -> Color {
    Color::from_rgba8(r, g, b, alpha)
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint
}

fn circle(pixmap: &mut Pixmap, x: f32, y: f32, radius: f32, color: Color) {
    if let Some(path) = PathBuilder::from_circle(x, y, radius) {
        pixmap.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
    }
}

fn arc(pixmap: &mut Pixmap, center: f32, radius: f32, from: f32, to: f32, color: Color) {
    let mut builder = PathBuilder::new();
    let steps = 48;
    for i in 0..=steps {
        let angle = from + (to - from) * i as f32 / steps as f32;
        let (x, y) = (center + radius * angle.cos(), center + radius * angle.sin());
        if i == 0 { builder.move_to(x, y) } else { builder.line_to(x, y) }
    }
    if let Some(path) = builder.finish() {
        let stroke = Stroke { width: 4.0, line_cap: LineCap::Round, ..Stroke::default() };
        pixmap.stroke_path(&path, &paint(color), &stroke, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::{Look, SIZE, draw};

    fn opaque_pixels(look: &Look) -> usize {
        draw(look, 0.0).unwrap().data().chunks_exact(4).filter(|rgba| rgba[3] > 0).count()
    }

    #[test]
    fn rien_n_est_dessine_au_repos() {
        assert!(draw(&Look::Hidden, 0.0).is_none());
    }

    #[test]
    fn le_halo_grandit_avec_la_voix() {
        let silent = opaque_pixels(&Look::Listening { level: 0.0, attachments: 0 });
        let loud = opaque_pixels(&Look::Listening { level: 1.0, attachments: 0 });
        assert!(loud > silent);
        assert!(loud < usize::from(SIZE) * usize::from(SIZE));
    }

    #[test]
    fn l_anneau_d_attente_se_vide() {
        let full = opaque_pixels(&Look::Holding { remaining: 1.0 });
        let almost_done = opaque_pixels(&Look::Holding { remaining: 0.1 });
        assert!(full > almost_done);
    }
}
