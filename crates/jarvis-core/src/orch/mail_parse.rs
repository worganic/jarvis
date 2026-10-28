// Décodage d'un mail brut (RFC 5322 / MIME) pour la card de lecture (orchestrator/mail-parse.js) :
// en-têtes, texte lisible (partie texte, sinon HTML converti), pièces jointes. Tolérant : un message
// tronqué ou mal formé donne ce qui a pu être lu.

use std::collections::HashMap;

use base64::Engine;
use serde_json::{json, Value};

use crate::js::J;

const CP1252_HIGH: &str = "€\u{81}‚ƒ„…†‡ˆ‰Š‹Œ\u{8d}Ž\u{8f}\u{90}‘’“”•–—˜™š›œ\u{9d}žŸ";

fn latin_family(cs: &str) -> bool {
    matches!(cs, "windows-1252" | "cp1252" | "iso-8859-1" | "iso8859-1" | "latin1" | "l1" | "us-ascii" | "ascii")
}

pub fn decode_charset(bytes: &[u8], charset: &str) -> String {
    let cs = charset.to_lowercase();
    let cs = cs.trim_matches(|c| c == '"' || c == '\'');
    let cs = cs.split('*').next().unwrap_or("utf-8");
    if latin_family(cs) {
        let high: Vec<char> = CP1252_HIGH.chars().collect();
        return bytes.iter().map(|&b| if (0x80..=0x9f).contains(&b) { high[(b - 0x80) as usize] } else { b as char }).collect();
    }
    let label = if cs == "utf8" { "utf-8" } else { cs };
    match encoding_rs::Encoding::for_label(label.as_bytes()) {
        Some(enc) => enc.decode(bytes).0.into_owned(),
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Quoted-printable → octets (les « =\r\n » de fin de ligne sont des coupures douces).
pub fn qp_bytes(text: &str, body: bool) -> Vec<u8> {
    let src = if body { crate::re!(r"=\r?\n").replace_all(text, "").into_owned() } else { text.to_string() };
    let chars: Vec<char> = src.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '=' && i + 2 < chars.len() + 0 && i + 2 <= chars.len() - 1 + 1 {
            let hex: String = chars[i + 1..(i + 3).min(chars.len())].iter().collect();
            if hex.len() == 2 && hex.chars().all(|h| h.is_ascii_hexdigit()) {
                out.push(u8::from_str_radix(&hex, 16).unwrap_or(b'?'));
                i += 3;
                continue;
            }
        }
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        i += 1;
    }
    out
}

/// Mots encodés des en-têtes (« =?UTF-8?Q?R=C3=A9union?= »), voisins regroupés.
pub fn decode_words(value: &str) -> String {
    let re = crate::re!(r"(?i)=\?([^?\s]+)\?([BQ])\?([^?]*)\?=");
    let mut out = String::new();
    let mut last = 0;
    let mut pending: Option<(String, Vec<u8>)> = None;
    let flush = |pending: &mut Option<(String, Vec<u8>)>, out: &mut String| {
        if let Some((cs, bytes)) = pending.take() {
            out.push_str(&decode_charset(&bytes, &cs));
        }
    };
    for m in re.captures_iter(value) {
        let whole = m.get(0).unwrap();
        let between = &value[last..whole.start()];
        if !(pending.is_some() && between.trim().is_empty()) {
            flush(&mut pending, &mut out);
            out.push_str(between);
        }
        let bytes = if m[2].eq_ignore_ascii_case("B") {
            base64::engine::general_purpose::STANDARD.decode(m[3].trim_end_matches('=').to_string() + &"=".repeat((4 - m[3].trim_end_matches('=').len() % 4) % 4)).unwrap_or_default()
        } else {
            qp_bytes(&m[3].replace('_', " "), false)
        };
        let cs = m[1].to_lowercase();
        match &mut pending {
            Some((pcs, chunks)) if *pcs == cs => chunks.extend(bytes),
            _ => {
                flush(&mut pending, &mut out);
                pending = Some((cs, bytes));
            }
        }
        last = whole.end();
    }
    flush(&mut pending, &mut out);
    out.push_str(&value[last..]);
    out
}

fn split_head(raw: &str) -> (&str, &str) {
    match crate::re!(r"\r?\n\r?\n").find(raw) {
        Some(m) => (&raw[..m.start()], &raw[m.end()..]),
        None => (raw, ""),
    }
}

fn parse_headers(head: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    let unfolded = crate::re!(r"\r?\n[ \t]+").replace_all(head, " ");
    for line in crate::re!(r"\r?\n").split(&unfolded) {
        let Some(i) = line.find(':') else { continue };
        if i == 0 {
            continue;
        }
        let name = line[..i].trim().to_lowercase();
        headers.entry(name).or_insert_with(|| line[i + 1..].trim().to_string());
    }
    headers
}

/// « text/html; charset="utf-8"; boundary=xyz » → (type, paramètres).
fn parse_params(value: &str) -> (String, HashMap<String, String>) {
    // Découpe sur les « ; » hors guillemets.
    let mut parts = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in value.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == ';' && !quoted {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    parts.push(cur);
    let mut params: HashMap<String, String> = HashMap::new();
    for p in parts.iter().skip(1) {
        let Some(i) = p.find('=') else { continue };
        if i == 0 {
            continue;
        }
        let mut key = p[..i].trim().to_lowercase();
        let mut val = p[i + 1..].trim().trim_start_matches('"').trim_end_matches('"').to_string();
        if crate::re!(r"\*\d*\*?$").is_match(&key) {
            let encoded = key.ends_with('*');
            key = crate::re!(r"\*\d*\*?$").replace(&key, "").into_owned();
            if encoded {
                let m = crate::re!(r"^([^']*)'[^']*'(.*)$").captures(&val).map(|c| (c[1].to_string(), c[2].to_string()));
                let (cs, enc) = m.clone().unwrap_or_else(|| ("utf-8".into(), val.clone()));
                let bytes = urlencoding::decode_binary(enc.as_bytes()).into_owned();
                val = decode_charset(&bytes, if cs.is_empty() { "utf-8" } else { &cs });
            }
        }
        let e = params.entry(key).or_default();
        e.push_str(&val);
    }
    (parts[0].trim().to_lowercase(), params)
}

fn decode_body(body: &str, encoding: &str, charset: &str) -> String {
    let enc = if encoding.is_empty() { "7bit".to_string() } else { encoding.to_lowercase() };
    let bytes = match enc.as_str() {
        "base64" => {
            let clean = crate::re!(r"[^A-Za-z0-9+/=]").replace_all(body, "");
            let trimmed = clean.trim_end_matches('=');
            let padded = format!("{trimmed}{}", "=".repeat((4 - trimmed.len() % 4) % 4));
            base64::engine::general_purpose::STANDARD.decode(padded).unwrap_or_default()
        }
        "quoted-printable" => qp_bytes(body, true),
        _ => return body.to_string(),
    };
    decode_charset(&bytes, if charset.is_empty() { "utf-8" } else { charset })
}

fn entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "nbsp" => " ",
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "eacute" => "é",
        "egrave" => "è",
        "ecirc" => "ê",
        "agrave" => "à",
        "acirc" => "â",
        "ccedil" => "ç",
        "ocirc" => "ô",
        "ucirc" => "û",
        "ugrave" => "ù",
        "icirc" => "î",
        "iuml" => "ï",
        "euml" => "ë",
        "laquo" => "«",
        "raquo" => "»",
        "rsquo" => "’",
        "lsquo" => "‘",
        "ldquo" => "“",
        "rdquo" => "”",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "euro" => "€",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "middot" => "·",
        "bull" => "•",
        "oelig" => "œ",
        "Eacute" => "É",
        "Agrave" => "À",
        _ => return None,
    })
}

