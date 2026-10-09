use std::f32::consts::TAU;
use std::process::Command;

use fontdue::layout::{CoordinateSystem, GlyphPosition, Layout, LayoutSettings, TextStyle};
use fontdue::{Font, FontSettings};
use tiny_skia::{FillRule, PathBuilder, Pixmap, Stroke, Transform};

use super::canvas::{GREEN, arc, color, paint};

const PADDING: f32 = 28.0;
const TEXT_SIZE: f32 = 30.0;
const FOOTER_SIZE: f32 = 18.0;
const FOOTER_ROW: f32 = 28.0;
const RING: f32 = 11.0;
const MIN_WIDTH: f32 = 420.0;

pub struct Panel {
    font: Font,
}

struct Laid {
    glyphs: Vec<GlyphPosition>,
    width: f32,
    height: f32,
}

impl Panel {
    pub fn load() -> Option<Self> {
        let out = Command::new("fc-match").args(["-f", "%{file}", "sans-serif"]).output().ok()?;
        let bytes = std::fs::read(String::from_utf8(out.stdout).ok()?).ok()?;
        Font::from_bytes(bytes, FontSettings::default()).ok().map(|font| Self { font })
    }

    pub fn draw(&self, spoken: &str, footer: &str, remaining: f32, max_width: f32) -> Option<Pixmap> {
        let text = self.layout(spoken, TEXT_SIZE, max_width - 2.0 * PADDING);
        let foot = self.layout(footer, FOOTER_SIZE, max_width - 2.0 * PADDING - 2.0 * RING - 12.0);
        let width = (text.width.max(foot.width + 2.0 * RING + 12.0) + 2.0 * PADDING).max(MIN_WIDTH).ceil();
        let height = (2.0 * PADDING + text.height + 20.0 + FOOTER_ROW).ceil();
        let mut pixmap = Pixmap::new(width as u32, height as u32)?;

        rounded_rect(&mut pixmap, width, height, 18.0);
        self.blit(&mut pixmap, &text, PADDING, PADDING, (255, 255, 255));
        let footer_center = PADDING + text.height + 20.0 + FOOTER_ROW / 2.0;
        arc(&mut pixmap, PADDING + RING, footer_center, RING - 2.0, -TAU / 4.0, -TAU / 4.0 + TAU * remaining, color(GREEN, 230));
        self.blit(&mut pixmap, &foot, PADDING + 2.0 * RING + 12.0, footer_center - foot.height / 2.0, (190, 190, 190));
        Some(pixmap)
    }

    #[cfg(test)]
    pub fn write(&self, pixmap: &mut Pixmap, text: &str, size: f32, x: f32, y: f32, rgb: (u8, u8, u8)) -> f32 {
        let laid = self.layout(text, size, f32::MAX);
        self.blit(pixmap, &laid, x, y, rgb);
        laid.width
    }

    fn layout(&self, text: &str, size: f32, max_width: f32) -> Laid {
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings { max_width: Some(max_width), ..LayoutSettings::default() });
        layout.append(&[&self.font], &TextStyle::new(text, size, 0));
        let glyphs = layout.glyphs().clone();
        let width = glyphs.iter().map(|glyph| glyph.x + glyph.width as f32).fold(0.0, f32::max);
        Laid { glyphs, width, height: layout.height() }
    }

    fn blit(&self, pixmap: &mut Pixmap, laid: &Laid, left: f32, top: f32, (r, g, b): (u8, u8, u8)) {
        let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
        let data = pixmap.data_mut();
        for glyph in &laid.glyphs {
            let (metrics, coverage) = self.font.rasterize_config(glyph.key);
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let alpha = u32::from(coverage[row * metrics.width + col]);
                    let (x, y) = ((left + glyph.x) as i32 + col as i32, (top + glyph.y) as i32 + row as i32);
                    if alpha == 0 || !(0..width).contains(&x) || !(0..height).contains(&y) {
                        continue;
                    }
                    let pixel = &mut data[((y * width + x) * 4) as usize..][..4];
                    for (channel, source) in pixel.iter_mut().zip([r, g, b, 255]) {
                        *channel = ((u32::from(source) * alpha + u32::from(*channel) * (255 - alpha)) / 255) as u8;
                    }
                }
            }
        }
    }
}

fn rounded_rect(pixmap: &mut Pixmap, width: f32, height: f32, radius: f32) {
    let mut builder = PathBuilder::new();
    builder.move_to(radius, 0.0);
    builder.line_to(width - radius, 0.0);
    builder.quad_to(width, 0.0, width, radius);
    builder.line_to(width, height - radius);
    builder.quad_to(width, height, width - radius, height);
    builder.line_to(radius, height);
    builder.quad_to(0.0, height, 0.0, height - radius);
    builder.line_to(0.0, radius);
    builder.quad_to(0.0, 0.0, radius, 0.0);
    builder.close();
    if let Some(path) = builder.finish() {
        pixmap.fill_path(&path, &paint(color((25, 25, 25), 235)), FillRule::Winding, Transform::identity(), None);
        let border = Stroke { width: 1.5, ..Stroke::default() };
        pixmap.stroke_path(&path, &paint(color((255, 255, 255), 60)), &border, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::Panel;

    #[test]
    fn le_texte_long_passe_a_la_ligne_et_agrandit_le_panneau() {
        let Some(panel) = Panel::load() else { return };
        let short = panel.draw("Bonjour", "Entrée : envoyer", 1.0, 900.0).unwrap();
        let long = panel.draw(&"Corrige le test du module de facturation ".repeat(6), "Entrée : envoyer", 1.0, 900.0).unwrap();
        assert!(long.width() <= 900);
        assert!(long.height() > short.height());
    }
}
