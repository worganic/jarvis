// Voix Microsoft Edge (« edge-tts ») : client du service de lecture à voix haute d'Edge, réécrit en
// Rust à partir du paquet Python edge_tts utilisé par la v3 (même URL, mêmes en-têtes, même jeton
// Sec-MS-GEC, même SSML, MP3 24 kHz mono décodé ici).
//
// Le service change parfois son jeton d'accès (octobre 2026 : jeton « version 2 », calcul propre à Microsoft).
// Quand il refuse ce client (403 après correction d'horloge), Jarvis passe par le paquet Python edge-tts,
// tenu à jour par sa communauté (« pip install -U edge-tts ») : mêmes voix, même MP3, décodé ici.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

const BASE_URL: &str = "speech.platform.bing.com/consumer/speech/synthesize/readaloud";
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_FULL_VERSION: &str = "143.0.3650.75";
const WIN_EPOCH: i64 = 11_644_473_600;

/// Correction d'horloge (secondes) apprise d'un refus du service.
static CLOCK_SKEW: AtomicI64 = AtomicI64::new(0);
/// Client Rust refusé par le service : les phrases suivantes passent directement par le paquet Python.
static USE_BRIDGE: AtomicBool = AtomicBool::new(false);

fn hex_id() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::getrandom(&mut b);
    hex::encode(b)
}

/// Jeton Sec-MS-GEC : heure Windows arrondie à 5 minutes + jeton client, SHA-256 en majuscules.
fn sec_ms_gec() -> String {
    let now = chrono::Utc::now().timestamp() + CLOCK_SKEW.load(Ordering::Relaxed);
    let mut ticks = now + WIN_EPOCH;
    ticks -= ticks % 300;
    let ticks100ns = ticks as i128 * 10_000_000;
    let mut h = Sha256::new();
    h.update(format!("{ticks100ns}{TRUSTED_CLIENT_TOKEN}"));
    hex::encode(h.finalize()).to_uppercase()
}

fn date_to_string() -> String {
    chrono::Utc::now().format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)").to_string()
}

fn escape(text: &str) -> String {
    // Caractères de contrôle refusés par le service, puis échappement XML.
    let clean: String = text
        .chars()
        .map(|c| {
            let n = c as u32;
            if n <= 8 || (11..=12).contains(&n) || (14..=31).contains(&n) {
                ' '
            } else {
                c
            }
        })
        .collect();
    clean.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn split_headers(data: &[u8], header_len: usize) -> (Vec<(String, String)>, &[u8]) {
    let head = String::from_utf8_lossy(&data[..header_len.min(data.len())]).to_string();
    let headers = head
        .split("\r\n")
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    let body = if data.len() >= header_len + 2 { &data[header_len + 2..] } else { &[][..] };
    (headers, body)
}

fn header<'a>(h: &'a [(String, String)], key: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Décodage MP3 → (échantillons mono float32, fréquence).
pub fn decode_mp3(data: Vec<u8>) -> Result<(Vec<f32>, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(data)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| e.to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("piste absente")?.clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    let mut sr = track.codec_params.sample_rate.unwrap_or(24000);
    let mut out = vec![];
    while let Ok(packet) = format.next_packet() {
        let Ok(decoded) = decoder.decode(&packet) else { continue };
        let spec = *decoded.spec();
        sr = spec.rate;
        let ch = spec.channels.count().max(1);
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks(ch) {
            out.push(frame.iter().sum::<f32>() / ch as f32);
        }
    }
    Ok((out, sr))
}

/// Synthèse d'une phrase : (échantillons mono, fréquence).
pub async fn synth(text: &str, voice: &str, rate: &str, pitch: &str) -> Result<(Vec<f32>, u32), String> {
    if USE_BRIDGE.load(Ordering::Relaxed) {
        return synth_bridge(text, voice, rate, pitch).await;
    }
    let first = match synth_once(text, voice, rate, pitch).await {
        Err(e) if e.starts_with("403:") => {
            // Horloge décalée : le service a refusé le jeton. Correction d'après sa date, puis nouvel essai.
            if let Some(server) = e.strip_prefix("403:").and_then(|d| chrono::DateTime::parse_from_rfc2822(d.trim()).ok()) {
                let skew = server.timestamp() - chrono::Utc::now().timestamp();
                CLOCK_SKEW.fetch_add(skew, Ordering::Relaxed);
            }
            synth_once(text, voice, rate, pitch).await
        }
        r => r,
    };
    match first {
        // Jeton refusé malgré l'heure corrigée : le service a changé, relais par le paquet Python à jour.
        Err(e) if e.starts_with("403:") => match synth_bridge(text, voice, rate, pitch).await {
            Ok(r) => {
                USE_BRIDGE.store(true, Ordering::Relaxed);
                crate::util::log("edge-tts : jeton refusé par Microsoft, relais par le paquet Python edge-tts");
                Ok(r)
            }
            Err(b) => Err(format!("{e} ; relais Python : {b}")),
        },
        r => r,
    }
}

/// Python à utiliser : JARVIS_PYTHON, sinon celui du PATH.
fn python() -> String {
    std::env::var("JARVIS_PYTHON").ok().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "python".into())
}

