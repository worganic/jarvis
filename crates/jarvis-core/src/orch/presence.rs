// Présence par webcam (idée reprise du projet Cornelius) : le HUD envoie une petite image (320×240, pixels RVB) chaque
// seconde ; le noyau y compte les visages avec YuNet (ONNX, sur le PC) et n'en garde rien. Arrivée après une absence :
// briefing du matin s'il est attendu, puis les annonces gardées. Départ (aucun visage depuis quelques minutes) : les
// heures calmes s'appliquent, et la musique peut être mise en pause. Deux visages ou plus : mode vie privée (cards
// floutées sur le HUD, annonces de mails retenues). Aucune image n'est enregistrée. Réglage config.presence.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::{OrchRef, Up};
use crate::js::{self, J};

pub const FRAME_W: usize = 320;
pub const FRAME_H: usize = 240;
const MODEL_FILE: &str = "yunet.onnx";
const MODEL_URL: &str = "https://media.githubusercontent.com/media/opencv/opencv_zoo/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_detection_yunet/face_detection_yunet_2023mar.onnx";
const MODEL_SHA256: &str = "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4";
const MIN_SCORE: f32 = 0.7;
/// Visage trop petit (personne au loin, reflet, affiche) : ignoré.
const MIN_FACE_PX: f32 = 18.0;
/// Images consécutives pour entrer en vie privée, puis pour en sortir.
const PRIVACY_ON: u32 = 2;
const PRIVACY_OFF: u32 = 5;

/// Un visage dans l'image 320×240 : score, centre, taille.
#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub score: f32,
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
}

fn iou(a: &Face, b: &Face) -> f32 {
    let (ax1, ay1, ax2, ay2) = (a.cx - a.w / 2.0, a.cy - a.h / 2.0, a.cx + a.w / 2.0, a.cy + a.h / 2.0);
    let (bx1, by1, bx2, by2) = (b.cx - b.w / 2.0, b.cy - b.h / 2.0, b.cx + b.w / 2.0, b.cy + b.h / 2.0);
    let iw = (ax2.min(bx2) - ax1.max(bx1)).max(0.0);
    let ih = (ay2.min(by2) - ay1.max(by1)).max(0.0);
    let inter = iw * ih;
    let union = a.w * a.h + b.w * b.h - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Suppression des doublons (une même tête est vue par plusieurs ancres) : les meilleurs scores d'abord.
pub fn nms(mut faces: Vec<Face>) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut kept: Vec<Face> = vec![];
    for f in faces {
        if kept.iter().all(|k| iou(k, &f) < 0.3) {
            kept.push(f);
        }
    }
    kept
}

/// Image RVB 320×240 → tenseur YuNet [1, 3, 640, 640] : BGR, doublée au plus proche voisin dans 640×480, bas noir.
pub fn to_tensor(rgb: &[u8]) -> Vec<f32> {
    let plane = 640 * 640;
    let mut t = vec![0f32; 3 * plane];
    for y in 0..FRAME_H * 2 {
        for x in 0..640 {
            let p = ((y / 2) * FRAME_W + x / 2) * 3;
            let i = y * 640 + x;
            t[i] = rgb[p + 2] as f32;
            t[plane + i] = rgb[p + 1] as f32;
            t[2 * plane + i] = rgb[p] as f32;
        }
    }
    t
}

/// Décodage d'une échelle (pas 8, 16 ou 32) : coordonnées ramenées à l'image 320×240.
pub fn decode(stride: usize, cls: &[f32], obj: &[f32], bbox: &[f32]) -> Vec<Face> {
    let cols = 640 / stride;
    let s = stride as f32;
    let mut out = vec![];
    for i in 0..cls.len().min(obj.len()) {
        let score = (cls[i].clamp(0.0, 1.0) * obj[i].clamp(0.0, 1.0)).sqrt();
        if score < MIN_SCORE {
            continue;
        }
        let (r, c) = ((i / cols) as f32, (i % cols) as f32);
        let b = &bbox[i * 4..i * 4 + 4];
        let face = Face { score, cx: (c + b[0]) * s / 2.0, cy: (r + b[1]) * s / 2.0, w: b[2].exp() * s / 2.0, h: b[3].exp() * s / 2.0 };
        if face.w >= MIN_FACE_PX && face.h >= MIN_FACE_PX {
            out.push(face);
        }
    }
    out
}

#[cfg(feature = "semantic")]
pub struct Detector {
    session: ort::session::Session,
    _env: ort::environment::Environment,
}

