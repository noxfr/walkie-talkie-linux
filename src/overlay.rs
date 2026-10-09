use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use x11rb::connection::Connection;
use x11rb::protocol::shape::SK;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{
    ColormapAlloc, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, Gcontext, ImageFormat, StackMode,
    VisualClass, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::capture;
use crate::panel::Panel;

const WIDTH: u16 = 108;
const HEIGHT: u16 = 38;
const SYMBOL: f32 = 19.0;
const OFFSET: i16 = 16;
const RED: (u8, u8, u8) = (229, 57, 53);
const ORANGE: (u8, u8, u8) = (251, 140, 0);
pub const GREEN: (u8, u8, u8) = (67, 160, 71);
const GREY: (u8, u8, u8) = (150, 150, 150);
const BLUE: (u8, u8, u8) = (30, 136, 229);

pub enum Look {
    Hidden,
    Listening { plain: bool, wave: Vec<(f32, bool)>, attachments: usize },
    Transcribing,
    Holding { remaining: f32, spoken: String, footer: String },
}

pub static PANEL_READY: AtomicBool = AtomicBool::new(false);

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
    screen: usize,
    root: Window,
    window: Window,
    gc: Gcontext,
    mapped: bool,
    panel: Option<(Panel, Window)>,
    panel_mapped: bool,
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
        conn.xfixes_query_version(5, 0).ok()?.reply().ok()?;
        let region = conn.generate_id().ok()?;
        conn.xfixes_create_region(region, &[]).ok()?;
        let create = |width: u16, height: u16| -> Option<Window> {
            let window = conn.generate_id().ok()?;
            let aux = CreateWindowAux::new().background_pixel(0).border_pixel(0).override_redirect(1).colormap(colormap);
            conn.create_window(32, window, root, 0, 0, width, height, 0, WindowClass::INPUT_OUTPUT, visual, &aux).ok()?;
            conn.xfixes_set_window_shape_region(window, SK::INPUT, 0, 0, region).ok()?;
            Some(window)
        };
        let window = create(WIDTH, HEIGHT)?;
        let panel = Panel::load().and_then(|panel| Some((panel, create(1, 1)?)));
        let gc = conn.generate_id().ok()?;
        conn.create_gc(gc, window, &CreateGCAux::new()).ok()?;
        conn.flush().ok()?;
        PANEL_READY.store(panel.is_some(), Ordering::Relaxed);
        Some(Self { conn, screen, root, window, gc, mapped: false, panel, panel_mapped: false })
    }

    fn show_panel(&mut self, look: &Look) -> Option<()> {
        let (Some((panel, window)), Look::Holding { remaining, spoken, footer }) = (&self.panel, look) else {
            if self.panel_mapped {
                self.panel_mapped = false;
                self.conn.unmap_window(self.panel.as_ref()?.1).ok()?;
            }
            return Some(());
        };
        let monitor = capture::monitor_under_pointer(&self.conn, self.screen)?;
        let pixmap = panel.draw(spoken, footer, *remaining, (monitor.width as f32 * 0.6).min(1000.0))?;
        let (width, height) = (pixmap.width(), pixmap.height());
        let place = ConfigureWindowAux::new()
            .x(monitor.x + monitor.width.saturating_sub(width) as i32 / 2)
            .y(monitor.y + monitor.height.saturating_sub(height) as i32 / 2)
            .width(width)
            .height(height)
            .stack_mode(StackMode::ABOVE);
        self.conn.configure_window(*window, &place).ok()?;
        if !self.panel_mapped {
            self.conn.map_window(*window).ok()?;
            self.panel_mapped = true;
        }
        self.conn.put_image(ImageFormat::Z_PIXMAP, *window, self.gc, width as u16, height as u16, 0, 0, 0, 32, &bgra(&pixmap)).ok()?;
        Some(())
    }

    fn show(&mut self, look: &Look, time: f32) -> Option<()> {
        self.show_panel(look);
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
        self.conn.put_image(ImageFormat::Z_PIXMAP, self.window, self.gc, WIDTH, HEIGHT, 0, 0, 0, 32, &bgra(&pixmap)).ok()?;
        self.conn.flush().ok()
    }
}

fn bgra(pixmap: &Pixmap) -> Vec<u8> {
    pixmap.data().chunks_exact(4).flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]]).collect()
}

