use std::process::Child;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use whisper_rs::WhisperContext;

use crate::audio::{self, LevelMeter};
use crate::capture::{self, Shot};
use crate::config::Config;
use crate::desktop::{self, Notifications};
use crate::keys::{self, Answer};
use crate::prompt::{self, Attachments};
use crate::ui::overlay::{self, Look};
use crate::whisper;

const WAVE_BARS: usize = 14;

enum State {
    Idle,
    Recording { recorder: Child, generation: u64, submit: bool },
    Transcribing,
    Holding { generation: u64, since: Instant, spoken: String, footer: String },
}

struct Walkie {
    state: State,
    generation: u64,
    notifications: Notifications,
    attachments: Attachments,
    wave: Vec<u32>,
    threshold: u32,
    capturing: bool,
}

impl Walkie {
    fn recording(&self, generation: u64) -> bool {
        matches!(self.state, State::Recording { generation: g, .. } if g == generation)
    }

    fn holding(&self, generation: u64) -> bool {
        matches!(self.state, State::Holding { generation: g, .. } if g == generation)
    }

    fn notify(&mut self, title: &str, body: &str, timeout_ms: u32) {
        self.notifications.notify(title, body, timeout_ms);
    }

    fn status(&mut self, title: &str, body: &str, timeout_ms: u32) {
        if !overlay::PANEL_READY.load(Ordering::Relaxed) {
            self.notify(title, body, timeout_ms);
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

pub struct Daemon {
    walkie: Mutex<Walkie>,
    whisper: WhisperContext,
    cfg: Config,
}

impl Daemon {
    pub fn new(cfg: Config, whisper: WhisperContext) -> Arc<Self> {
        let walkie = Walkie {
            state: State::Idle,
            generation: 0,
            notifications: Notifications::default(),
            attachments: Attachments::default(),
            wave: Vec::new(),
            threshold: audio::MIN_SPEECH_LEVEL,
            capturing: false,
        };
        Arc::new(Self { walkie: Mutex::new(walkie), whisper, cfg })
    }

    fn lock(&self) -> MutexGuard<'_, Walkie> {
        self.walkie.lock().unwrap()
    }

    pub fn toggle(self: &Arc<Self>, submit: bool) {
        let mut w = self.lock();
        match std::mem::replace(&mut w.state, State::Transcribing) {
            State::Idle => self.start_recording(&mut w, submit),
            State::Recording { recorder, submit, .. } => self.stop_recording(&mut w, recorder, submit),
            State::Transcribing => {}
            State::Holding { .. } => w.cancel(),
        }
    }

    fn start_recording(self: &Arc<Self>, w: &mut Walkie, submit: bool) {
        let Ok(recorder) = audio::record(&self.cfg.wav) else {
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
            let daemon = self.clone();
            thread::spawn(move || daemon.watch_selection(generation));
        }
        let daemon = self.clone();
        thread::spawn(move || daemon.watch_level(generation));
    }

    fn stop_recording(self: &Arc<Self>, w: &mut Walkie, recorder: Child, submit: bool) {
        let window = desktop::active_window();
        audio::stop(recorder);
        w.status("⏳ Transcription…", "", 0);
        let attachments = std::mem::take(&mut w.attachments);
        let daemon = self.clone();
        thread::spawn(move || daemon.hold(window, attachments, submit));
    }

    fn watch_level(self: &Arc<Self>, generation: u64) {
        let quiet_frames = (self.cfg.silence.as_secs_f32() * 10.0).round() as usize;
        let (mut meter, mut levels, mut active_until) = (LevelMeter::default(), Vec::new(), 0);
        loop {
            thread::sleep(Duration::from_millis(100));
            let frames = meter.read(&self.cfg.wav);

            let mut w = self.lock();
            if !w.recording(generation) {
                return;
            }
            levels.extend(&frames);
            if w.capturing {
                active_until = levels.len();
            }
            w.wave.extend(&frames);
            let excess = w.wave.len().saturating_sub(WAVE_BARS);
            w.wave.drain(..excess);
            w.threshold = audio::speech_threshold(&levels);
            if audio::silence_reached(&levels, quiet_frames, active_until)
                && let State::Recording { recorder, submit, .. } = std::mem::replace(&mut w.state, State::Transcribing)
            {
                self.stop_recording(&mut w, recorder, submit);
                return;
            }
        }
    }

    fn watch_selection(&self, generation: u64) {
        let mut last = desktop::primary_selection();
        loop {
            thread::sleep(Duration::from_millis(300));
            let current = desktop::primary_selection();
            let mut w = self.lock();
            if !w.recording(generation) {
                return;
            }
            if !current.is_empty() && current != last {
                w.attachments.add_selection(&current);
                w.notify_recording();
            }
            last = current;
        }
    }

    pub fn look(&self) -> Look {
        let w = self.lock();
        if w.capturing {
            return Look::Hidden;
        }
        match w.state {
            State::Idle => Look::Hidden,
            State::Recording { submit, .. } => Look::Listening {
                plain: !submit,
                wave: w.wave.iter().map(|&rms| ((rms as f32 / 12000.0).sqrt().min(1.0), rms > w.threshold)).collect(),
                attachments: w.attachments.count(),
            },
            State::Transcribing => Look::Transcribing,
            State::Holding { since, ref spoken, ref footer, .. } => Look::Holding {
                remaining: 1.0 - since.elapsed().as_secs_f32() / self.cfg.hold.as_secs_f32(),
                spoken: spoken.clone(),
                footer: footer.clone(),
            },
        }
    }

    pub fn shot(&self) {
        let generation = {
            let mut w = self.lock();
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
        let path = self.cfg.shots_dir.join(format!("shot-{millis}.png"));
        self.lock().capturing = true;
        thread::sleep(Duration::from_millis(100));
        let result = capture::interactive(&path);

        let mut w = self.lock();
        w.capturing = false;
        match result {
            Shot::Saved if w.recording(generation) => {
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

    fn hold(&self, window: Option<String>, attachments: Attachments, submit: bool) {
        let Some(spoken) = whisper::transcribe(&self.whisper, &self.cfg.wav, &self.cfg.language) else {
            let mut w = self.lock();
            w.state = State::Idle;
            w.notify("❌ Transcription impossible", "Mémoire GPU pleine ? Voir journalctl --user -u walkie-talkie", 5000);
            return;
        };
        let terminal = window.as_deref().is_none_or(desktop::is_terminal_window);
        let text = prompt::compose(&spoken, &attachments, terminal);
        let generation = {
            let mut w = self.lock();
            if spoken.is_empty() {
                w.state = State::Idle;
                w.notify("🤷 Rien entendu", "", 3000);
                return;
            }
            desktop::set_clipboard(&text);
            w.generation += 1;
            let footer = prompt::footer(submit, &attachments);
            w.state = State::Holding { generation: w.generation, since: Instant::now(), spoken: spoken.clone(), footer };
            if overlay::PANEL_READY.load(Ordering::Relaxed) {
                w.notifications.close();
            } else {
                let action = if submit { "📻 Envoi" } else { "✏️ Écriture" };
                let title = format!("{action} dans {} s — Entrée : tout de suite · Échap : annuler", self.cfg.hold.as_secs_f32());
                w.notify(&title, &text, 0);
            }
            w.generation
        };

        let answer = keys::await_answer(self.cfg.hold, || self.lock().holding(generation));

        let mut w = self.lock();
        if !w.holding(generation) {
            return;
        }
        if matches!(answer, Answer::Cancel) {
            w.cancel();
            return;
        }
        w.state = State::Idle;
        drop(w);
        if let Some(window) = &window {
            desktop::activate(window);
        }
        desktop::type_text(&text);
        if !terminal && !attachments.shots.is_empty() {
            for shot in &attachments.shots {
                desktop::paste_image(shot);
            }
            desktop::set_clipboard(&text);
        }
        thread::sleep(Duration::from_millis(300));
        if submit {
            desktop::press_return();
        }
        self.lock().status(if submit { "✅ Envoyé" } else { "✅ Écrit" }, &text, 3000);
    }
}
