use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};

pub fn screen_under_pointer(path: &Path) -> Option<()> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = &conn.setup().roots[screen];
    let pointer = conn.query_pointer(root.root).ok()?.reply().ok()?;
    let (x, y, width, height) = conn
        .randr_get_monitors(root.root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| {
            reply.monitors.into_iter().find(|m| {
                (m.x..m.x + m.width as i16).contains(&pointer.root_x) && (m.y..m.y + m.height as i16).contains(&pointer.root_y)
            })
        })
        .map(|m| (m.x, m.y, m.width, m.height))
        .unwrap_or((0, 0, root.width_in_pixels, root.height_in_pixels));

    let image = conn.get_image(ImageFormat::Z_PIXMAP, root.root, x, y, width, height, u32::MAX).ok()?.reply().ok()?;
    let rgb: Vec<u8> = image.data.chunks_exact(4).flat_map(|bgrx| [bgrx[2], bgrx[1], bgrx[0]]).collect();

    std::fs::create_dir_all(path.parent()?).ok()?;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path).ok()?), width.into(), height.into());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&rgb).ok()
}
