use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

use super::overlay::{Look, draw};
use super::panel::Panel;

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
