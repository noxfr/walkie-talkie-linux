use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn run(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd).args(args).status();
}

#[derive(Default)]
pub struct Notifications {
    id: Option<String>,
}

impl Notifications {
    pub fn notify(&mut self, title: &str, body: &str, timeout_ms: u32) {
        let timeout = timeout_ms.to_string();
        let mut args = vec!["-a", "Walkie Talkie", "-p", "-t", &timeout];
        if let Some(id) = &self.id {
            args.extend(["-r", id.as_str()]);
        }
        args.extend([title, body]);
        let id = output("notify-send", &args);
        self.id = id.or(self.id.take());
    }

    pub fn close(&mut self) {
        if let Some(id) = self.id.take() {
            run("gdbus", &[
                "call", "--session", "--dest", "org.freedesktop.Notifications", "--object-path", "/org/freedesktop/Notifications",
                "--method", "org.freedesktop.Notifications.CloseNotification", &id,
            ]);
        }
    }
}

const TERMINALS: [&str; 12] =
    ["ghostty", "gnome-terminal", "kitty", "alacritty", "xterm", "konsole", "tilix", "terminator", "wezterm", "foot", "urxvt", "jetbrains"];

fn is_terminal(class: &str) -> bool {
    let class = class.to_lowercase();
    TERMINALS.iter().any(|terminal| class.contains(terminal))
}

fn window_class(window: &str) -> String {
    let Ok(id) = window.parse::<u32>() else { return String::new() };
    x11rb::connect(None)
        .ok()
        .and_then(|(conn, _)| conn.get_property(false, id, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256).ok()?.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
        .unwrap_or_default()
}

pub fn is_terminal_window(window: &str) -> bool {
    is_terminal(&window_class(window))
}

pub fn active_window() -> Option<String> {
    output("xdotool", &["getactivewindow"])
}

pub fn activate(window: &str) {
    run("xdotool", &["windowactivate", "--sync", window]);
}

pub fn type_text(text: &str) {
    run("xdotool", &["type", "--clearmodifiers", "--delay", "4", "--", text]);
}

pub fn paste_image(png: &Path) {
    run("xclip", &["-selection", "clipboard", "-t", "image/png", "-i", &png.to_string_lossy()]);
    thread::sleep(Duration::from_millis(200));
    run("xdotool", &["key", "--clearmodifiers", "ctrl+v"]);
    thread::sleep(Duration::from_millis(800));
}

pub fn press_return() {
    run("xdotool", &["key", "--clearmodifiers", "Return"]);
}

pub fn primary_selection() -> String {
    output("xclip", &["-o", "-selection", "primary"]).unwrap_or_default()
}

pub fn set_clipboard(text: &str) {
    if let Ok(mut xclip) = Command::new("xclip").args(["-selection", "clipboard"]).stdin(Stdio::piped()).spawn() {
        let _ = xclip.stdin.take().unwrap().write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::is_terminal;

    #[test]
    fn reconnait_les_terminaux_par_leur_classe() {
        assert!(is_terminal("ghostty\0com.mitchellh.ghostty\0"));
        assert!(is_terminal("jetbrains-idea\0jetbrains-idea\0"));
        assert!(!is_terminal("brave-browser\0Brave-browser\0"));
        assert!(!is_terminal("slack\0slack\0"));
    }
}
