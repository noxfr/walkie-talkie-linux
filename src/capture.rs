use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::process::Command;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};
use x11rb::rust_connection::RustConnection;

pub enum Shot {
    Saved,
    Cancelled,
    Failed,
}

#[derive(Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

struct Frame {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

pub fn interactive(path: &Path) -> Shot {
    let Some((frame, monitor)) = freeze() else { return Shot::Failed };
    let rect = match Command::new("slop").args(["-f", "%x %y %w %h"]).output() {
        Err(_) => monitor,
        Ok(out) if out.status.success() => match parse_slop(&String::from_utf8_lossy(&out.stdout)) {
            Some(rect) => rect,
            None => return Shot::Cancelled,
        },
        Ok(_) => return Shot::Cancelled,
    };
    match save(&crop(&frame, &rect), path) {
        Some(()) => Shot::Saved,
        None => Shot::Failed,
    }
}

pub fn monitor_under_pointer(conn: &RustConnection, screen: usize) -> Option<Rect> {
    let root = &conn.setup().roots[screen];
    let pointer = conn.query_pointer(root.root).ok()?.reply().ok()?;
    let monitor = conn
        .randr_get_monitors(root.root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| {
            reply.monitors.into_iter().find(|m| {
                (m.x..m.x + m.width as i16).contains(&pointer.root_x) && (m.y..m.y + m.height as i16).contains(&pointer.root_y)
            })
        })
        .map(|m| Rect { x: m.x.into(), y: m.y.into(), width: m.width.into(), height: m.height.into() })
        .unwrap_or(Rect { x: 0, y: 0, width: root.width_in_pixels.into(), height: root.height_in_pixels.into() });
    Some(monitor)
}

fn freeze() -> Option<(Frame, Rect)> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = &conn.setup().roots[screen];
    let (width, height) = (root.width_in_pixels, root.height_in_pixels);
    let monitor = monitor_under_pointer(&conn, screen)?;

    let image = conn.get_image(ImageFormat::Z_PIXMAP, root.root, 0, 0, width, height, u32::MAX).ok()?.reply().ok()?;
    let rgb = image.data.chunks_exact(4).flat_map(|bgrx| [bgrx[2], bgrx[1], bgrx[0]]).collect();
    Some((Frame { width: width.into(), height: height.into(), rgb }, monitor))
}

fn parse_slop(output: &str) -> Option<Rect> {
    let numbers: Vec<i64> = output.split_whitespace().map(|n| n.parse().ok()).collect::<Option<_>>()?;
    let [x, y, width, height] = numbers[..] else { return None };
    (width > 0 && height > 0).then(|| Rect { x: x as i32, y: y as i32, width: width as u32, height: height as u32 })
}

fn crop(frame: &Frame, rect: &Rect) -> Frame {
    let x0 = rect.x.clamp(0, frame.width as i32) as u32;
    let y0 = rect.y.clamp(0, frame.height as i32) as u32;
    let x1 = (rect.x + rect.width as i32).clamp(0, frame.width as i32) as u32;
    let y1 = (rect.y + rect.height as i32).clamp(0, frame.height as i32) as u32;
    let rgb = (y0..y1)
        .flat_map(|y| {
            let start = ((y * frame.width + x0) * 3) as usize;
            frame.rgb[start..start + ((x1 - x0) * 3) as usize].iter().copied()
        })
        .collect();
    Frame { width: x1 - x0, height: y1 - y0, rgb }
}

fn save(frame: &Frame, path: &Path) -> Option<()> {
    std::fs::create_dir_all(path.parent()?).ok()?;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path).ok()?), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&frame.rgb).ok()
}

#[cfg(test)]
mod tests {
    use super::{Frame, Rect, crop, parse_slop};

    #[test]
    fn lit_la_zone_choisie_par_slop() {
        assert_eq!(parse_slop("10 20 300 200\n"), Some(Rect { x: 10, y: 20, width: 300, height: 200 }));
        assert_eq!(parse_slop("0 0 0 0"), None);
        assert_eq!(parse_slop(""), None);
    }

    #[test]
    fn decoupe_la_zone_en_restant_dans_l_image() {
        let frame = Frame { width: 3, height: 2, rgb: (0..18).collect() };
        let cropped = crop(&frame, &Rect { x: 1, y: 1, width: 5, height: 5 });
        assert_eq!((cropped.width, cropped.height), (2, 1));
        assert_eq!(cropped.rgb, [12, 13, 14, 15, 16, 17]);
    }
}
