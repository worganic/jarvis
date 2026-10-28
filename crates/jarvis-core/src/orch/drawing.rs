// Dessins dans Paint, sans IA (orchestrator/drawing.js) : « dessine-moi un mouton sur Paint ».
// Jarvis ne pilote ni la souris ni le clavier : chaque sujet connu est une image vectorielle (SVG,
// assets/dessins, exportée de la v3) rendue en PNG (resvg), enregistrée dans workspace/dessins puis
// ouverte dans Paint. Rien n'est effacé ni écrasé.
// parse_draw_request() reçoit un texte déjà « replié » (minuscules, sans accents).

use std::sync::LazyLock;

use serde_json::{json, Value};

use crate::js::{self, J};

pub struct Drawing {
    pub key: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub say: Option<String>,
}

pub static DRAWINGS: LazyLock<Vec<Drawing>> = LazyLock::new(|| {
    let file = crate::paths::root().join("assets").join("dessins").join("catalogue.json");
    let cat = js::read_json(&file).unwrap_or(Value::Array(vec![]));
    js::arr(&cat)
        .iter()
        .map(|d| Drawing { key: js::gs(d, "key"), name: js::gs(d, "name"), aliases: js::str_list(&d["aliases"]), say: d["say"].as_str().map(String::from) })
        .collect()
});

fn alias_key(alias: &str) -> Option<&'static Drawing> {
    DRAWINGS.iter().find(|d| d.key == alias || d.aliases.iter().any(|a| a == alias))
}

pub fn drawing(key: &str) -> Option<&'static Drawing> {
    DRAWINGS.iter().find(|d| d.key == key)
}

/// « un mouton, un chat, … et un smiley » : liste lue à voix haute quand un sujet est inconnu.
pub fn subject_list(limit: usize) -> String {
    let names: Vec<&str> = DRAWINGS.iter().map(|d| d.name.as_str()).take(limit).collect();
    if names.len() > 1 {
        format!("{} et {}", names[..names.len() - 1].join(", "), names[names.len() - 1])
    } else {
        names.join("")
    }
}

const PAINT_NAME: &str = r"(?:ms |microsoft )?(?:paint(?: 3d)?|paints|painte|peint|peinte|pain|pinte)";
const ABILITY: &str = r"(?:(?:tu sais|sais tu|tu saurais|saurais tu|tu es capable de|t es capable de|es tu capable de)\s+)?";
const PRONOUN: &str = r"(?:(?:me|m|nous)\s+)?";

static PAINT_WHERE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(&format!(r"\s(?:sur|dans|avec|via|depuis|en utilisant|a l aide de|grace a)\s+(?:(?:l |le |mon |ton )?(?:application|appli|logiciel|programme|outil)\s+(?:de dessin\s+)?)?(?:le |l )?{PAINT_NAME}(?=\s)")).unwrap()
});
static PAINT_FIRST: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(&format!(r"^(?:ouvre|lance|demarre|utilise|prends|va sur|va dans)\s+(?:l application |l appli |le logiciel )?{PAINT_NAME}\s+(?:et puis |et |puis |pour )?")).unwrap()
});
static DRAW_VERB: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(&format!(r"^{ABILITY}{PRONOUN}(?:dessine[rsz]?|desine[rsz]?|dessinne[rsz]?|redessine[rsz]?|gribouille[rsz]?|crayonne[rsz]?)(?:\s+(?:moi|nous))?(?:\s+(.*))?$")).unwrap()
});
static MAKE_DRAWING: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(&format!(r"^{ABILITY}{PRONOUN}(?:fais|fait|faire|faites|realise[rsz]?|cree[rsz]?|creer)(?:\s+(?:moi|nous))?\s+(?:un |une |le |ce )?(?:petit |joli |beau )?(?:dessin|croquis|gribouillis|gribouillage)(?:\s+(?:de|d|du|des|avec|representant|qui represente))?(?:\s+(.*))?$")).unwrap()
});
static MAKE_IN_PAINT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(&format!(r"^{PRONOUN}(?:fais|fait|faire|cree[rsz]?|creer|mets|mettre|affiche)(?:\s+(?:moi|nous))?\s+(.+)$")).unwrap()
});

fn find_subject(rest: &str) -> (Option<String>, String) {
    let said = rest.trim();
    let s = crate::re!(r"^(?:(?:un|une|le|la|les|l|des|du|de|d|mon|ma|mes|ton|ta|tes|ce|cette|quelques|deux|trois)\s+)").replace(said, "");
    let s = crate::re!(r"^(?:(?:petit|petite|petits|petites|joli|jolie|jolis|jolies|beau|belle|beaux|gros|grosse|grand|grande|mignon|mignonne|super|tres|vrai|magnifique)\s+)+").replace(&s, "");
    let words: Vec<&str> = s.split(' ').filter(|w| !w.is_empty()).collect();
    for n in (1..=words.len().min(4)).rev() {
        if let Some(d) = alias_key(&words[..n].join(" ")) {
            return (Some(d.key.clone()), d.name.clone());
        }
    }
    (None, said.split(' ').take(5).collect::<Vec<_>>().join(" "))
}

