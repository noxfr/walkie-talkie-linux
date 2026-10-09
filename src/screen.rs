use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

#[derive(Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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
