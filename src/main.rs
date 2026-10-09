mod capture;
mod overlay;
mod panel;

use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, GrabMode, ModMask};
use x11rb::rust_connection::RustConnection;
use capture::Shot;
use overlay::Look;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

enum State {
    Idle,
    Recording { recorder: Child, generation: u64, submit: bool },
    Transcribing,
    Holding { generation: u64, since: Instant, spoken: String, footer: String },
}

struct Walkie {
    state: State,
    generation: u64,
    notification_id: Option<String>,
    attachments: Attachments,
    wave: Vec<u32>,
    threshold: u32,
    capturing: bool,
}

#[derive(Default)]
struct Attachments {
    selections: Vec<String>,
    shots: Vec<PathBuf>,
}

impl Attachments {
    fn add_selection(&mut self, text: &str) {
        match self.selections.last_mut() {
            Some(last) if same_anchor(last, text) => *last = text.to_string(),
            _ => self.selections.push(text.to_string()),
        }
    }

    fn summary(&self) -> String {
        if self.shots.is_empty() && self.selections.is_empty() {
            return String::new();
        }
        format!("📸 {} · ✂️ {}", self.shots.len(), self.selections.len())
    }
}

fn same_anchor(a: &str, b: &str) -> bool {
    a.starts_with(b) || b.starts_with(a) || a.ends_with(b) || b.ends_with(a)
}

fn footer(submit: bool, attachments: &Attachments) -> String {
    let mut parts = vec![if submit { "Entrée : envoyer" } else { "Entrée : écrire" }.to_string(), "Échap : annuler".to_string()];
    match attachments.shots.len() {
        0 => {}
        1 => parts.push("1 capture".into()),
        n => parts.push(format!("{n} captures")),
    }
    match attachments.selections.len() {
        0 => {}
        1 => parts.push("1 texte surligné".into()),
        n => parts.push(format!("{n} textes surlignés")),
    }
    parts.join(" · ")
}

