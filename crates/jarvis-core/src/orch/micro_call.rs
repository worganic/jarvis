// « Micro-appel » (orchestrator/micro-call.js) : pour une question générale sans action ni information
// personnelle, Claude Code est lancé sans son prompt système complet, sans outil, sans connecteur MCP
// et sans reprise de conversation (quelques centaines de tokens au lieu de plusieurs milliers).

use crate::js;

fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r#"['’\-?!,;:.…"«»]"#).replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

pub fn is_micro_candidate(text: &str, last_ai_at: i64) -> bool {
    let t = fold(text);
    if t.is_empty() || t.chars().count() > 220 {
        return false;
    }
    let general = crate::re!(r"^(?:(?:dis moi |explique moi |explique |sais tu )?(?:c est quoi|qu est ce que|qu est ce qu|qui (?:est|etait|a invente|a ecrit|a peint|sont)|quel(?:le)?s? (?:est|sont|etait|etaient)|combien (?:de|d|mesure|pese|y a t il)|comment (?:dit on|s appelle|fonctionne|marche|faire|fait on)|pourquoi|que (?:veut dire|signifie)|definis|definition|ou (?:se trouve|est situe)|en quelle annee|quand (?:a ete|est ne|est mort|a eu lieu))|raconte(?: moi)? (?:une|un) (?:blague|histoire|anecdote|devinette)|donne(?: moi)? (?:une idee|un conseil|une citation|un synonyme|un exemple)|traduis|conjugue|explique(?: moi)?)\b");
    let needs_full = crate::re!(r"\b(?:je|j|me|m|mon|ma|mes|moi|nous|notre|nos|ouvre|lance|cree|ecris un fichier|modifie|supprime|efface|envoie|installe|telecharge|fichier|dossier|projet|code|script|programme|git|commande|terminal|mail|email|agenda|rendez vous|musique|spotify|ecran|capture|memoire|souviens|retiens|rappelle|site|page|lien|url|recherche sur|cherche sur|internet|web|actualite|aujourd hui|demain|maintenant|en ce moment|meteo|prix|cours de)\b");
    let follow_up = crate::re!(r"^(?:et|mais|alors|donc|du coup|ok et|pourquoi ca|comment ca|c est a dire|precise|continue|encore|la suite|et apres|autre chose)\b|\b(?:ca|cela|celui ci|celle ci|ce dernier|la meme|le meme|il|elle|ils|elles)\s*$");
    // « raconte-moi », « explique-moi » : pronom de politesse, pas une donnée personnelle.
    let core = crate::re!(r"\b(dis|raconte|explique|donne|traduis|conjugue|apprends) moi\b").replace_all(&t, "$1");
    if !general.is_match(&t) || needs_full.is_match(&core) {
        return false;
    }
    if follow_up.is_match(&t) && js::now_ms() - last_ai_at < 3 * 60 * 1000 {
        return false;
    }
    true
}

/// Consigne courte à la place du prompt système complet de Claude Code.
pub fn micro_system_prompt(dialogue: &str) -> String {
    [
        "Tu es Jarvis, l'assistant personnel vocal de l'utilisateur.",
        "Ta réponse est lue à voix haute : réponds en français, en une à trois phrases courtes,",
        "sans markdown, sans liste, sans emoji. Tu n’as aucun outil : réponds de mémoire, et dis-le",
        "simplement si tu ne sais pas ou si l’information peut avoir changé récemment.",
        dialogue,
    ]
    .join(" ")
    .trim()
    .to_string()
}

pub fn micro_args(system_prompt: &str, model: &str) -> Vec<String> {
    [
        "-p", "--verbose", "--output-format", "stream-json", "--include-partial-messages",
        "--tools", "", "--strict-mcp-config", "--mcp-config", r#"{"mcpServers":{}}"#,
        "--permission-mode", "manual", "--no-session-persistence", "--system-prompt", system_prompt,
        "--model", model, "--effort", "low",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
