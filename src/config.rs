use std::env;
use std::path::PathBuf;
use std::time::Duration;

pub struct Config {
    pub model: PathBuf,
    pub language: String,
    pub hold: Duration,
    pub silence: Duration,
    pub wav: PathBuf,
    pub shots_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env::var("HOME").unwrap()).join(".local/share"))
            .join("walkie-talkie");
        Self {
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
}

fn runtime_dir() -> PathBuf {
    env::var("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|_| env::temp_dir())
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("walkie.sock")
}