#[cfg(feature = "semantic")]
impl Detector {
    pub fn load(file: &std::path::Path) -> Result<Self, String> {
        let env = ort::init().with_name("jarvis-presence").build().map_err(|e| e.to_string())?;
        let mut builder = ort::session::Session::builder(&env).map_err(|e| e.to_string())?;
        builder = builder.with_intra_threads(1).map_err(|e| e.to_string())?;
        let session = builder.commit_from_file(file).map_err(|e| e.to_string())?;
        Ok(Self { session, _env: env })
    }

    pub fn detect(&mut self, rgb: &[u8]) -> Result<Vec<Face>, String> {
        if rgb.len() != FRAME_W * FRAME_H * 3 {
            return Err("image de taille inattendue".into());
        }
        let input = ort::value::Tensor::from_array(([1usize, 3, 640, 640], to_tensor(rgb))).map_err(|e| e.to_string())?;
        let outputs = self.session.run(ort::inputs!["input" => input]).map_err(|e| e.to_string())?;
        let mut faces = vec![];
        for stride in [8usize, 16, 32] {
            let get = |name: String| outputs[name.as_str()].try_extract_tensor::<f32>().map(|(_, d)| d.to_vec()).map_err(|e| e.to_string());
            let (cls, obj, bbox) = (get(format!("cls_{stride}"))?, get(format!("obj_{stride}"))?, get(format!("bbox_{stride}"))?);
            faces.extend(decode(stride, &cls, &obj, &bbox));
        }
        Ok(nms(faces))
    }
}

#[cfg(not(feature = "semantic"))]
pub struct Detector;

#[cfg(not(feature = "semantic"))]
impl Detector {
    pub fn load(_: &std::path::Path) -> Result<Self, String> {
        Err("détection de visage non compilée (fonction « semantic »)".into())
    }
    pub fn detect(&mut self, _: &[u8]) -> Result<Vec<Face>, String> {
        Err("détection de visage non compilée".into())
    }
}

#[derive(Default)]
struct State {
    present: bool,
    faces: usize,
    last_seen_ms: i64,
    /// Début de l'absence en cours (0 : présent).
    away_since_ms: i64,
    privacy: bool,
    crowd_frames: u32,
    alone_frames: u32,
    last_error: String,
}

pub struct Presence {
    o: OrchRef,
    model: PathBuf,
    detector: RefCell<Option<Arc<Mutex<Detector>>>>,
    loading: Cell<bool>,
    busy: Cell<bool>,
    state: RefCell<State>,
}

impl Presence {
    pub fn new(o: OrchRef) -> Self {
        Self { o, model: crate::paths::models_dir().join(MODEL_FILE), detector: RefCell::new(None), loading: Cell::new(false), busy: Cell::new(false), state: RefCell::new(State::default()) }
    }

    fn settings(&self) -> J {
        self.o.up().config.get("presence")
    }

    pub fn enabled(&self) -> bool {
        self.settings()["enabled"] == json!(true)
    }

    /// Ce que le HUD doit faire : capturer ou non, et à quel rythme.
    pub fn hud_config(&self) -> J {
        let cfg = self.settings();
        json!({ "enabled": self.enabled(), "intervalMs": 1000, "width": FRAME_W, "height": FRAME_H, "privacyBlur": cfg["privacy"] != json!(false) })
    }

    /// État pour l'administration, la page d'aide et le HUD (jamais d'image).
    pub fn state(&self) -> J {
        let s = self.state.borrow();
        json!({
            "enabled": self.enabled(), "present": s.present, "faces": s.faces, "privacy": s.privacy,
            "lastSeen": if s.last_seen_ms > 0 { json!(js::iso_of(s.last_seen_ms)) } else { json!(null) },
            "model": self.model.exists(), "error": s.last_error
        })
    }

    /// Plus de capture (réglage coupé) : on oublie l'état.
    pub fn reset(&self) {
        *self.state.borrow_mut() = State::default();
        self.o.up().emit(json!({ "type": "presence", "present": false, "faces": 0, "privacy": false }));
    }

    pub fn in_privacy(&self) -> bool {
        self.state.borrow().privacy
    }

    fn set_error(&self, e: &str) {
        self.state.borrow_mut().last_error = e.to_string();
    }