/// { key, spoken, paint, random? } ou None. key vaut null pour un sujet inconnu (seulement quand
/// Paint est cité : sinon la demande reste au modèle).
pub fn parse_draw_request(text: &str) -> Option<J> {
    let t = crate::re!(r"[.…]+").replace_all(text, " ");
    let t = format!(" {} ", crate::re!(r"\s+").replace_all(&t, " ").trim());
    let mut paint = false;
    let t = PAINT_WHERE.replace_all(&t, |_: &fancy_regex::Captures| {
        paint = true;
        " "
    });
    let mut t = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
    if PAINT_FIRST.is_match(&t) {
        paint = true;
        t = PAINT_FIRST.replace(&t, "").into_owned();
    }
    let m = DRAW_VERB
        .captures(&t)
        .or_else(|| MAKE_DRAWING.captures(&t))
        .or_else(|| crate::re!(r"^(?:un |le )?(?:petit |joli |beau )?(?:dessin|croquis)\s+(?:de|d|du|des)\s+(.+)$").captures(&t))
        .or_else(|| if paint { MAKE_IN_PAINT.captures(&t) } else { None });
    let rest = match m {
        Some(c) => c.get(1).map(|x| x.as_str().trim().to_string()).unwrap_or_default(),
        None if paint && find_subject(&t).0.is_some() => t.clone(),
        None => return None,
    };
    if crate::re!(r"^(?:quelque chose(?: de (?:joli|beau|sympa|mignon))?|n importe quoi|ce que tu veux|un truc|ce qui te plait|un dessin|un joli dessin|un beau dessin)?$").is_match(&rest) {
        let keys: Vec<&Drawing> = DRAWINGS.iter().filter(|d| d.key != "boite").collect();
        let d = keys[js::rand_below(keys.len())];
        return Some(json!({ "key": d.key, "spoken": d.name, "paint": paint, "random": true }));
    }
    let (key, spoken) = find_subject(&rest);
    if key.is_none() && !paint {
        return None;
    }
    Some(json!({ "key": key, "spoken": spoken, "paint": paint }))
}

/// Sujet donné par un modèle (« mouton », « un mouton ») ou par le routeur (clé).
pub fn resolve_subject(sujet: &str) -> Option<String> {
    let folded = js::fold_lower(sujet);
    let folded = crate::re!(r"['’\-]").replace_all(&folded, " ");
    let folded = crate::re!(r"\s+").replace_all(&folded, " ").trim().to_string();
    if drawing(&folded).is_some() {
        return Some(folded);
    }
    find_subject(&folded).0
}

pub fn open_in_paint(file: &str) {
    crate::proc::detached("cmd.exe", &["/c", "start", "", "mspaint", file]);
}

fn render_png(svg: &str, out: &std::path::Path) -> Result<(), String> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = tree.size().to_int_size();
    let mut pix = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("image vide")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::default(), &mut pix.as_mut());
    pix.save_png(out).map_err(|e| e.to_string())
}

/// Dessine le sujet dans workspace/dessins/<sujet>-<date>.png puis l'ouvre dans Paint.
pub fn draw(sujet: &str, workspace: Option<&std::path::Path>, ouvrir: bool) -> Result<J, String> {
    let Some(key) = resolve_subject(sujet) else {
        return Err(format!("Je ne sais pas encore dessiner « {sujet} ». Sujets connus : {}.", subject_list(usize::MAX)));
    };
    let slug = crate::re!(r"\s+").replace_all(&key, "-").into_owned();
    let svg = std::fs::read_to_string(crate::paths::root().join("assets").join("dessins").join(format!("{slug}.svg"))).map_err(|e| e.to_string())?;
    let dir = workspace.map(|w| w.to_path_buf()).unwrap_or_else(crate::paths::workspace).join("dessins");
    let _ = std::fs::create_dir_all(&dir);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let file = dir.join(format!("{slug}-{stamp}.png"));
    render_png(&svg, &file)?;
    let path = file.to_string_lossy().into_owned();
    if ouvrir {
        open_in_paint(&path);
    }
    let name = drawing(&key).map(|d| d.name.clone()).unwrap_or_default();
    Ok(json!({
        "ok": true, "sujet": key, "nom": name, "chemin": path,
        "message": format!("Dessin ({name}) {} : {path}", if ouvrir { "ouvert dans Paint" } else { "enregistré" })
    }))
}
