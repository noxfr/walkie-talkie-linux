use std::path::Path;
use std::thread;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub fn load(model: &Path) -> WhisperContext {
    WhisperContext::new_with_params(model.to_str().unwrap(), WhisperContextParameters::default())
        .unwrap_or_else(|e| panic!("modèle {} illisible : {e}", model.display()))
}

pub fn transcribe(ctx: &WhisperContext, wav: &Path, language: &str) -> Option<String> {
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
    use super::strip_annotations;

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
}
