mod capture;

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
use x11rb::rust_connection::RustConnection;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

enum State {
    Idle,
    Recording { recorder: Child, generation: u64 },
    Transcribing,
    Holding { generation: u64 },
}

struct Walkie {
    state: State,
    generation: u64,
    notification_id: Option<String>,
    attachments: Attachments,
}

#[derive(Default)]
struct Attachments {
    selections: Vec<String>,
    shots: Vec<PathBuf>,
}

impl Attachments {
    fn add_selection(&mut self, text: &str) {
        match self.selections.last_mut() {
            Some(last) if text.contains(last.as_str()) || last.contains(text) => *last = text.to_string(),
            _ => self.selections.push(text.to_string()),
        }
    }

    fn summary(&self) -> String {
        format!("📸 {} · ✂️ {}", self.shots.len(), self.selections.len())
    }
}

fn compose(text: &str, attachments: &Attachments) -> String {
    let mut parts = vec![text.to_string()];
    for selection in &attachments.selections {
        parts.push(format!("[texte sélectionné : « {} »]", selection.split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    for shot in &attachments.shots {
        parts.push(format!("[capture d'écran : {}]", shot.display()));
    }
    parts.join(" ")
}

struct Config {
    model: PathBuf,
    language: String,
    hold: Duration,
    wav: PathBuf,
    shots_dir: PathBuf,
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
        shots_dir: env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env::var("HOME").unwrap()).join(".cache"))
            .join("walkie-talkie/shots"),
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

    fn notify_recording(&mut self) {
        let summary = self.attachments.summary();
        self.notify("🎙️ Écoute…", &summary, 0);
    }

    fn cancel(&mut self) {
        self.state = State::Idle;
        self.notify("❌ Annulé", "Le texte reste dans le presse-papiers", 3000);
    }
}

fn transcribe(ctx: &WhisperContext, wav: &Path, language: &str) -> String {
    let samples: Vec<i16> = match hound::WavReader::open(wav) {
        Ok(reader) => reader.into_samples::<i16>().filter_map(Result::ok).collect(),
        Err(_) => return String::new(),
    };
    let mut audio = vec![0.0f32; samples.len()];
    whisper_rs::convert_integer_to_float_audio(&samples, &mut audio).unwrap();

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(language));
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
    let text = state.as_iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" ");
    strip_annotations(&text)
}