/// Relais Python resté ouvert : le paquet edge_tts n'est chargé qu'une fois (2 s), chaque phrase prend ensuite
/// environ 1 s. Une ligne JSON par demande sur l'entrée standard, une ligne JSON par réponse (même « id »).
const BRIDGE_PY: &str = r#"
import sys, json, asyncio, tempfile, os
import edge_tts
async def one(m):
    c = edge_tts.Communicate(m["text"], m["voice"], rate=m["rate"], pitch=m["pitch"])
    fd, path = tempfile.mkstemp(suffix=".mp3", prefix="jarvis-edge-")
    os.close(fd)
    await c.save(path)
    return path
sys.stdout.write(json.dumps({"ready": True}) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    m = {}
    try:
        m = json.loads(line)
        out = {"id": m.get("id"), "file": asyncio.run(one(m))}
    except Exception as e:
        out = {"id": m.get("id"), "error": (type(e).__name__ + ": " + str(e))[:300]}
    sys.stdout.write(json.dumps(out) + "\n"); sys.stdout.flush()
"#;

struct Bridge {
    _child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
}

fn bridge_slot() -> &'static tokio::sync::Mutex<Option<Bridge>> {
    static SLOT: std::sync::OnceLock<tokio::sync::Mutex<Option<Bridge>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| tokio::sync::Mutex::new(None))
}

async fn start_bridge() -> Result<Bridge, String> {
    use tokio::io::AsyncBufReadExt;
    let mut cmd = tokio::process::Command::new(python());
    cmd.args(["-u", "-c", BRIDGE_PY])
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd.spawn().map_err(|e| format!("Python introuvable ({e}) : installe Python puis « pip install edge-tts »"))?;
    let stdin = child.stdin.take().ok_or("entrée du relais indisponible")?;
    let mut lines = tokio::io::BufReader::new(child.stdout.take().ok_or("sortie du relais indisponible")?).lines();
    match tokio::time::timeout(std::time::Duration::from_secs(20), lines.next_line()).await {
        Ok(Ok(Some(l))) if l.contains("ready") => Ok(Bridge { _child: child, stdin, lines }),
        _ => Err("paquet Python edge-tts absent ou en échec : pip install -U edge-tts".into()),
    }
}

/// Synthèse par le paquet Python edge-tts (relais resté ouvert) : MP3 écrit dans un fichier temporaire, puis décodé.
async fn synth_bridge(text: &str, voice: &str, rate: &str, pitch: &str) -> Result<(Vec<f32>, u32), String> {
    use tokio::io::AsyncWriteExt;
    let clean: String = text.chars().map(|c| if (c as u32) < 32 { ' ' } else { c }).collect();
    let id = hex_id();
    let mut slot = bridge_slot().lock().await;
    if slot.is_none() {
        *slot = Some(start_bridge().await?);
    }
    let res: Result<String, String> = async {
        let b = slot.as_mut().unwrap();
        let req = serde_json::json!({ "id": id, "text": clean, "voice": voice, "rate": rate, "pitch": pitch });
        b.stdin.write_all(format!("{req}\n").as_bytes()).await.map_err(|e| e.to_string())?;
        b.stdin.flush().await.map_err(|e| e.to_string())?;
        // Une réponse d'une demande abandonnée (délai dépassé) peut précéder la nôtre : on l'écarte.
        loop {
            let line = tokio::time::timeout(std::time::Duration::from_secs(20), b.lines.next_line())
                .await
                .map_err(|_| "délai dépassé".to_string())?
                .map_err(|e| e.to_string())?
                .ok_or("relais Python arrêté")?;
            let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            if v["id"].as_str() != Some(id.as_str()) {
                if let Some(f) = v["file"].as_str() {
                    let _ = std::fs::remove_file(f);
                }
                continue;
            }
            return match v["file"].as_str() {
                Some(f) => Ok(f.to_string()),
                None => Err(v["error"].as_str().unwrap_or("échec").to_string()),
            };
        }
    }
    .await;
    let file = match res {
        Ok(f) => f,
        Err(e) => {
            if e.contains("arrêté") || e.contains("délai") || e.contains("pipe") {
                *slot = None; // relais à relancer à la prochaine phrase
            }
            return Err(e);
        }
    };
    drop(slot);
    let mp3 = std::fs::read(&file).unwrap_or_default();
    let _ = std::fs::remove_file(&file);
    if mp3.is_empty() {
        return Err("aucun son reçu".into());
    }
    decode_mp3(mp3)
}

