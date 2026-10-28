// Moteur vocal unifié de Jarvis v4 (voice/engine.py de la v3) — un seul processus, sans serveur tiers.
//
// Chaîne (chaque étage filtre avant l'étage plus coûteux) :
//
//   micro WASAPI 48 kHz ─► anti-écho / bruit / gain (WebRTC APM, AEC3) ─► 16 kHz
//         ▲ référence : tout le son du PC (loopback), sinon la voix de Jarvis elle-même
//   ─► VAD Silero ─► mot d'éveil acoustique (accélérateur) ─► garde « Jarvis »
//   ─► Whisper large-v3-turbo (GPU) + filtres anti-hallucination ─► locuteur (« signaler »)
//   ─► phrase finale envoyée au noyau
//
//   voix : edge-tts (Denise/Henri…) ; Piper local en secours automatique ; cache audio des phrases
//   fréquentes ; interruption par la parole (barge-in).
//
// Protocole WebSocket (ws://127.0.0.1:JARVIS_ENGINE_PORT), réservé au noyau (jeton obligatoire) :
// identique à la v3 (voir voice_link.rs du noyau).
//
// Tests sans micro ni haut-parleur : JARVIS_ENGINE_INPUT=<fichier.wav|liste;…>,
// JARVIS_ENGINE_OUTPUT=null (ou un chemin .wav où la voix est enregistrée).
//
// Sous-commandes de diagnostic : embed <wav> <modèle>, stt <wav…>, say <texte> <wav>, wake <wav…>,
// bench stt|speaker (mesure sur le corpus de l'utilisateur).

#![allow(dead_code)]

#[macro_use]
mod util;
mod audio;
mod bench;
mod corpus;
mod debug;
mod edge;
mod filters;
mod hub;
mod listener;
mod meeting;
mod oww;
mod speaker;
mod tts;
mod vad;
mod wake;
mod whisper;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use crate::util::{log, J};

fn default_options() -> J {
    json!({
        "echoCancellation": true,
        "noiseSuppression": true,
        "autoGainControl": true,
        "systemAudioReference": true,
        "minEndpointingDelay": 0.45,
        "maxEndpointingDelay": 2.5,
        "maxSegment": 25.0,
        "allowInterruptions": true,
        "minInterruptionDuration": 0.25,
        "sttModel": "turbo",
        "sttBeam": 2,
        "acousticWake": true,
        "wakeEngine": "phonetique",
        "wakeModel": "jarvis.onnx",
        "wakeThreshold": 0.5,
        "wakePrefixSeconds": 1.6,
        "ttsFallback": true,
        "debugAudio": false,
        "sttRawAudio": true
    })
}

fn options() -> J {
    let mut o = default_options();
    if let Ok(raw) = std::env::var("JARVIS_ENGINE_OPTIONS") {
        if let Ok(J::Object(m)) = serde_json::from_str::<J>(&raw) {
            for (k, v) in m {
                o[k] = v;
            }
        }
    }
    o
}

struct Engine {
    hub: hub::Hub,
    audio: Arc<audio::Audio>,
    voice: Arc<tts::Voice>,
    listener: Arc<listener::Listener>,
    verifier: Arc<Mutex<speaker::SpeakerVerifier>>,
    corpus: Arc<corpus::CorpusRecorder>,
    meeting: Arc<meeting::MeetingRecorder>,
    opts: Arc<Mutex<J>>,
}

fn s(msg: &J, k: &str) -> String {
    msg[k].as_str().unwrap_or("").to_string()
}