fn strip_annotations(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(['*', '[', '(']) {
        let closer = match rest.as_bytes()[start] {
            b'[' => ']',
            b'(' => ')',
            _ => '*',
        };
        let Some(length) = rest[start + 1..].find(closer) else { break };
        kept.push_str(&rest[..start]);
        kept.push(' ');
        rest = &rest[start + length + 2..];
    }
    kept.push_str(rest);
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Attachments, compose, strip_annotations};

    #[test]
    fn retire_les_annotations_de_bruit() {
        assert_eq!(strip_annotations(" *Bruit de la porte* "), "");
        assert_eq!(strip_annotations("[Musique] Bonjour (rires) Claude *toux*"), "Bonjour Claude");
    }

    #[test]
    fn garde_le_texte_sans_annotation() {
        assert_eq!(strip_annotations("  Lance les tests  du module. "), "Lance les tests du module.");
        assert_eq!(strip_annotations("Ouvre la parenthèse ( sans la fermer"), "Ouvre la parenthèse ( sans la fermer");
    }

    #[test]
    fn une_selection_qui_grandit_remplace_la_precedente() {
        let mut attachments = Attachments::default();
        attachments.add_selection("fn ma");
        attachments.add_selection("fn main() {");
        attachments.add_selection("autre chose");
        assert_eq!(attachments.selections, ["fn main() {", "autre chose"]);
    }

    #[test]
    fn compose_le_prompt_sur_une_seule_ligne() {
        let attachments = Attachments {
            selections: vec!["let x =\n    42;".into()],
            shots: vec![PathBuf::from("/tmp/shot-1.png")],
        };
        assert_eq!(
            compose("Corrige ça", &attachments),
            "Corrige ça [texte sélectionné : « let x = 42; »] [capture d'écran : /tmp/shot-1.png]"
        );
        assert_eq!(compose("Juste du texte", &Attachments::default()), "Juste du texte");
    }
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
            w.generation += 1;
            w.state = State::Recording { recorder, generation: w.generation };
            w.attachments = Attachments::default();
            w.notify_recording();
            let (walkie, generation) = (walkie.clone(), w.generation);
            thread::spawn(move || watch_selection(&walkie, generation));
        }
        State::Recording { mut recorder, .. } => {
            let window = output("xdotool", &["getactivewindow"]);
            run("kill", &["-INT", &recorder.id().to_string()]);
            let _ = recorder.wait();
            w.notify("⏳ Transcription…", "", 0);
            let attachments = std::mem::take(&mut w.attachments);
            let (walkie, ctx, cfg) = (walkie.clone(), ctx.clone(), cfg.clone());
            thread::spawn(move || hold(&walkie, &ctx, &cfg, window, attachments));
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

fn primary_selection() -> String {
    output("xclip", &["-o", "-selection", "primary"]).unwrap_or_default()
}

fn watch_selection(walkie: &Arc<Mutex<Walkie>>, generation: u64) {
    let mut last = primary_selection();
    loop {
        thread::sleep(Duration::from_millis(300));
        let current = primary_selection();
        let mut w = walkie.lock().unwrap();
        if !matches!(w.state, State::Recording { generation: g, .. } if g == generation) {
            return;
        }
        if !current.is_empty() && current != last {
            w.attachments.add_selection(&current);
            w.notify_recording();
        }
        last = current;
    }
}

fn shot(walkie: &Arc<Mutex<Walkie>>, cfg: &Config) {
    let mut w = walkie.lock().unwrap();
    if !matches!(w.state, State::Recording { .. }) {
        w.notify("📸 Capture possible seulement pendant une dictée", "", 3000);
        return;
    }
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let path = cfg.shots_dir.join(format!("shot-{millis}.png"));
    if capture::screen_under_pointer(&path).is_some() {
        w.attachments.shots.push(path);
        w.notify_recording();
    } else {
        w.notify("📸 Capture impossible", "", 3000);
    }
}

fn hold(walkie: &Arc<Mutex<Walkie>>, ctx: &WhisperContext, cfg: &Config, window: Option<String>, attachments: Attachments) {
    let text = transcribe(ctx, &cfg.wav, &cfg.language);
    let generation = {
        let mut w = walkie.lock().unwrap();
        if text.is_empty() {
            w.state = State::Idle;
            w.notify("🤷 Rien entendu", "", 3000);
            return;
        }
        let text = compose(&text, &attachments);
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

fn load_model(cfg: &Config) -> WhisperContext {
    WhisperContext::new_with_params(cfg.model.to_str().unwrap(), WhisperContextParameters::default())
        .unwrap_or_else(|e| panic!("modèle {} illisible : {e}", cfg.model.display()))
}

fn serve() {
    let cfg = Arc::new(config());
    let ctx = Arc::new(load_model(&cfg));
    let walkie = Arc::new(Mutex::new(Walkie {
        state: State::Idle,
        generation: 0,
        notification_id: None,
        attachments: Attachments::default(),
    }));

    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("socket");
    println!("Prêt ({}) sur {}", cfg.model.display(), path.display());

    for mut conn in listener.incoming().flatten() {
        let mut buf = [0u8; 16];
        let n = conn.read(&mut buf).unwrap_or(0);
        match &buf[..n] {
            b"toggle" => toggle(&walkie, &ctx, &cfg),
            b"shot" => shot(&walkie, &cfg),
            _ => {}
        }
    }
}

fn send(command: &str) {
    let mut stream = UnixStream::connect(socket_path()).expect("le service walkie-talkie ne tourne pas");
    stream.write_all(command.as_bytes()).unwrap();
}

fn main() {
    match env::args().nth(1).as_deref() {
        Some("serve") => serve(),
        Some("transcribe") => {
            let wav = env::args().nth(2).expect("usage : walkie-talkie transcribe <fichier.wav>");
            let cfg = config();
            println!("{}", transcribe(&load_model(&cfg), Path::new(&wav), &cfg.language));
        }
        Some(command @ ("toggle" | "shot")) => send(command),
        None => send("toggle"),
        Some(other) => {
            eprintln!("usage : walkie-talkie [serve|toggle|shot|transcribe <fichier.wav>] (reçu : {other})");
            std::process::exit(2);
        }
    }
}
