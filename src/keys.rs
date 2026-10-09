use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
use x11rb::rust_connection::RustConnection;

pub enum Answer {
    Send,
    Cancel,
    Timeout,
}

const XK_RETURN: u32 = 0xff0d;
const XK_KP_ENTER: u32 = 0xff8d;
const XK_ESCAPE: u32 = 0xff1b;

fn keycodes(conn: &impl Connection, keysyms: &[u32]) -> Option<Vec<(u8, u32)>> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = conn.get_keyboard_mapping(min, max - min + 1).ok()?.reply().ok()?;
    let per = mapping.keysyms_per_keycode as usize;
    Some(
        keysyms
            .iter()
            .filter_map(|&sym| {
                let index = mapping.keysyms.chunks(per).position(|syms| syms.contains(&sym))?;
                Some((min + index as u8, sym))
            })
            .collect(),
    )
}

fn grab_answer_keys() -> Option<(RustConnection, Vec<(u8, u32)>)> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen].root;
    let keys = keycodes(&conn, &[XK_RETURN, XK_KP_ENTER, XK_ESCAPE])?;
    for &(code, _) in &keys {
        for modifiers in [ModMask::from(0u16), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2] {
            conn.grab_key(true, root, modifiers, code, GrabMode::ASYNC, GrabMode::ASYNC).ok()?;
        }
    }
    conn.flush().ok()?;
    Some((conn, keys))
}

pub fn await_answer(hold: Duration, holding: impl Fn() -> bool) -> Answer {
    let deadline = Instant::now() + hold;
    let grab = grab_answer_keys();
    let mut pressed = None;
    while Instant::now() < deadline && holding() {
        if let Some((conn, keys)) = &grab {
            while let Ok(Some(event)) = conn.poll_for_event() {
                match event {
                    Event::KeyPress(key) => pressed = keys.iter().find(|(code, _)| *code == key.detail).map(|&(_, sym)| sym),
                    Event::KeyRelease(_) if pressed == Some(XK_ESCAPE) => return Answer::Cancel,
                    Event::KeyRelease(_) if pressed.is_some() => return Answer::Send,
                    _ => {}
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    Answer::Timeout
}
