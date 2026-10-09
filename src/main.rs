use std::env;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
use x11rb::rust_connection::RustConnection;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

enum State {
    Idle,
    Recording { recorder: Child },
    Transcribing,
    Holding { generation: u64 },
}

struct Walkie {
    state: State,
    generation: u64,
    notification_id: Option<String>,
}

struct Config {
    model: PathBuf,
    language: String,
    hold: Duration,
    wav: PathBuf,
}

fn runtime_dir() -> PathBuf {
    env::var("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|_| env::temp_dir())
}

fn socket_path() -> PathBuf {
    runtime_dir().join("walkie.sock")
}

fn config() -> Config {
    let data_dir = env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env::var("HOME").unwrap()).join(".local/share"))
        .join("walkie-talkie");
    Config {
        model: env::var("WALKIE_MODEL").map(PathBuf::from).unwrap_or(data_dir.join("ggml-small.bin")),
        language: env::var("WALKIE_LANG").unwrap_or("fr".into()),
        hold: Duration::from_secs_f32(env::var("WALKIE_HOLD").ok().and_then(|s| s.parse().ok()).unwrap_or(5.0)),
        wav: runtime_dir().join("walkie.wav"),
    }
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn run(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd).args(args).status();
}

impl Walkie {
    fn notify(&mut self, title: &str, body: &str, timeout_ms: u32) {
        let timeout = timeout_ms.to_string();
        let mut args = vec!["-a", "Walkie Talkie", "-p", "-t", &timeout];
        if let Some(id) = &self.notification_id {
            args.extend(["-r", id.as_str()]);
        }
        args.extend([title, body]);
        let id = output("notify-send", &args);
        self.notification_id = id.or(self.notification_id.take());
    }

    fn cancel(&mut self) {
        self.state = State::Idle;
        self.notify("❌ Annulé", "Le texte reste dans le presse-papiers", 3000);
    }
}

fn transcribe(ctx: &WhisperContext, cfg: &Config) -> String {
    let samples: Vec<i16> = match hound::WavReader::open(&cfg.wav) {
        Ok(reader) => reader.into_samples::<i16>().filter_map(Result::ok).collect(),
        Err(_) => return String::new(),
    };
    let mut audio = vec![0.0f32; samples.len()];
    whisper_rs::convert_integer_to_float_audio(&samples, &mut audio).unwrap();

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(&cfg.language));
    params.set_n_threads(thread::available_parallelism().map_or(4, |n| n.get() / 2) as i32);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_no_speech_thold(0.6);

    let mut state = ctx.create_state().expect("état whisper");
    if state.full(params, &audio).is_err() {
        return String::new();
    }
    state.as_iter().map(|s| s.to_string().trim().to_string()).collect::<Vec<_>>().join(" ").trim().to_string()
}

fn toggle(walkie: &Arc<Mutex<Walkie>>, ctx: &Arc<WhisperContext>, cfg: &Arc<Config>) {
    let mut w = walkie.lock().unwrap();
    match std::mem::replace(&mut w.state, State::Transcribing) {
        State::Idle => {
            let recorder = Command::new("pw-record")
                .args(["--rate", "16000", "--channels", "1", "--format", "s16"])
                .arg(&cfg.wav)
                .spawn()
                .expect("pw-record introuvable");
            w.state = State::Recording { recorder };
            w.notify("🎙️ Écoute…", "Raccourci pour arrêter", 0);
        }
        State::Recording { mut recorder } => {
            let window = output("xdotool", &["getactivewindow"]);
            run("kill", &["-INT", &recorder.id().to_string()]);
            let _ = recorder.wait();
            w.notify("⏳ Transcription…", "", 0);
            let (walkie, ctx, cfg) = (walkie.clone(), ctx.clone(), cfg.clone());
            thread::spawn(move || hold(&walkie, &ctx, &cfg, window));
        }
        State::Transcribing => {}
        State::Holding { .. } => w.cancel(),
    }
}

enum Answer {
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

fn await_answer(hold: Duration, holding: impl Fn() -> bool) -> Answer {
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

fn hold(walkie: &Arc<Mutex<Walkie>>, ctx: &WhisperContext, cfg: &Config, window: Option<String>) {
    let text = transcribe(ctx, cfg);
    let generation = {
        let mut w = walkie.lock().unwrap();
        if text.is_empty() {
            w.state = State::Idle;
            w.notify("🤷 Rien entendu", "", 3000);
            return;
        }
        if let Ok(mut xclip) = Command::new("xclip").args(["-selection", "clipboard"]).stdin(Stdio::piped()).spawn() {
            let _ = xclip.stdin.take().unwrap().write_all(text.as_bytes());
        }
        w.generation += 1;
        w.state = State::Holding { generation: w.generation };
        let title = format!("📻 Envoi dans {} s — Entrée : envoyer · Échap : annuler", cfg.hold.as_secs_f32());
        w.notify(&title, &text, 0);
        w.generation
    };

    let holding = || matches!(walkie.lock().unwrap().state, State::Holding { generation: g } if g == generation);
    let answer = await_answer(cfg.hold, holding);

    let mut w = walkie.lock().unwrap();
    if !matches!(w.state, State::Holding { generation: g } if g == generation) {
        return;
    }
    if matches!(answer, Answer::Cancel) {
        w.cancel();
        return;
    }
    w.state = State::Idle;
    if let Some(window) = &window {
        run("xdotool", &["windowactivate", "--sync", window]);
    }
    run("xdotool", &["type", "--clearmodifiers", "--delay", "4", "--", &text]);
    thread::sleep(Duration::from_millis(300));
    run("xdotool", &["key", "--clearmodifiers", "Return"]);
    w.notify("✅ Envoyé", &text, 3000);
}

fn serve() {
    let cfg = Arc::new(config());
    let ctx = Arc::new(
        WhisperContext::new_with_params(cfg.model.to_str().unwrap(), WhisperContextParameters::default())
            .unwrap_or_else(|e| panic!("modèle {} illisible : {e}", cfg.model.display())),
    );
    let walkie = Arc::new(Mutex::new(Walkie { state: State::Idle, generation: 0, notification_id: None }));

    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("socket");
    println!("Prêt ({}) sur {}", cfg.model.display(), path.display());

    for mut conn in listener.incoming().flatten() {
        let mut buf = [0u8; 16];
        let n = conn.read(&mut buf).unwrap_or(0);
        if &buf[..n] == b"toggle" {
            toggle(&walkie, &ctx, &cfg);
        }
    }
}

fn main() {
    match env::args().nth(1).as_deref() {
        Some("serve") => serve(),
        Some("toggle") | None => {
            let mut stream = UnixStream::connect(socket_path()).expect("le service walkie-talkie ne tourne pas");
            stream.write_all(b"toggle").unwrap();
        }
        Some(other) => {
            eprintln!("usage : walkie-talkie [serve|toggle] (reçu : {other})");
            std::process::exit(2);
        }
    }
}