    /// Charge YuNet, après l'avoir téléchargé et vérifié s'il manque.
    async fn ensure_detector(&self) -> Option<Arc<Mutex<Detector>>> {
        if let Some(d) = self.detector.borrow().clone() {
            return Some(d);
        }
        if self.loading.replace(true) {
            return None;
        }
        let res = async {
            if !self.model.exists() {
                let r = crate::net::http().get(MODEL_URL).timeout(std::time::Duration::from_secs(60)).send().await.map_err(|e| e.to_string())?;
                if !r.status().is_success() {
                    return Err(format!("téléchargement du modèle : HTTP {}", r.status().as_u16()));
                }
                let bytes = r.bytes().await.map_err(|e| e.to_string())?;
                use sha2::Digest;
                if hex::encode(sha2::Sha256::digest(&bytes)) != MODEL_SHA256 {
                    return Err("modèle YuNet refusé : empreinte inattendue".into());
                }
                if let Some(dir) = self.model.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::write(&self.model, &bytes).map_err(|e| e.to_string())?;
            }
            let file = self.model.clone();
            tokio::task::spawn_blocking(move || Detector::load(&file)).await.map_err(|e| e.to_string())?
        }
        .await;
        self.loading.set(false);
        match res {
            Ok(d) => {
                let d = Arc::new(Mutex::new(d));
                *self.detector.borrow_mut() = Some(d.clone());
                self.set_error("");
                Some(d)
            }
            Err(e) => {
                self.set_error(&e);
                None
            }
        }
    }

    /// Image du HUD (base64 de 320×240×3 octets RVB). Rend le nombre de visages.
    pub async fn frame(&self, b64: &str) -> J {
        if !self.enabled() {
            return json!({ "ok": false, "enabled": false });
        }
        use base64::Engine;
        let Ok(rgb) = base64::engine::general_purpose::STANDARD.decode(b64) else { return json!({ "ok": false, "error": "image illisible" }) };
        if rgb.len() != FRAME_W * FRAME_H * 3 {
            return json!({ "ok": false, "error": "image de taille inattendue" });
        }
        if self.busy.replace(true) {
            return json!({ "ok": false, "error": "analyse précédente en cours" });
        }
        let detected = match self.ensure_detector().await {
            Some(d) => tokio::task::spawn_blocking(move || d.lock().map_err(|_| "détecteur indisponible".to_string())?.detect(&rgb)).await.unwrap_or_else(|e| Err(e.to_string())),
            None => Err(self.state.borrow().last_error.clone()),
        };
        self.busy.set(false);
        match detected {
            Ok(faces) => {
                self.observe(faces.len(), js::now_ms());
                json!({ "ok": true, "faces": faces.len() })
            }
            Err(e) => {
                self.set_error(&e);
                json!({ "ok": false, "error": e })
            }
        }
    }

    /// Fait évoluer la présence d'après le nombre de visages vus à l'instant `now`.
    pub fn observe(&self, faces: usize, now: i64) {
        let cfg = self.settings();
        let away_ms = (js::num(&cfg["awayMinutes"]).unwrap_or(5.0).clamp(1.0, 120.0) * 60_000.0) as i64;
        let privacy_wanted = cfg["privacy"] != json!(false);
        let (arrived, left, privacy_changed) = {
            let mut s = self.state.borrow_mut();
            s.faces = faces;
            let mut arrived = None;
            let mut left = false;
            if faces > 0 {
                if !s.present {
                    s.present = true;
                    arrived = Some(if s.away_since_ms > 0 { now - s.away_since_ms } else { i64::MAX });
                }
                s.last_seen_ms = now;
                s.away_since_ms = 0;
            } else if s.present && now - s.last_seen_ms >= away_ms {
                s.present = false;
                s.away_since_ms = s.last_seen_ms;
                left = true;
            }
            let before = s.privacy;
            if faces >= 2 {
                s.crowd_frames += 1;
                s.alone_frames = 0;
            } else {
                s.alone_frames += 1;
                s.crowd_frames = 0;
            }
            if privacy_wanted && !s.privacy && s.crowd_frames >= PRIVACY_ON {
                s.privacy = true;
            } else if s.privacy && (!privacy_wanted || s.alone_frames >= PRIVACY_OFF) {
                s.privacy = false;
            }
            (arrived, left, before != s.privacy)
        };
        let Some(o) = self.o.upgrade() else { return };
        if faces > 0 {
            o.announcer.seen();
        }
        if let Some(away) = arrived {
            self.on_arrival(away, away_ms);
        }
        if left {
            o.record("presence_depart", json!({}));
            if cfg["pauseMusic"] == json!(true) {
                let o2 = o.clone();
                js::spawn(async move {
                    o2.media_action(json!({ "action": "pause" })).await;
                });
            }
        }
        if arrived.is_some() || left || privacy_changed {
            let s = self.state.borrow();
            if privacy_changed {
                o.record("presence_vie_privee", json!({ "active": s.privacy }));
            }
            o.emit(json!({ "type": "presence", "present": s.present, "faces": s.faces, "privacy": s.privacy }));
        }
    }

