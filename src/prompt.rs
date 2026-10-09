use std::path::PathBuf;

#[derive(Default)]
pub struct Attachments {
    pub selections: Vec<String>,
    pub shots: Vec<PathBuf>,
}

impl Attachments {
    pub fn add_selection(&mut self, text: &str) {
        match self.selections.last_mut() {
            Some(last) if same_anchor(last, text) => *last = text.to_string(),
            _ => self.selections.push(text.to_string()),
        }
    }

    pub fn count(&self) -> usize {
        self.shots.len() + self.selections.len()
    }

    pub fn summary(&self) -> String {
        if self.shots.is_empty() && self.selections.is_empty() {
            return String::new();
        }
        format!("📸 {} · ✂️ {}", self.shots.len(), self.selections.len())
    }
}

fn same_anchor(a: &str, b: &str) -> bool {
    a.starts_with(b) || b.starts_with(a) || a.ends_with(b) || b.ends_with(a)
}

pub fn footer(submit: bool, attachments: &Attachments) -> String {
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

pub fn compose(text: &str, attachments: &Attachments, shot_paths: bool) -> String {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Attachments, compose};

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
}