impl Engine {
    async fn dispatch(self: &Arc<Self>, kind: &str, msg: &J) -> Option<String> {
        let emit_with = |t: &str, extra: J| {
            let mut ev = json!({ "t": t });
            if let (Some(e), Some(x)) = (ev.as_object_mut(), extra.as_object()) {
                for (k, v) in x {
                    e.insert(k.clone(), v.clone());
                }
            }
            self.hub.emit(ev);
        };
        match kind {
            "say" => {
                let text = s(msg, "text");
                self.listener.remember_said(&text);
                if msg["interrupt"].as_bool().unwrap_or(false) {
                    self.voice.stop();
                }
                self.voice.say_with(&text, msg["voice"].as_str());
            }
            "say_end" => {} // la file de parole s'enchaîne d'elle-même
            "stop" => self.voice.stop(),
            "duck" => self.voice.hold(true, None),
            // Reprise demandée par le noyau : pas si l'utilisateur a repris la parole entre-temps.
            "unduck" => {
                if !self.hub.state_flag("capturing") {
                    self.voice.hold(false, None);
                }
            }
            "volume" => self.voice.set_gain(msg["level"].as_f64().unwrap_or(1.0)),
            "voice" | "voix" => {
                let v = msg["voice"].as_str().or_else(|| msg["name"].as_str());
                self.voice.set_voice(v, msg["rate"].as_str(), msg["pitch"].as_str(), msg["effect"].as_str());
                if msg["sample"].as_bool().unwrap_or(false) {
                    self.voice.stop();
                    let name = self.voice.settings.lock().unwrap().voice_name.clone();
                    let text = msg["sample_text"].as_str().map(String::from).unwrap_or_else(|| format!("Bonjour, je parle désormais avec la voix de {name}."));
                    self.voice.say(&text);
                }
            }
            "vocab" => self.listener.set_vocabulary(&msg["terms"], &msg["corrections"]),
            "mute" => {
                self.listener.s.muted.store(true, Ordering::SeqCst);
                self.audio.send(audio::AudioCmd::Suspend(true));
                self.hub.emit_state(json!({ "muted": true }));
            }
            "unmute" => {
                self.listener.s.muted.store(false, Ordering::SeqCst);
                self.audio.send(audio::AudioCmd::Suspend(false));
                self.hub.emit_state(json!({ "muted": false }));
            }
            "gate" => self.listener.set_gate(msg["open"].as_bool().unwrap_or(false), msg["seconds"].as_f64().unwrap_or(12.0)),
            "mode" => {
                let m = if msg["mode"] == "continuous" { "continuous" } else { "wakeword" };
                *self.listener.s.mode.lock().unwrap() = m.into();
                log(&format!("mode d'écoute : {m}"));
            }
            "tune" => {
                let mut o = self.opts.lock().unwrap();
                for k in ["minEndpointingDelay", "maxEndpointingDelay", "minInterruptionDuration", "allowInterruptions"] {
                    if !msg[k].is_null() {
                        o[k] = msg[k].clone();
                    }
                }
                log("délais d'écoute ajustés (pris en compte au prochain segment)");
            }
            "pc_meter" => self.audio.send(audio::AudioCmd::PcMeter(msg["on"].as_bool().unwrap_or(false))),
            "meeting" => {
                let m = self.meeting.clone();
                let on = msg["on"].as_bool().unwrap_or(false);
                let _ = tokio::task::spawn_blocking(move || if on { m.start() } else { m.stop() }).await;
            }
            "meeting_name_voice" => {
                let (m, id, name, t) = (self.meeting.clone(), s(msg, "id"), s(msg, "name"), s(msg, "type"));
                let _ = tokio::task::spawn_blocking(move || m.name_voice(&id, &name, &t)).await;
                let st = self.verifier.lock().unwrap().get_status();
                self.hub.emit_state(json!({ "speaker": st }));
            }
            "reopen_audio" => self.audio.send(audio::AudioCmd::Reopen),
            "corpus" => {
                let config = self.audio.shared.config();
                match msg["action"].as_str() {
                    Some("phrase") => self.corpus.start_phrase(&s(msg, "id"), &s(msg, "text"), &config),
                    Some("negatif") => self.corpus.start_negative(msg["seconds"].as_f64().unwrap_or(60.0), &config),
                    _ => self.corpus.stop(),
                }
            }
            "speaker_status" => {
                let mut st = self.verifier.lock().unwrap().get_status();
                st["t"] = json!("speaker_status");
                return Some(st.to_string());
            }
            "speaker_enroll_start" => {
                let t = msg["type"].as_str().map(String::from);
                let status = self.verifier.lock().unwrap().start_enrollment(msg["name"].as_str().unwrap_or("Johann"), msg["count"].as_u64().unwrap_or(8) as usize, t.as_deref());
                self.hub.emit(json!({ "t": "speaker_enroll_started", "status": status }));
                self.hub.emit_state(json!({ "speaker": status }));
            }
            "speaker_enroll_cancel" => {
                let st = self.verifier.lock().unwrap().cancel_enrollment();
                self.hub.emit_state(json!({ "speaker": st }));
            }
            "speaker_delete" => {
                let r = self.verifier.lock().unwrap().remove_speaker(&s(msg, "id"));
                emit_with("speaker_deleted", r);
            }
            "speaker_rename" => {
                let r = self.verifier.lock().unwrap().rename_speaker(&msg["id"], msg["name"].as_str());
                emit_with("speaker_renamed", r);
            }
            "speaker_primary" => {
                let r = self.verifier.lock().unwrap().set_primary(&s(msg, "id"));
                emit_with("speaker_updated", r);
            }
            "speaker_set_enabled" => {
                let st = self.verifier.lock().unwrap().set_enabled(msg["enabled"].as_bool().unwrap_or(true));
                self.hub.emit_state(json!({ "speaker": st }));
            }
            "speaker_set_mode" => {
                let st = self.verifier.lock().unwrap().set_mode(&s(msg, "mode"));
                self.hub.emit_state(json!({ "speaker": st }));
            }
            "speaker_set_threshold" => {
                let st = self.verifier.lock().unwrap().set_threshold(msg["threshold"].as_f64().unwrap_or(0.65));
                self.hub.emit_state(json!({ "speaker": st }));
            }
            "speaker_set_thresholds" => {
                let mut v = self.verifier.lock().unwrap();
                v.set_thresholds(&msg["thresholds"]);
                let st = v.get_status();
                drop(v);
                self.hub.emit_state(json!({ "speaker": st }));
            }
            "speaker_learn" => {
                let (v, sample, id) = (self.verifier.clone(), s(msg, "sample"), s(msg, "id"));
                let r = tokio::task::spawn_blocking(move || v.lock().unwrap().learn(&sample, &id, "confirme")).await.unwrap_or_default();
                let mut ev = r;
                ev["sample"] = msg["sample"].clone();
                emit_with("speaker_learned", ev);
            }
            "speaker_create" => {
                let (v, sample, name, t) = (self.verifier.clone(), s(msg, "sample"), s(msg, "name"), s(msg, "type"));
                let r = tokio::task::spawn_blocking(move || v.lock().unwrap().create_from_sample(&sample, &name, &t)).await.unwrap_or_default();
                let mut ev = r;
                ev["sample"] = msg["sample"].clone();
                emit_with("speaker_learned", ev);
            }
            "speaker_unlearn" => {
                let (v, sample) = (self.verifier.clone(), s(msg, "sample"));
                let r = tokio::task::spawn_blocking(move || v.lock().unwrap().unlearn(&sample)).await.unwrap_or_default();
                let mut ev = r;
                ev["sample"] = msg["sample"].clone();
                emit_with("speaker_unlearned", ev);
            }
            "speaker_set_type" => {
                let r = self.verifier.lock().unwrap().set_type(&s(msg, "id"), &s(msg, "type"));
                emit_with("speaker_updated", r);
            }
            "speaker_set_options" => {
                let st = self.verifier.lock().unwrap().set_options(msg["doubtMargin"].as_f64(), msg["autoLearn"].as_bool(), msg["learning"].as_bool());
                self.hub.emit_state(json!({ "speaker": st }));
            }
            "speaker_reset" => {
                let status = self.verifier.lock().unwrap().reset();
                self.hub.emit(json!({ "t": "speaker_reset", "status": status }));
            }
            _ => {}
        }
        None
    }
}