fn draw(look: &Look, time: f32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(WIDTH.into(), HEIGHT.into())?;
    match look {
        Look::Hidden => return None,
        Look::Listening { plain, wave, attachments } => {
            let voice = if *plain { BLUE } else { RED };
            rounded(&mut pixmap, f32::from(WIDTH), f32::from(HEIGHT), color((30, 30, 30), 150));
            circle(&mut pixmap, SYMBOL, SYMBOL, 7.0, color(voice, 230));
            for (i, &(height, speech)) in wave.iter().enumerate() {
                let bar = (height * 24.0).max(2.0);
                let rect = Rect::from_xywh(36.0 + i as f32 * 5.0, SYMBOL - bar / 2.0, 3.0, bar);
                if let Some(rect) = rect {
                    pixmap.fill_rect(rect, &paint(color(if speech { voice } else { GREY }, 230)), Transform::identity(), None);
                }
            }
            for i in 0..(*attachments).min(5) {
                circle(&mut pixmap, 12.0 + i as f32 * 6.0, f32::from(HEIGHT) - 4.0, 2.0, Color::WHITE);
            }
        }
        Look::Transcribing => {
            let start = time * TAU;
            arc(&mut pixmap, SYMBOL, SYMBOL, 10.0, start, start + TAU * 0.7, color(ORANGE, 230));
        }
        Look::Holding { remaining, .. } => {
            arc(&mut pixmap, SYMBOL, SYMBOL, 10.0, -TAU / 4.0, -TAU / 4.0 + TAU * remaining, color(GREEN, 230));
            circle(&mut pixmap, SYMBOL, SYMBOL, 4.0, color(GREEN, 230));
        }
    }
    Some(pixmap)
}

pub fn color((r, g, b): (u8, u8, u8), alpha: u8) -> Color {
    Color::from_rgba8(r, g, b, alpha)
}

pub fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint
}

fn circle(pixmap: &mut Pixmap, x: f32, y: f32, radius: f32, color: Color) {
    if let Some(path) = PathBuilder::from_circle(x, y, radius) {
        pixmap.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
    }
}

fn rounded(pixmap: &mut Pixmap, width: f32, height: f32, color: Color) {
    let radius = height / 2.0;
    let mut builder = PathBuilder::new();
    builder.push_circle(radius, radius, radius);
    builder.push_circle(width - radius, radius, radius);
    if let Some(rect) = Rect::from_xywh(radius, 0.0, width - height, height) {
        builder.push_rect(rect);
    }
    if let Some(path) = builder.finish() {
        pixmap.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
    }
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

#[cfg(test)]
mod tests {
    use super::{Look, draw};

    fn painted(look: &Look, keep: impl Fn(&[u8]) -> bool) -> usize {
        draw(look, 0.0).unwrap().data().chunks_exact(4).filter(|rgba| keep(rgba)).count()
    }

    fn red(rgba: &[u8]) -> bool {
        rgba[0] > 150 && rgba[1] < 100
    }

    #[test]
    fn rien_n_est_dessine_au_repos() {
        assert!(draw(&Look::Hidden, 0.0).is_none());
    }

    #[test]
    fn la_wave_grandit_avec_la_voix_et_rougit_au_dessus_du_seuil() {
        let noise = painted(&Look::Listening { plain: false, wave: vec![(0.2, false); 14], attachments: 0 }, red);
        let speech = painted(&Look::Listening { plain: false, wave: vec![(0.8, true); 14], attachments: 0 }, red);
        assert!(speech > noise * 2);
    }

    #[test]
    fn la_dictee_simple_est_bleue() {
        let blue = |rgba: &[u8]| rgba[2] > 150 && rgba[0] < 100;
        let wave = vec![(0.8, true); 14];
        assert_eq!(painted(&Look::Listening { plain: false, wave: wave.clone(), attachments: 0 }, blue), 0);
        assert!(painted(&Look::Listening { plain: true, wave, attachments: 0 }, blue) > 0);
    }

    #[test]
    fn l_anneau_d_attente_se_vide() {
        let opaque = |rgba: &[u8]| rgba[3] > 0;
        let holding = |remaining| Look::Holding { remaining, spoken: String::new(), footer: String::new() };
        let full = painted(&holding(1.0), opaque);
        let almost_done = painted(&holding(0.1), opaque);
        assert!(full > almost_done);
    }
}

#[cfg(test)]
mod readme {
    use super::{Look, draw};
    use crate::panel::Panel;
    use tiny_skia::{Color, Pixmap, PixmapPaint, Transform};

    const BACKGROUND: (u8, u8, u8, u8) = (48, 52, 60, 255);
    const SCALE: f32 = 2.0;

    fn sheet(width: u32, height: u32) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let (r, g, b, a) = BACKGROUND;
        pixmap.fill(Color::from_rgba8(r, g, b, a));
        pixmap
    }

    fn paste(sheet: &mut Pixmap, image: &Pixmap, x: f32, y: f32, scale: f32) {
        let transform = Transform::from_row(scale, 0.0, 0.0, scale, x, y);
        sheet.draw_pixmap(0, 0, image.as_ref(), &PixmapPaint::default(), transform, None);
    }

    #[test]
    #[ignore = "régénère les images du README : cargo test -- --ignored readme"]
    fn images_du_readme() {
        let speech = [800u32, 4037, 5238, 2703, 8642, 9934, 6477, 3349, 5409, 7144, 2230, 450, 310, 260];
        let wave = |plain| Look::Listening {
            plain,
            wave: speech.iter().map(|&rms| ((rms as f32 / 12000.0).sqrt().min(1.0), rms > 1100)).collect(),
            attachments: if plain { 0 } else { 2 },
        };
        let looks = [wave(false), wave(true), Look::Transcribing, Look::Holding { remaining: 0.6, spoken: String::new(), footer: String::new() }];
        let mut indicator = sheet(4 * 260, 120);
        for (i, look) in looks.iter().enumerate() {
            paste(&mut indicator, &draw(look, 0.0).unwrap(), 24.0 + i as f32 * 260.0, 22.0, SCALE);
        }
        indicator.save_png("docs/readme/indicateur.png").unwrap();

        let panel = Panel::load().expect("police système introuvable");
        let card = panel
            .draw(
                "Le test de facturation échoue depuis la migration, regarde la capture et corrige le calcul de la TVA sur les avoirs.",
                "Entrée : envoyer · Échap : annuler · 1 capture · 1 texte surligné",
                0.6,
                1000.0,
            )
            .unwrap();
        let mut hold = sheet(card.width() + 120, card.height() + 120);
        paste(&mut hold, &card, 60.0, 60.0, 1.0);
        hold.save_png("docs/readme/panneau.png").unwrap();
    }
}