fn strip_blocks(s: &str) -> String {
    // <script>…</script> etc. (pas de référence arrière en Rust : une expression par balise).
    let mut out = s.to_string();
    for tag in ["script", "style", "head", "title", "noscript", "template", "svg"] {
        let re = regex::Regex::new(&format!(r"(?is)<{tag}[^>]*>.*?</{tag}>")).unwrap();
        out = re.replace_all(&out, "").into_owned();
    }
    for tag in ["div", "span", "p", "td"] {
        let re = regex::Regex::new(&format!(r#"(?is)<{tag}\b[^>]*style\s*=\s*["'][^"']*(?:display\s*:\s*none|mso-hide\s*:\s*all|max-height\s*:\s*0|font-size\s*:\s*0|opacity\s*:\s*0)[^"']*["'][^>]*>.*?</{tag}>"#)).unwrap();
        out = re.replace_all(&out, "").into_owned();
    }
    out
}

/// HTML → texte lisible : blocs en lignes, listes en puces, liens réduits à leur texte.
pub fn html_to_text(html: &str) -> String {
    let s = crate::re!(r"(?s)<!--.*?-->").replace_all(html, "");
    let s = strip_blocks(&s);
    let s = crate::re!(r"(?is)<a\b[^>]*>(.*?)</a>").replace_all(&s, |c: &regex::Captures| format!(" {} ", crate::re!(r"<[^>]+>").replace_all(&c[1], "").trim()));
    let s = crate::re!(r"(?i)<br\s*/?>").replace_all(&s, "\n");
    let s = crate::re!(r"(?i)<li[^>]*>").replace_all(&s, "\n• ");
    let s = crate::re!(r"(?i)</(p|div|tr|h[1-6]|ul|ol|table|blockquote|section|article|header|footer)>").replace_all(&s, "\n");
    let s = crate::re!(r"(?i)</li>").replace_all(&s, "");
    let s = crate::re!(r"(?i)<(p|div|tr|h[1-6]|table|blockquote|section|article)[^>]*>").replace_all(&s, "\n");
    let s = crate::re!(r"(?i)</t[dh]>").replace_all(&s, "  ");
    let s = crate::re!(r"<[^>]+>").replace_all(&s, "");
    let s = crate::re!(r"&#(\d+);").replace_all(&s, |c: &regex::Captures| c[1].parse::<u32>().ok().and_then(char::from_u32).map(|ch| ch.to_string()).unwrap_or_default());
    let s = crate::re!(r"(?i)&#x([0-9a-f]+);").replace_all(&s, |c: &regex::Captures| u32::from_str_radix(&c[1], 16).ok().and_then(char::from_u32).map(|ch| ch.to_string()).unwrap_or_default());
    let s = crate::re!(r"(?i)&([a-z]+);").replace_all(&s, |c: &regex::Captures| entity(&c[1]).map(String::from).unwrap_or_else(|| c[0].to_string()));
    tidy(&s)
}

/// Texte du mail : sans adresses web, balises restées, caractères invisibles ni séparateurs.
pub fn clean_text(text: &str) -> String {
    let mut s = text.to_string();
    if crate::re!(r"(?i)</?(?:div|p|br|table|td|span|a|img)\b[^>]*>").find_iter(&s).count() >= 3 {
        s = html_to_text(&s);
    }
    let s = crate::re!("[\u{200b}-\u{200f}\u{2060}\u{feff}\u{ad}\u{34f}]").replace_all(&s, "");
    let s = crate::re!(r"(?i)[ \t]*[(<\[]\s*(?:https?://|www\.|mailto:)[^)>\]]*[)>\]]").replace_all(&s, "");
    let s = crate::re!(r#"(?i)(?:https?://|www\.)[^\s<>()\[\]"']+"#).replace_all(&s, "");
    let s = crate::re!(r"(?i)</?[a-z][a-z0-9]*\b[^>\n]*>").replace_all(&s, "");
    let s = crate::re!(r"(?m)^[ \t]*(?:[-=_*~#.·•|]{3,}|[ \t]*[•·|:,-]+)[ \t]*$").replace_all(&s, "");
    let s = crate::re!(r"[ \t]+([.,])").replace_all(&s, "$1");
    let s = crate::re!(r"\(\s*\)|\[\s*\]").replace_all(&s, "");
    tidy(&s)
}

fn tidy(text: &str) -> String {
    let s = text.replace('\r', "");
    let s = crate::re!("[ \t\u{a0}\u{200b}\u{200c}\u{34f}]+\n").replace_all(&s, "\n");
    let s = crate::re!("[ \t\u{a0}]{2,}").replace_all(&s, " ");
    let s = crate::re!(r"\n[ \t]+").replace_all(&s, "\n");
    let s = crate::re!(r"\n{3,}").replace_all(&s, "\n\n");
    s.trim().to_string()
}

struct Parts {
    text: String,
    html: String,
    attachments: Vec<J>,
}

fn walk(raw: &str, out: &mut Parts, depth: u32) {
    if depth > 8 {
        return;
    }
    let (head, body) = split_head(raw);
    let h = parse_headers(head);
    let (ct, ctp) = parse_params(h.get("content-type").map(|s| s.as_str()).unwrap_or("text/plain; charset=utf-8"));
    let (disp, dp) = parse_params(h.get("content-disposition").map(|s| s.as_str()).unwrap_or(""));
    let name = decode_words(dp.get("filename").or_else(|| ctp.get("name")).map(|s| s.as_str()).unwrap_or(""));
    if ct.starts_with("multipart/") {
        if let Some(b) = ctp.get("boundary") {
            let re = regex::Regex::new(&format!(r"\r?\n?--{}(?:--)?[ \t]*\r?\n?", regex::escape(b))).unwrap();
            for piece in re.split(body).skip(1) {
                if !piece.trim().is_empty() {
                    walk(piece, out, depth + 1);
                }
            }
            return;
        }
    }
    if ct == "message/rfc822" {
        walk(body, out, depth + 1);
        return;
    }
    let encoding = h.get("content-transfer-encoding").cloned().unwrap_or_default();
    if disp == "attachment" || (!name.is_empty() && !ct.starts_with("text/")) {
        let size = if encoding.to_lowercase() == "base64" { (body.chars().filter(|c| !c.is_whitespace()).count() as f64 * 0.75).floor() as usize } else { body.len() };
        out.attachments.push(json!({ "name": if name.is_empty() { "pièce jointe".to_string() } else { name }, "type": ct, "size": size }));
        return;
    }
    let charset = ctp.get("charset").cloned().unwrap_or_default();
    if ct == "text/plain" && out.text.is_empty() {
        out.text = tidy(&decode_body(body, &encoding, &charset));
    } else if ct == "text/html" && out.html.is_empty() {
        out.html = decode_body(body, &encoding, &charset);
    }
}

pub fn parse_mail(raw: &str) -> J {
    let (head, _) = split_head(raw);
    let h = parse_headers(head);
    let mut out = Parts { text: String::new(), html: String::new(), attachments: vec![] };
    walk(raw, &mut out, 0);
    let mut text = clean_text(&out.text);
    if !out.html.is_empty() {
        let head200: String = text.chars().take(200).collect();
        if text.is_empty() || text.chars().count() < 40 || crate::re!(r"(?i)version (?:html|en ligne)|view (?:it )?in (?:your )?browser|afficher (?:ce|le) (?:mail|message)").is_match(&head200) {
            let from_html = clean_text(&html_to_text(&out.html));
            if from_html.chars().count() > text.chars().count() {
                text = from_html;
            }
        }
    }
    let g = |k: &str| h.get(k).cloned().unwrap_or_default();
    let subject = crate::re!(r"\s+").replace_all(&decode_words(&g("subject")), " ").trim().to_string();
    json!({
        "from": decode_words(&g("from")),
        "to": decode_words(&g("to")),
        "cc": decode_words(&g("cc")),
        "subject": if subject.is_empty() { "(sans objet)".to_string() } else { subject },
        "date": g("date"),
        "messageId": g("message-id"),
        "text": if text.is_empty() { "(message sans texte)".to_string() } else { text },
        "hasHtml": !out.html.is_empty(),
        "attachments": out.attachments
    })
}

#[allow(dead_code)]
fn _v(_: Value) {}
