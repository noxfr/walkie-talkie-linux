mod audio;
mod capture;
mod config;
mod desktop;
mod keys;
mod prompt;
mod screen;
mod ui;
mod walkie;
mod whisper;

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;

use config::Config;
use walkie::Daemon;

fn serve() {
    let cfg = Config::from_env();
    let model = cfg.model.display().to_string();
    let whisper = whisper::load(&cfg.model);
    let daemon = Daemon::new(cfg, whisper);
    let watched = daemon.clone();
    thread::spawn(move || ui::overlay::run(|| watched.look()));

    let path = config::socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("socket");
    println!("Prêt ({model}) sur {}", path.display());

    for mut conn in listener.incoming().flatten() {
        let mut buf = [0u8; 16];
        let n = conn.read(&mut buf).unwrap_or(0);
        match &buf[..n] {
            b"toggle" => daemon.toggle(true),
            b"plain" => daemon.toggle(false),
            b"shot" => {
                let daemon = daemon.clone();
                thread::spawn(move || daemon.shot());
            }
            _ => {}
        }
    }
}

fn send(command: &str) {
    let mut stream = UnixStream::connect(config::socket_path()).expect("le service walkie-talkie ne tourne pas");
    stream.write_all(command.as_bytes()).unwrap();
}

fn main() {
    match env::args().nth(1).as_deref() {
        Some("serve") => serve(),
        Some("transcribe") => {
            let wav = env::args().nth(2).expect("usage : walkie-talkie transcribe <fichier.wav>");
            let cfg = Config::from_env();
            match whisper::transcribe(&whisper::load(&cfg.model), Path::new(&wav), &cfg.language) {
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
