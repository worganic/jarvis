// Traduction à la voix : « traduis où est la gare en espagnol », « comment dit-on merci en japonais ». Le texte est traduit
// par l'IA (Claude Haiku sans outil, comme la mise en forme des listes), affiché, puis dit avec une voix edge-tts de la
// langue demandée (la voix des réglages n'est pas modifiée).

use serde_json::json;

use super::Orchestrator;
use crate::js::{self, J};

/// (mot français replié, nom anglais pour l'IA, libellé affiché, voix edge-tts)
pub const LANGUAGES: &[(&str, &str, &str, &str)] = &[
    ("anglais", "English", "anglais", "en-GB-SoniaNeural"),
    ("americain", "American English", "américain", "en-US-JennyNeural"),
    ("espagnol", "Spanish", "espagnol", "es-ES-ElviraNeural"),
    ("allemand", "German", "allemand", "de-DE-KatjaNeural"),
    ("italien", "Italian", "italien", "it-IT-ElsaNeural"),
    ("portugais", "Portuguese", "portugais", "pt-PT-RaquelNeural"),
    ("neerlandais", "Dutch", "néerlandais", "nl-NL-ColetteNeural"),
    ("polonais", "Polish", "polonais", "pl-PL-ZofiaNeural"),
    ("russe", "Russian", "russe", "ru-RU-SvetlanaNeural"),
    ("japonais", "Japanese", "japonais", "ja-JP-NanamiNeural"),
    ("chinois", "Mandarin Chinese", "chinois", "zh-CN-XiaoxiaoNeural"),
    ("arabe", "Arabic", "arabe", "ar-SA-ZariyahNeural"),
    ("turc", "Turkish", "turc", "tr-TR-EmelNeural"),
    ("coreen", "Korean", "coréen", "ko-KR-SunHiNeural"),
];

const SYSTEM_PROMPT: &str = "Tu es un traducteur. Traduis fidèlement le texte de l'utilisateur dans la langue demandée. \
Réponds uniquement par la traduction, sans guillemets, sans explication, sans transcription phonétique ni texte dans une autre langue.";

fn language(key: &str) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    LANGUAGES.iter().find(|l| l.0 == key)
}

/// Reconnaît une demande de traduction dans une phrase déjà repliée : (texte à traduire, clé de la langue).
pub fn parse_request(t: &str) -> Option<(String, &'static str)> {
    let names = LANGUAGES.iter().map(|l| l.0).collect::<Vec<_>>().join("|");
    let patterns = [
        format!(r"^(?:traduis|traduit)(?: moi)?(?: en)? (.+?) en ({names})$"),
        format!(r"^(?:traduis|traduit)(?: moi)? en ({names})(?: que| ceci| ca)? (.+)$"),
        format!(r"^comment (?:est-ce qu )?(?:on )?dit(?:[- ]on)? (.+?) en ({names})$"),
        format!(r"^(?:dis|ecris) (?:moi )?(.+?) en ({names})$"),
    ];
    for (i, p) in patterns.iter().enumerate() {
        if let Some(m) = regex::Regex::new(p).ok()?.captures(t.trim()) {
            // Dans le 2ᵉ motif, la langue précède le texte.
            let (text, lang) = if i == 1 { (m[2].to_string(), m[1].to_string()) } else { (m[1].to_string(), m[2].to_string()) };
            let text = text.trim().trim_matches(|c| c == '"' || c == '«' || c == '»').trim().to_string();
            let key = LANGUAGES.iter().find(|l| l.0 == lang)?.0;
            if !text.is_empty() {
                return Some((text, key));
            }
        }
    }
    None
}

impl Orchestrator {
    /// Traduit `text` dans la langue `key` : (traduction, libellé de la langue, voix).
    pub async fn translate_text(&self, text: &str, key: &str) -> Result<(String, &'static str, &'static str), String> {
        let (_, english, label, voice) = *language(key).ok_or("Langue inconnue.")?;
        if js::env("JARVIS_TEST").is_some() {
            return Err("traduction indisponible (test)".into());
        }
        let translated = self.quick_haiku(SYSTEM_PROMPT, &format!("Langue cible : {english}\n\nTexte :\n{}", js::slice_chars(text, 2000)), 60).await?;
        Ok((js::slice_chars(&translated, 1200), label, voice))
    }

    /// Claude Haiku sans outil, sans connecteur ni session : une consigne système, un texte, une réponse texte.
    /// Sert à la traduction et à la mémoire automatique.
    pub async fn quick_haiku(&self, system: &str, input: &str, timeout_s: u64) -> Result<String, String> {
        let claude = self.claude_path().ok_or("Claude Code est introuvable.")?;
        use tokio::io::AsyncWriteExt;
        let mut cmd = tokio::process::Command::new(&claude);
        cmd.args([
            "-p", "--output-format", "text", "--tools", "", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}",
            "--permission-mode", "manual", "--no-session-persistence", "--system-prompt", system, "--model", "haiku",
        ])
        .current_dir(crate::paths::workspace())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes()).await;
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(timeout_s), child.wait_with_output())
            .await
            .map_err(|_| "délai dépassé".to_string())?
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !text.is_empty() {
            Ok(text)
        } else {
            Err("réponse vide".into())
        }
    }

    /// Phrase vocale de traduction : annonce en français, puis la traduction dite avec la voix de la langue.
    pub async fn translate_intent_run(&self, text: &str, key: &str) -> J {
        match self.translate_text(text, key).await {
            Ok((translated, label, voice)) => {
                self.emit(json!({ "type": "say", "text": format!("En {label} :") }));
                self.emit(json!({ "type": "say", "text": translated, "voice": voice }));
                json!({ "display": format!("🌐 {label} : {translated}") })
            }
            Err(e) => json!({ "say": format!("Je n'ai pas pu traduire : {e}."), "display": "Traduction impossible" }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demandes_de_traduction() {
        assert_eq!(parse_request("traduis ou est la gare en espagnol"), Some(("ou est la gare".into(), "espagnol")));
        assert_eq!(parse_request("traduis moi bonjour en anglais"), Some(("bonjour".into(), "anglais")));
        assert_eq!(parse_request("traduis en allemand bonne nuit"), Some(("bonne nuit".into(), "allemand")));
        assert_eq!(parse_request("comment dit on merci en japonais"), Some(("merci".into(), "japonais")));
        assert_eq!(parse_request("dis bonne journee en italien"), Some(("bonne journee".into(), "italien")));
        assert_eq!(parse_request("traduis ce texte en klingon"), None);
        assert_eq!(parse_request("quelle heure est il"), None);
    }

    #[test]
    fn voix_de_chaque_langue_valide() {
        for (_, _, _, voice) in LANGUAGES {
            assert!(crate::re!(r"^[a-z]{2,3}-[A-Z]{2,4}-[A-Za-z]{3,30}Neural$").is_match(voice), "{voice}");
        }
    }
}