async fn synth_once(text: &str, voice: &str, rate: &str, pitch: &str) -> Result<(Vec<f32>, u32), String> {
    let url = format!(
        "wss://{BASE_URL}/edge/v1?TrustedClientToken={TRUSTED_CLIENT_TOKEN}&ConnectionId={}&Sec-MS-GEC={}&Sec-MS-GEC-Version=1-{CHROMIUM_FULL_VERSION}",
        hex_id(),
        sec_ms_gec()
    );
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    let major = CHROMIUM_FULL_VERSION.split('.').next().unwrap_or("143");
    let hdrs = req.headers_mut();
    let mut put = |k: &'static str, v: String| {
        if let Ok(hv) = HeaderValue::from_str(&v) {
            hdrs.insert(k, hv);
        }
    };
    put("Pragma", "no-cache".into());
    put("Cache-Control", "no-cache".into());
    put("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".into());
    put("User-Agent", format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36 Edg/{major}.0.0.0"));
    put("Accept-Encoding", "gzip, deflate, br, zstd".into());
    put("Accept-Language", "en-US,en;q=0.9".into());
    put("Cookie", format!("muid={};", hex_id().to_uppercase()));
    let (mut ws, _) = match tokio_tungstenite::connect_async(req).await {
        Ok(c) => c,
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) if resp.status().as_u16() == 403 => {
            let date = resp.headers().get("Date").and_then(|d| d.to_str().ok()).unwrap_or("").to_string();
            return Err(format!("403:{date}"));
        }
        Err(e) => return Err(e.to_string()),
    };
    let config = format!(
        "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"true\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n",
        date_to_string()
    );
    ws.send(Message::Text(config.into())).await.map_err(|e| e.to_string())?;
    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><voice name='{voice}'><prosody pitch='{pitch}' rate='{rate}' volume='+0%'>{}</prosody></voice></speak>",
        escape(text)
    );
    let request = format!("X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}Z\r\nPath:ssml\r\n\r\n{ssml}", hex_id(), date_to_string());
    ws.send(Message::Text(request.into())).await.map_err(|e| e.to_string())?;
    let mut mp3 = vec![];
    while let Some(msg) = ws.next().await {
        match msg.map_err(|e| e.to_string())? {
            Message::Text(t) => {
                let bytes = t.as_bytes();
                let hl = t.find("\r\n\r\n").unwrap_or(bytes.len());
                let (h, _) = split_headers(bytes, hl);
                if header(&h, "Path") == Some("turn.end") {
                    break;
                }
            }
            Message::Binary(b) => {
                if b.len() < 2 {
                    return Err("message binaire sans longueur d'en-tête".into());
                }
                let hl = u16::from_be_bytes([b[0], b[1]]) as usize;
                if hl > b.len() {
                    return Err("en-tête plus long que le message".into());
                }
                let (h, data) = split_headers(&b, hl);
                if header(&h, "Path") != Some("audio") {
                    return Err("message binaire hors audio".into());
                }
                mp3.extend_from_slice(data);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = ws.close(None).await;
    if mp3.is_empty() {
        return Err("aucun son reçu".into());
    }
    decode_mp3(mp3)
}

#[cfg(test)]
mod tests {
    /// Réseau + Python requis : cargo test -p jarvis-voice relais -- --ignored --nocapture
    #[test]
    #[ignore]
    fn relais_python_reste_ouvert() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            for text in ["Bonjour, je suis Jarvis.", "Les accents : été, à, où, ça, forêt.", "Le rotor du H160 a cinq pales."] {
                let t = std::time::Instant::now();
                let (a, sr) = super::synth_bridge(text, "fr-FR-DeniseNeural", "+0%", "+0Hz").await.expect("synthèse");
                println!("{:.2} s → {} échantillons à {sr} Hz : {text}", t.elapsed().as_secs_f64(), a.len());
                assert!(a.len() > 10_000);
            }
        });
    }
}