    /// Retour devant le PC après une vraie absence : briefing du matin s'il est attendu, puis annonces gardées.
    fn on_arrival(&self, away: i64, away_ms: i64) {
        let Some(o) = self.o.upgrade() else { return };
        if away < away_ms {
            return;
        }
        o.record("presence_arrivee", json!({ "absenceMinutes": if away == i64::MAX { json!(null) } else { json!(away / 60_000) } }));
        if o.briefing.wants_auto() {
            let intent = o.briefing.intent("Bonjour !");
            js::spawn(async move {
                o.run_intent(intent, "briefing à l'arrivée").await;
            });
        } else {
            o.announcer.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_doublons_d_un_meme_visage_sont_fusionnes() {
        let f = |score, cx| Face { score, cx, cy: 100.0, w: 60.0, h: 70.0 };
        let kept = nms(vec![f(0.8, 100.0), f(0.9, 102.0), f(0.85, 99.0), f(0.9, 250.0)]);
        assert_eq!(kept.len(), 2);
        assert!((kept[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn le_decodage_ramene_les_coordonnees_a_l_image_du_hud() {
        // Ancre de la ligne 2, colonne 3 au pas 32 (20 colonnes), visage de exp(ln 2)*32 = 64 px dans le tenseur, 32 px à l'écran.
        let cols = 20;
        let n = cols * cols;
        let (mut cls, mut obj, mut bbox) = (vec![0f32; n], vec![0f32; n], vec![0f32; n * 4]);
        let i = 2 * cols + 3;
        cls[i] = 0.9;
        obj[i] = 0.9;
        bbox[i * 4] = 0.5;
        bbox[i * 4 + 1] = 0.5;
        bbox[i * 4 + 2] = std::f32::consts::LN_2;
        bbox[i * 4 + 3] = std::f32::consts::LN_2;
        let faces = decode(32, &cls, &obj, &bbox);
        assert_eq!(faces.len(), 1);
        let f = faces[0];
        assert!((f.cx - 56.0).abs() < 1e-3 && (f.cy - 40.0).abs() < 1e-3 && (f.w - 32.0).abs() < 1e-3);
        assert!((f.score - 0.9).abs() < 1e-3);
        // Trop petit après réduction (exp(-1)*8/2 ≈ 1,5 px) : ignoré.
        let mut tiny = bbox.clone();
        tiny[i * 4 + 2] = -1.0;
        tiny[i * 4 + 3] = -1.0;
        assert!(decode(8, &cls[..], &obj[..], &tiny[..]).is_empty());
    }

    /// Essai réel : JARVIS_TEST_YUNET=<dossier contenant yunet.onnx et des images *.rgb (320×240 RVB)>, avec
    /// ORT_DYLIB_PATH vers onnxruntime.dll. Chaque image doit contenir exactement un visage.
    #[cfg(feature = "semantic")]
    #[test]
    #[ignore]
    fn yunet_trouve_un_visage_par_image() {
        let dir = PathBuf::from(std::env::var("JARVIS_TEST_YUNET").expect("JARVIS_TEST_YUNET"));
        let mut d = Detector::load(&dir.join("yunet.onnx")).unwrap();
        let mut seen = 0;
        for e in std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().extension().map(|x| x == "rgb").unwrap_or(false)) {
            let faces = d.detect(&std::fs::read(e.path()).unwrap()).unwrap();
            assert_eq!(faces.len(), 1, "{:?} : {faces:?}", e.path());
            seen += 1;
        }
        assert!(seen > 0);
    }

    #[test]
    fn le_tenseur_est_en_bgr_et_double() {
        let mut rgb = vec![0u8; FRAME_W * FRAME_H * 3];
        rgb[0] = 10; // R du pixel (0,0)
        rgb[1] = 20;
        rgb[2] = 30;
        let t = to_tensor(&rgb);
        let plane = 640 * 640;
        assert_eq!((t[0], t[plane], t[2 * plane]), (30.0, 20.0, 10.0));
        assert_eq!(t[641], 30.0, "pixel doublé en (1,1)");
        assert_eq!(t[480 * 640], 0.0, "bas du carré laissé noir");
    }
}