/// Mémoire (Mo) et CPU (%) du moteur, toutes les 30 s.
fn process_metrics(prev: &mut Option<(u64, std::time::Instant)>) -> J {
    let Some((ws, cpu)) = jarvis_native::window::process_stats(std::process::id()) else { return Value::Null };
    let now = std::time::Instant::now();
    let pct = match prev {
        Some((c, t)) => {
            let dt = now.duration_since(*t).as_secs_f64();
            if dt > 0.0 {
                (cpu.saturating_sub(*c)) as f64 / 1e7 / dt * 100.0
            } else {
                0.0
            }
        }
        None => 0.0,
    };
    *prev = Some((cpu, now));
    json!({ "t": "metrics", "mb": (ws as f64 / 1e6).round(), "cpu": (pct * 10.0).round() / 10.0 })
}

async fn run() -> i32 {
    let opts = Arc::new(Mutex::new(options()));
    let port: u16 = std::env::var("JARVIS_ENGINE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8798);
    let token = std::env::var("JARVIS_ENGINE_TOKEN").unwrap_or_default();
    let hub = hub::Hub::new();
    hub.emit_state(json!({ "backend": "engine", "tts": "edge" }));
    // Le port d'abord : un second moteur s'arrête aussitôt (code 3) au lieu d'ouvrir le micro.
    let server = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(_) => {
            log(&format!("le port {port} est déjà pris : un autre moteur vocal tourne déjà"));
            return 3;
        }
    };
    // Mot d'éveil openWakeWord demandé : ses deux modèles de prétraitement sont téléchargés s'ils manquent.
    if opts.lock().unwrap()["wakeEngine"].as_str().map(|m| m != "phonetique").unwrap_or(false) {
        if let Err(e) = oww::ensure_preprocess().await {
            log(&format!("openWakeWord : prétraitement indisponible ({e})"));
        }
    }
    let (audio, blocks) = audio::Audio::start(opts.lock().unwrap().clone(), hub.clone());
    let audio = Arc::new(audio);
    let debug = if opts.lock().unwrap()["debugAudio"].as_bool().unwrap_or(false) {
        let d = Arc::new(debug::AudioDebug::new(util::data_dir().join("voice-debug")));
        log(&format!("diagnostic audio actif : chaque phrase est gardée dans {}", d.folder.display()));
        *audio.shared.debug_ref.lock().unwrap() = Some(d.clone());
        Some(d)
    } else {
        None
    };
    let profile = std::env::var("JARVIS_VOICEPRINT").map(std::path::PathBuf::from).unwrap_or_else(|_| util::data_dir().join("voiceprint.json"));
    let verifier = Arc::new(Mutex::new(speaker::SpeakerVerifier::new(
        util::models_dir().join("speaker").join("wespeaker_en_voxceleb_CAM++.onnx"),
        profile,
        std::env::var("JARVIS_VOICE_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| util::data_dir().join("voice-corpus")),
    )));
    let corpus = corpus::CorpusRecorder::new();
    let t0 = std::time::Instant::now();
    log("chargement de Whisper…");
    let loaded = tokio::task::spawn_blocking(|| {
        let mut w = whisper::Whisper::load(&util::models_dir().join("whisper-large-v3-turbo-onnx"))?;
        w.warmup()?;
        Ok::<_, String>(w)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    let whisper = match loaded {
        Ok(w) => w,
        Err(e) => {
            log(&format!("Whisper indisponible : {e}"));
            return 1;
        }
    };
    let label = whisper.label.clone();
    log(&format!("Whisper prêt : {label} ({:.1} s)", t0.elapsed().as_secs_f64()));
    let whisper = Arc::new(Mutex::new(whisper));
    // Mémoire vidéo rendue au bureau et aux navigateurs quand on ne parle plus : Whisper se recharge à la phrase suivante.
    let idle = std::time::Duration::from_secs(60 * std::env::var("JARVIS_STT_IDLE_MIN").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(10));
    if !idle.is_zero() && label.contains("gpu") {
        let w = whisper.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(30));
            if let Ok(mut g) = w.try_lock() {
                if g.unload_if_idle(idle) {
                    log("Whisper déchargé du GPU (inactif) : mémoire vidéo libérée");
                }
            }
        });
    }
    audio.send(audio::AudioCmd::Open); // d'abord le micro (et la liste des périphériques)
    let voice = tts::Voice::start(hub.clone(), audio.clone(), opts.clone()); // ensuite la sortie de la voix
    let listener = listener::Listener::start(hub.clone(), audio.clone(), voice.clone(), whisper, verifier.clone(), opts.clone(), corpus.clone(), blocks, debug);
    let (l1, l2) = (listener.clone(), listener.clone());
    let meeting = meeting::MeetingRecorder::new(
        hub.clone(),
        audio.clone(),
        Arc::new(move |a: &[f32]| l1.meeting_transcribe(a)),
        Arc::new(move || l2.recent_phrases(20.0)),
        verifier.clone(),
    );
    let engine = Arc::new(Engine { hub: hub.clone(), audio: audio.clone(), voice: voice.clone(), listener, verifier: verifier.clone(), corpus, meeting, opts });
    // Mesure mémoire / CPU du moteur, toutes les 30 s.
    let h2 = hub.clone();
    tokio::spawn(async move {
        let mut prev = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let m = tokio::task::spawn_blocking(move || {
                let mut p = prev;
                let r = process_metrics(&mut p);
                (r, p)
            })
            .await;
            if let Ok((m, p)) = m {
                prev = p;
                if !m.is_null() {
                    h2.emit(m);
                }
            }
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (vname, vid, rate) = {
        let st = voice.settings.lock().unwrap();
        (st.voice_name.clone(), st.voice.clone(), st.rate.clone())
    };
    let spk_status = verifier.lock().unwrap().get_status();
    hub.emit_state(json!({ "ready": true, "model": label, "voice": vid, "voice_name": vname, "rate": rate, "speaker": spk_status, "config": audio.shared.config() }));
    log(&format!("moteur vocal prêt · ws://127.0.0.1:{port}"));
    loop {
        let Ok((stream, _)) = server.accept().await else { continue };
        let (engine, token) = (engine.clone(), token.clone());
        tokio::spawn(async move {
            let Ok(ws) = tokio_tungstenite::accept_async_with_config(stream, Some(tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(1 << 20)))).await else { return };
            let (mut sink, mut source) = ws.split();
            let mut authed = token.is_empty();
            let mut rx = engine.hub.subscribe();
            if authed {
                let _ = sink.send(Message::Text(engine.hub.state().to_string().into())).await;
            }
            loop {
                tokio::select! {
                    incoming = source.next() => {
                        let Some(Ok(msg)) = incoming else { break };
                        let text = match msg { Message::Text(t) => t.to_string(), Message::Close(_) => break, _ => continue };
                        let Ok(m) = serde_json::from_str::<J>(&text) else { continue };
                        let kind = m["t"].as_str().unwrap_or("").to_string();
                        if !authed {
                            if kind == "auth" && m["token"].as_str() == Some(token.as_str()) {
                                authed = true;
                                let _ = sink.send(Message::Text(engine.hub.state().to_string().into())).await;
                                continue;
                            }
                            let _ = sink.send(Message::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame { code: 4001.into(), reason: "jeton invalide".into() }))).await;
                            break;
                        }
                        if let Some(reply) = engine.dispatch(&kind, &m).await {
                            let _ = sink.send(Message::Text(reply.into())).await;
                        }
                    }
                    ev = rx.recv(), if authed => {
                        match ev {
                            Ok(s) => if sink.send(Message::Text(s.into())).await.is_err() { break },
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }
                }
            }
        });
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bench") => std::process::exit(bench::main(&args[1..])),
        Some("embed") => {
            // Diagnostic : empreinte d'un fichier WAV (comparaison avec la v3).
            let mut ex = speaker::WespeakerEmbedder::new(std::path::Path::new(&args[2]), 2).expect("modèle");
            let (audio, _) = util::read_wav(std::path::Path::new(&args[1])).expect("wav");
            let e = ex.embed(&audio).expect("empreinte");
            let n: f32 = e.iter().map(|x| x * x).sum::<f32>().sqrt();
            println!("{}", e.iter().take(6).map(|x| format!("{:.5}", x / n)).collect::<Vec<_>>().join(" "));
            return;
        }
        Some("stt") => {
            let t0 = std::time::Instant::now();
            let mut w = whisper::Whisper::load(&util::models_dir().join("whisper-large-v3-turbo-onnx")).expect("Whisper");
            println!("chargé en {:.1} s ({})", t0.elapsed().as_secs_f64(), w.label);
            let _ = w.warmup();
            for f in &args[1..] {
                let (audio, sr) = util::read_wav(std::path::Path::new(f)).expect("wav");
                let audio = util::interp_resample(&audio, sr, 16000);
                let t = std::time::Instant::now();
                let beam = std::env::var("BEAM").ok().and_then(|b| b.parse().ok()).unwrap_or(2);
                let d = w.transcribe(&audio, &filters::build_initial_prompt(&[]), None, beam);
                println!("{:.2} s · « {} » → « {} » {}", t.elapsed().as_secs_f64(), d.text, filters::clean_and_autocorrect(&d.text, &[]), d.info());
            }
            return;
        }
        Some("say") => {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            match rt.block_on(edge::synth(&args[1], "fr-FR-DeniseNeural", "+0%", "+0Hz")) {
                Ok((a, sr)) => {
                    let _ = util::write_wav(std::path::Path::new(&args[2]), &a, sr as usize);
                    println!("{} échantillons à {sr} Hz", a.len());
                }
                Err(e) => println!("échec : {e}"),
            }
            return;
        }
        Some("wake") => {
            let mut v = vad::Vad::load(&util::models_dir().join("vad").join("silero_vad_v6.onnx")).expect("VAD");
            let (k, t, sc) = wake::calibrated_settings();
            let mut w = wake::WakeSpotter::new(&k, t, sc).expect("KWS");
            for f in &args[1..] {
                let (audio, sr) = util::read_wav(std::path::Path::new(f)).expect("wav");
                let audio = util::interp_resample(&audio, sr, 16000);
                let (mut voiced, mut heard) = (0, vec![]);
                for (i, b) in audio.chunks(512).enumerate() {
                    if b.len() < 512 {
                        break;
                    }
                    if v.prob(b) >= 0.5 {
                        voiced += 1;
                    }
                    if w.feed(b) {
                        heard.push(format!("{:.2}s", i as f64 * 0.032));
                    }
                }
                println!("{f} : {voiced}/{} blocs de voix, « Jarvis » entendu à {heard:?}", audio.len() / 512);
            }
            return;
        }
        _ => {}
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).build().expect("runtime");
    let code = rt.block_on(run());
    std::process::exit(code);
}