fn compose(text: &str, attachments: &Attachments, shot_paths: bool) -> String {
    let mut parts = vec![text.to_string()];
    for selection in &attachments.selections {
        parts.push(format!("[texte sélectionné : « {} »]", selection.split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    if shot_paths {
        for shot in &attachments.shots {
            parts.push(format!("[capture d'écran : {}]", shot.display()));
        }
    }
    parts.join(" ")
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

struct Config {
    model: PathBuf,
    language: String,
    hold: Duration,
    silence: Duration,
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
        silence: Duration::from_secs_f32(env::var("WALKIE_SILENCE").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0)),
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

    fn status(&mut self, title: &str, body: &str, timeout_ms: u32) {
        if !overlay::PANEL_READY.load(Ordering::Relaxed) {
            self.notify(title, body, timeout_ms);
        }
    }

    fn close_notification(&mut self) {
        if let Some(id) = self.notification_id.take() {
            run("gdbus", &[
                "call", "--session", "--dest", "org.freedesktop.Notifications", "--object-path", "/org/freedesktop/Notifications",
                "--method", "org.freedesktop.Notifications.CloseNotification", &id,
            ]);
        }
    }

    fn notify_recording(&mut self) {
        let summary = self.attachments.summary();
        let title = if matches!(self.state, State::Recording { submit: false, .. }) { "✏️ Dictée simple…" } else { "🎙️ Écoute…" };
        self.status(title, &summary, 0);
    }

    fn cancel(&mut self) {
        self.state = State::Idle;
        self.status("❌ Annulé", "Le texte reste dans le presse-papiers", 3000);
    }
}

fn transcribe(ctx: &WhisperContext, wav: &Path, language: &str) -> Option<String> {
    let samples: Vec<i16> = match hound::WavReader::open(wav) {
        Ok(reader) => reader.into_samples::<i16>().filter_map(Result::ok).collect(),
        Err(_) => return Some(String::new()),
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

    let mut state = ctx.create_state().ok()?;
    state.full(params, &audio).ok()?;
    let text = state.as_iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" ");
    Some(strip_annotations(&text))
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

    use super::{Attachments, compose, is_terminal, silence_reached, strip_annotations};

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
    fn s_arrete_apres_deux_secondes_de_silence_qui_suivent_la_parole() {
        let mut levels = vec![272, 758, 288, 205, 5214, 3540, 7334, 8097, 5405, 508];
        levels.extend([200; 18]);
        assert!(!silence_reached(&levels, 20, 0));
        levels.push(801);
        assert!(silence_reached(&levels, 20, 0));
    }

    #[test]
    fn ne_s_arrete_pas_sans_avoir_entendu_parler() {
        assert!(!silence_reached(&[250; 60], 20, 0));
        assert!(!silence_reached(&[250, 6000, 250, 250], 20, 0));
    }

    #[test]
    fn le_silence_se_compte_a_partir_de_la_fin_de_la_capture() {
        let mut levels = vec![6000; 5];
        levels.extend([200; 55]);
        assert!(!silence_reached(&levels, 20, 50));
        levels.extend([200; 10]);
        assert!(silence_reached(&levels, 20, 50));
    }

    #[test]
    fn une_capture_avant_de_parler_ne_coupe_pas_le_micro() {
        assert!(!silence_reached(&[200; 80], 20, 30));
    }

    #[test]
    fn un_silence_a_zero_desactive_l_arret_automatique() {
        let mut levels = vec![6000; 5];
        levels.extend([200; 50]);
        assert!(!silence_reached(&levels, 0, 0));
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
    fn une_partie_d_une_selection_precedente_s_ajoute_sans_l_ecraser() {
        let mut attachments = Attachments::default();
        attachments.add_selection("let total = prix * quantite;");
        attachments.add_selection("quantite");
        assert_eq!(attachments.selections, ["let total = prix * quantite;", "quantite"]);
    }

    #[test]
    fn compose_le_prompt_sur_une_seule_ligne() {
        let attachments = Attachments {
            selections: vec!["let x =\n    42;".into()],
            shots: vec![PathBuf::from("/tmp/shot-1.png")],
        };
        assert_eq!(
            compose("Corrige ça", &attachments, true),
            "Corrige ça [texte sélectionné : « let x = 42; »] [capture d'écran : /tmp/shot-1.png]"
        );
        assert_eq!(compose("Corrige ça", &attachments, false), "Corrige ça [texte sélectionné : « let x = 42; »]");
        assert_eq!(compose("Juste du texte", &Attachments::default(), true), "Juste du texte");
    }

    #[test]
    fn reconnait_les_terminaux_par_leur_classe() {
        assert!(is_terminal("ghostty\0com.mitchellh.ghostty\0"));
        assert!(is_terminal("jetbrains-idea\0jetbrains-idea\0"));
        assert!(!is_terminal("brave-browser\0Brave-browser\0"));
        assert!(!is_terminal("slack\0slack\0"));
    }
}

fn toggle(walkie: &Arc<Mutex<Walkie>>, ctx: &Arc<WhisperContext>, cfg: &Arc<Config>, submit: bool) {
    let mut w = walkie.lock().unwrap();
    match std::mem::replace(&mut w.state, State::Transcribing) {
        State::Idle => start_recording(&mut w, walkie, ctx, cfg, submit),
        State::Recording { recorder, submit, .. } => stop_recording(&mut w, recorder, submit, walkie, ctx, cfg),
        State::Transcribing => {}
        State::Holding { .. } => w.cancel(),
    }
}

fn start_recording(w: &mut Walkie, walkie: &Arc<Mutex<Walkie>>, ctx: &Arc<WhisperContext>, cfg: &Arc<Config>, submit: bool) {
    let _ = std::fs::remove_file(&cfg.wav);
    let Ok(recorder) = Command::new("pw-record").args(["--rate", "16000", "--channels", "1", "--format", "s16"]).arg(&cfg.wav).spawn()
    else {
        w.state = State::Idle;
        w.notify("❌ Micro indisponible", "pw-record n'a pas pu démarrer (PipeWire)", 5000);
        return;
    };
    w.generation += 1;
    w.state = State::Recording { recorder, generation: w.generation, submit };
    w.attachments = Attachments::default();
    w.wave.clear();
    w.notify_recording();
    let generation = w.generation;
    if submit {
        let watched = walkie.clone();
        thread::spawn(move || watch_selection(&watched, generation));
    }
    let (walkie, ctx, cfg) = (walkie.clone(), ctx.clone(), cfg.clone());
    thread::spawn(move || watch_level(&walkie, &ctx, &cfg, generation));
}

fn stop_recording(w: &mut Walkie, mut recorder: Child, submit: bool, walkie: &Arc<Mutex<Walkie>>, ctx: &Arc<WhisperContext>, cfg: &Arc<Config>) {
    let window = output("xdotool", &["getactivewindow"]);
    run("kill", &["-INT", &recorder.id().to_string()]);
    let _ = recorder.wait();
    w.status("⏳ Transcription…", "", 0);
    let attachments = std::mem::take(&mut w.attachments);
    let (walkie, ctx, cfg) = (walkie.clone(), ctx.clone(), cfg.clone());
    thread::spawn(move || hold(&walkie, &ctx, &cfg, window, attachments, submit));
}

const WAV_HEADER: u64 = 44;
const FRAME_BYTES: usize = 3200;

fn frame_rms(frame: &[u8]) -> u32 {
    let sum: f64 = frame.chunks_exact(2).map(|b| f64::from(i16::from_le_bytes([b[0], b[1]])).powi(2)).sum();
    (sum / (frame.len() / 2) as f64).sqrt() as u32
}

const MIN_SPEECH_LEVEL: u32 = 1000;
const WAVE_BARS: usize = 14;

fn speech_threshold(levels: &[u32]) -> u32 {
    let mut sorted = levels.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 10).map_or(0, |noise| noise.saturating_mul(5)).max(MIN_SPEECH_LEVEL)
}

fn silence_reached(levels: &[u32], quiet_frames: usize, active_until: usize) -> bool {
    if quiet_frames == 0 || levels.len() <= quiet_frames {
        return false;
    }
    let threshold = speech_threshold(levels);
    let quiet = levels[active_until.min(levels.len())..].iter().rev().take_while(|&&level| level <= threshold).count();
    quiet >= quiet_frames && levels.iter().any(|&level| level > threshold)
}

fn watch_level(walkie: &Arc<Mutex<Walkie>>, ctx: &Arc<WhisperContext>, cfg: &Arc<Config>, generation: u64) {
    let quiet_frames = (cfg.silence.as_secs_f32() * 10.0).round() as usize;
    let (mut offset, mut pending, mut levels, mut active_until) = (WAV_HEADER, Vec::new(), Vec::new(), 0);
    loop {
        thread::sleep(Duration::from_millis(100));
        if let Ok(mut file) = File::open(&cfg.wav)
            && file.seek(SeekFrom::Start(offset)).is_ok()
        {
            offset += file.read_to_end(&mut pending).unwrap_or(0) as u64;
        }
        let complete = pending.len() / FRAME_BYTES * FRAME_BYTES;
        let frames: Vec<u32> = pending.drain(..complete).collect::<Vec<_>>().chunks_exact(FRAME_BYTES).map(frame_rms).collect();

        let mut w = walkie.lock().unwrap();
        if !matches!(w.state, State::Recording { generation: g, .. } if g == generation) {
            return;
        }
        levels.extend(&frames);
        if w.capturing {
            active_until = levels.len();
        }
        w.wave.extend(&frames);
        let excess = w.wave.len().saturating_sub(WAVE_BARS);
        w.wave.drain(..excess);
        w.threshold = speech_threshold(&levels);
        if silence_reached(&levels, quiet_frames, active_until)
            && let State::Recording { recorder, submit, .. } = std::mem::replace(&mut w.state, State::Transcribing)
        {
            stop_recording(&mut w, recorder, submit, walkie, ctx, cfg);
            return;
        }
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

fn look(w: &Walkie, hold: Duration) -> Look {
    if w.capturing {
        return Look::Hidden;
    }
    match w.state {
        State::Idle => Look::Hidden,
        State::Recording { submit, .. } => Look::Listening {
            plain: !submit,
            wave: w.wave.iter().map(|&rms| ((rms as f32 / 12000.0).sqrt().min(1.0), rms > w.threshold)).collect(),
            attachments: w.attachments.shots.len() + w.attachments.selections.len(),
        },
        State::Transcribing => Look::Transcribing,
        State::Holding { since, ref spoken, ref footer, .. } => Look::Holding {
            remaining: 1.0 - since.elapsed().as_secs_f32() / hold.as_secs_f32(),
            spoken: spoken.clone(),
            footer: footer.clone(),
        },
    }
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
    let generation = {
        let mut w = walkie.lock().unwrap();
        match w.state {
            State::Recording { submit: false, .. } => {
                w.notify("📸 Pas de capture en dictée simple", "", 3000);
                return;
            }
            State::Recording { generation, .. } => generation,
            _ => {
                w.notify("📸 Capture possible seulement pendant une dictée", "", 3000);
                return;
            }
        }
    };
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let path = cfg.shots_dir.join(format!("shot-{millis}.png"));
    walkie.lock().unwrap().capturing = true;
    thread::sleep(Duration::from_millis(100));
    let result = capture::interactive(&path);

    let mut w = walkie.lock().unwrap();
    w.capturing = false;
    match result {
        Shot::Saved if matches!(w.state, State::Recording { generation: g, .. } if g == generation) => {
            w.attachments.shots.push(path);
            w.notify_recording();
        }
        Shot::Saved => {
            let _ = std::fs::remove_file(path);
        }
        Shot::Cancelled => {}
        Shot::Failed => w.notify("📸 Capture impossible", "", 3000),
    }
}

fn hold(walkie: &Arc<Mutex<Walkie>>, ctx: &WhisperContext, cfg: &Config, window: Option<String>, attachments: Attachments, submit: bool) {
    let Some(spoken) = transcribe(ctx, &cfg.wav, &cfg.language) else {
        let mut w = walkie.lock().unwrap();
        w.state = State::Idle;
        w.notify("❌ Transcription impossible", "Mémoire GPU pleine ? Voir journalctl --user -u walkie-talkie", 5000);
        return;
    };
    let terminal = window.as_deref().is_none_or(|window| is_terminal(&window_class(window)));
    let text = compose(&spoken, &attachments, terminal);
    let generation = {
        let mut w = walkie.lock().unwrap();
        if spoken.is_empty() {
            w.state = State::Idle;
            w.notify("🤷 Rien entendu", "", 3000);
            return;
        }
        set_clipboard(&text);
        w.generation += 1;
        let footer = footer(submit, &attachments);
        w.state = State::Holding { generation: w.generation, since: Instant::now(), spoken: spoken.clone(), footer };
        if overlay::PANEL_READY.load(Ordering::Relaxed) {
            w.close_notification();
        } else {
            let action = if submit { "📻 Envoi" } else { "✏️ Écriture" };
            let title = format!("{action} dans {} s — Entrée : tout de suite · Échap : annuler", cfg.hold.as_secs_f32());
            w.notify(&title, &text, 0);
        }
        w.generation
    };

    let holding = || matches!(walkie.lock().unwrap().state, State::Holding { generation: g, .. } if g == generation);
    let answer = await_answer(cfg.hold, holding);

    let mut w = walkie.lock().unwrap();
    if !matches!(w.state, State::Holding { generation: g, .. } if g == generation) {
        return;
    }
    if matches!(answer, Answer::Cancel) {
        w.cancel();
        return;
    }
    w.state = State::Idle;
    drop(w);
    if let Some(window) = &window {
        run("xdotool", &["windowactivate", "--sync", window]);
    }
    run("xdotool", &["type", "--clearmodifiers", "--delay", "4", "--", &text]);
    if !terminal && !attachments.shots.is_empty() {
        for shot in &attachments.shots {
            run("xclip", &["-selection", "clipboard", "-t", "image/png", "-i", &shot.to_string_lossy()]);
            thread::sleep(Duration::from_millis(200));
            run("xdotool", &["key", "--clearmodifiers", "ctrl+v"]);
            thread::sleep(Duration::from_millis(800));
        }
        set_clipboard(&text);
    }
    thread::sleep(Duration::from_millis(300));
    if submit {
        run("xdotool", &["key", "--clearmodifiers", "Return"]);
    }
    walkie.lock().unwrap().status(if submit { "✅ Envoyé" } else { "✅ Écrit" }, &text, 3000);
}

fn set_clipboard(text: &str) {
    if let Ok(mut xclip) = Command::new("xclip").args(["-selection", "clipboard"]).stdin(Stdio::piped()).spawn() {
        let _ = xclip.stdin.take().unwrap().write_all(text.as_bytes());
    }
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
        wave: Vec::new(),
        threshold: MIN_SPEECH_LEVEL,
        capturing: false,
    }));
    let (watched, hold) = (walkie.clone(), cfg.hold);
    thread::spawn(move || overlay::run(|| look(&watched.lock().unwrap(), hold)));

    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("socket");
    println!("Prêt ({}) sur {}", cfg.model.display(), path.display());

    for mut conn in listener.incoming().flatten() {
        let mut buf = [0u8; 16];
        let n = conn.read(&mut buf).unwrap_or(0);
        match &buf[..n] {
            b"toggle" => toggle(&walkie, &ctx, &cfg, true),
            b"plain" => toggle(&walkie, &ctx, &cfg, false),
            b"shot" => {
                let (walkie, cfg) = (walkie.clone(), cfg.clone());
                thread::spawn(move || shot(&walkie, &cfg));
            }
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
            match transcribe(&load_model(&cfg), Path::new(&wav), &cfg.language) {
                Some(text) => println!("{text}"),
                None => {
                    eprintln!("transcription impossible");
                    std::process::exit(1);
                }
            }
        }
        Some(command @ ("toggle" | "plain" | "shot")) => send(command),
        None => send("toggle"),
        Some(other) => {
            eprintln!("usage : walkie-talkie [serve|toggle|plain|shot|transcribe <fichier.wav>] (reçu : {other})");
            std::process::exit(2);
        }
    }
}