#[cfg(test)]
mod demo {
    use super::{Look, draw};
    use crate::panel::Panel;
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

    const FPS: f32 = 15.0;
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;
    const CURSOR: (f32, f32) = (900.0, 330.0);
    const SPOKEN: &str = "Corrige le test de facturation qui échoue depuis la migration";
    const VOICE: [u32; 40] = [
        677, 838, 1011, 917, 620, 798, 597, 379, 422, 338, 986, 380, 594, 1127, 1118, 818, 482, 431, 779, 910, 1397, 1512, 1054,
        371, 489, 783, 1835, 1422, 1272, 1151, 92, 617, 1266, 969, 1074, 683, 431, 400, 1222, 1036,
    ];

    fn rect(pixmap: &mut Pixmap, x: f32, y: f32, width: f32, height: f32, rgba: (u8, u8, u8, u8)) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(rgba.0, rgba.1, rgba.2, rgba.3));
        if let Some(rect) = Rect::from_xywh(x, y, width, height) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    fn arrow(pixmap: &mut Pixmap, (x, y): (f32, f32)) {
        let points = [(0.0, 0.0), (0.0, 22.0), (6.0, 16.5), (10.0, 25.0), (13.5, 23.5), (9.5, 15.5), (17.0, 15.5)];
        let mut builder = PathBuilder::new();
        for (i, (dx, dy)) in points.iter().enumerate() {
            if i == 0 { builder.move_to(x + dx, y + dy) } else { builder.line_to(x + dx, y + dy) }
        }
        builder.close();
        let path = builder.finish().unwrap();
        let mut white = Paint::default();
        white.set_color(Color::WHITE);
        pixmap.fill_path(&path, &white, FillRule::Winding, Transform::identity(), None);
        let mut black = Paint::default();
        black.set_color(Color::BLACK);
        pixmap.stroke_path(&path, &black, &tiny_skia::Stroke { width: 1.5, ..Default::default() }, Transform::identity(), None);
    }

    fn caption(pixmap: &mut Pixmap, font: &Panel, text: &str) {
        let mut probe = Pixmap::new(1, 1).unwrap();
        let width = font.write(&mut probe, text, 24.0, 0.0, 0.0, (0, 0, 0));
        let x = (WIDTH as f32 - width) / 2.0;
        rect(pixmap, x - 22.0, 636.0, width + 44.0, 48.0, (0, 0, 0, 170));
        font.write(pixmap, text, 24.0, x, 646.0, (255, 255, 255));
    }

    fn terminal(pixmap: &mut Pixmap, font: &Panel, typed: &str, sent: bool, caret: bool) {
        rect(pixmap, 80.0, 50.0, 1120.0, 560.0, (24, 24, 27, 255));
        rect(pixmap, 80.0, 50.0, 1120.0, 38.0, (38, 38, 43, 255));
        font.write(pixmap, "ghostty — claude", 16.0, 580.0, 60.0, (170, 170, 170));
        font.write(pixmap, "Claude Code", 22.0, 110.0, 110.0, (217, 119, 87));
        font.write(pixmap, "~/projets/facturation", 18.0, 110.0, 142.0, (140, 140, 140));
        let prompt_y = if sent { 290.0 } else { 200.0 };
        if sent {
            font.write(pixmap, &format!("> {typed}"), 20.0, 110.0, 200.0, (150, 150, 150));
            let mut dot = Paint::default();
            dot.set_color(Color::from_rgba8(217, 119, 87, 255));
            pixmap.fill_path(&PathBuilder::from_circle(116.0, 254.0, 5.0).unwrap(), &dot, FillRule::Winding, Transform::identity(), None);
            font.write(pixmap, "Je regarde le test de facturation et la migration…", 20.0, 130.0, 240.0, (230, 230, 230));
        }
        let shown = if sent { "" } else { typed };
        let width = font.write(pixmap, &format!("> {shown}"), 20.0, 110.0, prompt_y, (255, 255, 255));
        if caret {
            rect(pixmap, 112.0 + width, prompt_y + 2.0, 10.0, 22.0, (230, 230, 230, 255));
        }
    }

    fn wave_at(seconds: f32, silent_from: f32) -> Vec<(f32, bool)> {
        let frame = (seconds * 10.0) as usize;
        (0..14)
            .map(|i| {
                let index = (frame + i).saturating_sub(13);
                let at = index as f32 / 10.0;
                let level = if at < silent_from { VOICE[index % VOICE.len()] } else { 30 };
                ((level as f32 / 12000.0).sqrt().min(1.0), level > 300)
            })
            .collect()
    }

    #[test]
    #[ignore = "génère les images de la démo : voir docs/readme/demo.sh"]
    fn images_de_la_demo() {
        let out = std::env::var("DEMO_FRAMES").expect("DEMO_FRAMES : dossier de sortie");
        let font = Panel::load().expect("police système introuvable");
        let words: Vec<&str> = SPOKEN.split(' ').collect();
        let (listen, silence, transcribe, hold, typing, sent, end) = (1.0, 5.0, 7.0, 8.2, 11.2, 12.8, 15.0);
        for frame in 0..(end * FPS) as usize {
            let t = frame as f32 / FPS;
            let mut pixmap = Pixmap::new(WIDTH, HEIGHT).unwrap();
            pixmap.fill(Color::from_rgba8(48, 52, 60, 255));
            let typed = if t < typing {
                String::new()
            } else {
                let count = (((t - typing) / (sent - typing - 0.3)).min(1.0) * SPOKEN.chars().count() as f32) as usize;
                SPOKEN.chars().take(count).collect()
            };
            terminal(&mut pixmap, &font, &typed, t >= sent, (t * 2.0) as u32 % 2 == 0);

            let look = if t < listen || t >= typing {
                Look::Hidden
            } else if t < transcribe {
                Look::Listening { plain: false, wave: wave_at(t - listen, silence - listen), attachments: 0 }
            } else if t < hold {
                Look::Transcribing
            } else {
                Look::Holding { remaining: 1.0 - (t - hold) / (typing - hold), spoken: SPOKEN.into(), footer: String::new() }
            };
            if let Some(overlay) = draw(&look, t - transcribe) {
                let place = Transform::from_row(1.6, 0.0, 0.0, 1.6, CURSOR.0 + 20.0, CURSOR.1 + 20.0);
                pixmap.draw_pixmap(0, 0, overlay.as_ref(), &PixmapPaint::default(), place, None);
            }
            if let Look::Holding { remaining, .. } = look {
                let card = font.draw(SPOKEN, "Entrée : envoyer · Échap : annuler", remaining, 760.0).unwrap();
                let (x, y) = ((WIDTH - card.width()) as i32 / 2, (HEIGHT - card.height()) as i32 / 2 - 40);
                pixmap.draw_pixmap(x, y, card.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
            arrow(&mut pixmap, CURSOR);

            let said = ((t - listen) / (silence - listen - 0.4) * words.len() as f32).clamp(0.0, words.len() as f32) as usize;
            let text = match t {
                t if t < listen => "Super + Q pour parler".to_string(),
                t if t < silence => format!("🎙  « {} »", words[..said.max(1)].join(" ")),
                t if t < transcribe => "Je me tais 2 s : le micro se coupe tout seul".to_string(),
                t if t < hold => "Transcription locale avec Whisper".to_string(),
                t if t < typing => "5 s pour relire — Entrée : envoyer · Échap : annuler".to_string(),
                t if t < sent => "Le texte est tapé dans Claude Code…".to_string(),
                _ => "… et envoyé".to_string(),
            };
            caption(&mut pixmap, &font, &text.replace("🎙  ", ""));
            pixmap.save_png(format!("{out}/frame-{frame:04}.png")).unwrap();
        }
    }
}
