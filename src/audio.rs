use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Child, Command};

const WAV_HEADER: u64 = 44;
const FRAME_BYTES: usize = 3200;
pub const MIN_SPEECH_LEVEL: u32 = 300;

pub fn record(wav: &Path) -> io::Result<Child> {
    let _ = std::fs::remove_file(wav);
    Command::new("pw-record").args(["--rate", "16000", "--channels", "1", "--format", "s16"]).arg(wav).spawn()
}

pub fn stop(mut recorder: Child) {
    let _ = Command::new("kill").args(["-INT", &recorder.id().to_string()]).status();
    let _ = recorder.wait();
}

pub struct LevelMeter {
    offset: u64,
    pending: Vec<u8>,
}

impl Default for LevelMeter {
    fn default() -> Self {
        Self { offset: WAV_HEADER, pending: Vec::new() }
    }
}

impl LevelMeter {
    pub fn read(&mut self, wav: &Path) -> Vec<u32> {
        if let Ok(mut file) = File::open(wav)
            && file.seek(SeekFrom::Start(self.offset)).is_ok()
        {
            self.offset += file.read_to_end(&mut self.pending).unwrap_or(0) as u64;
        }
        let complete = self.pending.len() / FRAME_BYTES * FRAME_BYTES;
        self.pending.drain(..complete).collect::<Vec<_>>().chunks_exact(FRAME_BYTES).map(frame_rms).collect()
    }
}

fn frame_rms(frame: &[u8]) -> u32 {
    let sum: f64 = frame.chunks_exact(2).map(|b| f64::from(i16::from_le_bytes([b[0], b[1]])).powi(2)).sum();
    (sum / (frame.len() / 2) as f64).sqrt() as u32
}

pub fn speech_threshold(levels: &[u32]) -> u32 {
    let mut sorted = levels.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 10).map_or(0, |noise| noise.saturating_mul(5)).max(MIN_SPEECH_LEVEL)
}

pub fn silence_reached(levels: &[u32], quiet_frames: usize, active_until: usize) -> bool {
    if quiet_frames == 0 || levels.len() <= quiet_frames {
        return false;
    }
    let threshold = speech_threshold(levels);
    let quiet = levels[active_until.min(levels.len())..].iter().rev().take_while(|&&level| level <= threshold).count();
    quiet >= quiet_frames && levels.iter().any(|&level| level > threshold)
}

#[cfg(test)]
mod tests {
    use super::{silence_reached, speech_threshold};

    #[test]
    fn s_arrete_apres_deux_secondes_de_silence_qui_suivent_la_parole() {
        let mut levels = vec![272, 758, 288, 205, 5214, 3540, 7334, 8097, 5405, 508];
        levels.extend([200; 18]);
        assert!(!silence_reached(&levels, 20, 0));
        levels.push(801);
        assert!(silence_reached(&levels, 20, 0));
    }

    #[test]
    fn une_voix_faible_garde_une_marge_avant_la_coupure() {
        let speech = [0, 7, 28, 25, 677, 838, 1011, 917, 620, 798, 597, 379, 422, 338, 986, 380, 594, 1127, 1118, 818, 482, 431, 779, 910, 1397, 1512, 1054, 371, 489, 783, 1835, 1422, 1272, 1151, 92, 617, 1266, 969, 1074, 683, 431, 400, 1222, 1036, 266, 1064, 975, 609, 42, 45, 61, 74, 57, 88, 885, 1504, 825, 1153, 1062, 629, 1583, 349, 219, 340, 639, 935, 963, 1172, 502, 42, 25, 137, 993, 1258, 1146, 362, 896, 945, 615, 77, 674, 585, 59, 37, 41, 61, 67, 54, 41, 31, 283, 1388, 1467, 978, 332, 666, 745, 222, 570, 887, 500, 151, 195, 1282, 2013, 1629, 1284, 953, 86, 23, 23, 298, 1245, 1649, 1852, 1778, 1770, 1848, 1955, 1351, 354, 118, 421, 1261, 1339, 897, 395, 42, 29, 27, 33, 29, 35, 210, 1536, 898, 470, 980, 590, 132, 403, 142, 27, 29];
        let threshold = speech_threshold(&speech);
        let longest_pause = speech.split(|&level| level > threshold).map(<[u32]>::len).max().unwrap();
        assert!(longest_pause < 10, "pause de {:.1} s en pleine phrase", longest_pause as f32 / 10.0);
        let mut finished = speech.to_vec();
        finished.extend([25; 20]);
        assert!(silence_reached(&finished, 20, 0));
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
}
