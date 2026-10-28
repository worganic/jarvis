// Lumières Yeelight (orchestrator/lights.js), protocole local officiel : découverte SSDP vers
// 239.255.255.250:1982, commandes JSON sur TCP 55443. Pré-requis : « Contrôle LAN » activé.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::js::{self, J};

pub fn color(name: &str) -> Option<i64> {
    Some(match name {
        "rouge" => 0xff0000,
        "vert" => 0x00ff00,
        "bleu" => 0x0000ff,
        "jaune" => 0xffd000,
        "orange" => 0xff7a00,
        "violet" => 0x8a2be2,
        "rose" => 0xff4f9a,
        "cyan" => 0x00e5ff,
        "turquoise" => 0x30d5c8,
        "blanc" => 0xffffff,
        "magenta" => 0xff00ff,
        "indigo" => 0x4b0082,
        _ => return None,
    })
}

pub const COLOR_NAMES: &[&str] = &["rouge", "vert", "bleu", "jaune", "orange", "violet", "rose", "cyan", "turquoise", "blanc", "magenta", "indigo"];

/// (clé, libellé, luminosité, température, rgb)
pub const SCENES: &[(&str, &str, i64, i64, i64)] = &[
    ("travail", "travail", 100, 5000, 0),
    ("concentration", "concentration", 100, 5500, 0),
    ("detente", "détente", 35, 2700, 0),
    ("cinema", "cinéma", 8, 0, 0x1a2cff),
    ("lecture", "lecture", 80, 4000, 0),
    ("nuit", "nuit", 1, 2000, 0),
    ("soiree", "soirée", 50, 0, 0xff7a3a),
];

pub fn parse_ssdp(text: &str) -> Option<J> {
    let get = |name: &str| -> String {
        regex::Regex::new(&format!(r"(?mi)^{name}:\s*(.*)$")).unwrap().captures(text).map(|c| c[1].trim().to_string()).unwrap_or_default()
    };
    let loc = get("Location");
    let m = crate::re!(r"yeelight://([\d.]+):(\d+)").captures(&loc)?;
    let n = |k: &str| get(k).parse::<f64>().unwrap_or(0.0);
    Some(json!({
        "id": get("id"), "ip": &m[1], "port": m[2].parse::<u16>().unwrap_or(55443), "model": get("model"),
        "name": urlencoding::decode(&get("name")).map(|s| s.into_owned()).unwrap_or_default(), "power": get("power"),
        "bright": n("bright"), "rgb": n("rgb"), "ct": n("ct"), "colorMode": n("color_mode"),
        "support": get("support").split_whitespace().collect::<Vec<_>>()
    }))
}

fn fold(s: &str) -> String {
    js::fold_lower(s).trim().to_string()
}

pub struct Lights {
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    pub bulbs: RefCell<Vec<J>>,
    last_scan: Cell<i64>,
    seq: Cell<u64>,
}

impl Lights {
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let bulbs = js::read_json(&file).map(|v| js::arr(&v)).unwrap_or_default();
        Self { file, log, bulbs: RefCell::new(bulbs), last_scan: Cell::new(0), seq: Cell::new(1) }
    }

    fn save(&self) {
        let _ = js::write_json(&self.file, &Value::Array(self.bulbs.borrow().clone()));
    }

    /// Découverte (≈ 2,5 s) : ampoules répondant sur le réseau local.
    pub async fn discover(&self) -> Vec<J> {
        let found: Vec<J> = async {
            let sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
            let msg = b"M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1982\r\nMAN: \"ssdp:discover\"\r\nST: wifi_bulb\r\n\r\n";
            let _ = sock.send_to(msg, "239.255.255.250:1982").await;
            let mut out: Vec<(String, J)> = vec![];
            let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
            let resend = tokio::time::Instant::now() + Duration::from_millis(400);
            let mut resent = false;
            let mut buf = vec![0u8; 4096];
            loop {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                if !resent && now >= resend {
                    resent = true;
                    let _ = sock.send_to(msg, "239.255.255.250:1982").await;
                }
                let wait = if resent { deadline - now } else { resend.saturating_duration_since(now).max(Duration::from_millis(1)) };
                if let Ok(Ok((n, _))) = tokio::time::timeout(wait, sock.recv_from(&mut buf)).await {
                    if let Some(b) = parse_ssdp(&String::from_utf8_lossy(&buf[..n])) {
                        let key = if js::gs(&b, "id").is_empty() { js::gs(&b, "ip") } else { js::gs(&b, "id") };
                        out.retain(|(k, _)| *k != key);
                        out.push((key, b));
                    }
                }
            }
            Some(out.into_iter().map(|(_, b)| b).collect())
        }
        .await
        .unwrap_or_default();
        if !found.is_empty() {
            let previous: HashMap<String, J> = self.bulbs.borrow().iter().map(|b| (if js::gs(b, "id").is_empty() { js::gs(b, "ip") } else { js::gs(b, "id") }, b.clone())).collect();
            let merged: Vec<J> = found
                .into_iter()
                .map(|mut b| {
                    if js::gs(&b, "name").is_empty() {
                        let key = if js::gs(&b, "id").is_empty() { js::gs(&b, "ip") } else { js::gs(&b, "id") };
                        b["name"] = json!(previous.get(&key).map(|p| js::gs(p, "name")).unwrap_or_default());
                    }
                    b
                })
                .collect();
            *self.bulbs.borrow_mut() = merged;
            self.save();
        }
        self.last_scan.set(js::now_ms());
        self.bulbs.borrow().clone()
    }

    pub async fn ensure_bulbs(&self) -> Vec<J> {
        if self.bulbs.borrow().is_empty() || js::now_ms() - self.last_scan.get() > 10 * 60000 {
            self.discover().await;
        }
        self.bulbs.borrow().clone()
    }

    /// Une commande JSON à une ampoule ; sa réponse (ou une erreur après 3 s).
    pub async fn send(&self, bulb: &J, method: &str, params: J) -> Result<J, String> {
        let id = self.seq.get();
        self.seq.set(id + 1);
        let ip = js::gs(bulb, "ip");
        let port = bulb["port"].as_u64().unwrap_or(55443) as u16;
        let run = async {
            let mut s = tokio::net::TcpStream::connect((ip.as_str(), port)).await.map_err(|e| e.to_string())?;
            s.write_all(format!("{}\r\n", json!({ "id": id, "method": method, "params": params })).as_bytes()).await.map_err(|e| e.to_string())?;
            let mut buf = String::new();
            let mut chunk = vec![0u8; 4096];
            loop {
                let n = s.read(&mut chunk).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("connexion fermée".to_string());
                }
                buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
                for line in buf.split("\r\n") {
                    let Ok(msg) = serde_json::from_str::<J>(line.trim()) else { continue };
                    if msg["id"].as_u64() != Some(id) {
                        continue;
                    }
                    if js::truthy(&msg["error"]) {
                        let m = js::gs(&msg["error"], "message");
                        return Err(if m.is_empty() { "refus de l’ampoule".into() } else { m });
                    }
                    return Ok(msg["result"].clone());
                }
            }
        };
        match tokio::time::timeout(Duration::from_secs(3), run).await {
            Ok(r) => r,
            Err(_) => Err(format!("ampoule {ip} muette")),
        }
    }

    /// Ampoules visées : celles dont le nom contient les mots dits, sinon toutes.
    pub fn pick(&self, which: &str) -> Vec<J> {
        let want = fold(which);
        let bulbs = self.bulbs.borrow();
        if want.is_empty() {
            return bulbs.clone();
        }
        bulbs.iter().filter(|b| fold(&js::gs(b, "name")).contains(&want)).cloned().collect()
    }

    /// État de chaque ampoule visée (avant une commande : pour « défais »).
    pub async fn snapshot(&self, which: &str) -> Vec<J> {
        self.ensure_bulbs().await;
        let mut out = vec![];
        for b in self.pick(which) {
            if let Ok(r) = self.send(&b, "get_prop", json!(["power", "bright", "ct", "rgb", "color_mode"])).await {
                let g = |i: usize| r[i].clone();
                let n = |i: usize| js::num(&r[i]).unwrap_or(0.0);
                out.push(json!({ "bulb": b, "power": g(0), "bright": n(1), "ct": n(2), "rgb": n(3), "colorMode": n(4) }));
            }
        }
        out
    }

    pub async fn restore(&self, states: &J) {
        for s in js::arr(states) {
            let b = &s["bulb"];
            let res: Result<(), String> = async {
                if js::gs(&s, "power") == "on" {
                    self.send(b, "set_power", json!(["on", "smooth", 300])).await?;
                    self.send(b, "set_bright", json!([js::num(&s["bright"]).unwrap_or(1.0).max(1.0), "smooth", 300])).await?;
                    if js::num(&s["colorMode"]) == Some(1.0) && js::truthy(&s["rgb"]) {
                        self.send(b, "set_rgb", json!([s["rgb"], "smooth", 300])).await?;
                    } else if js::truthy(&s["ct"]) {
                        self.send(b, "set_ct_abx", json!([s["ct"], "smooth", 300])).await?;
                    }
                } else {
                    self.send(b, "set_power", json!(["off", "smooth", 300])).await?;
                }
                Ok(())
            }
            .await;
            let _ = res;
        }
    }

    /// Commande générique (outil « lumiere ») : { action, valeur, lampe }.
    pub async fn command(&self, p: &J) -> J {
        let bulbs = self.ensure_bulbs().await;
        if bulbs.is_empty() {
            return json!({ "ok": false, "error": "aucune ampoule Yeelight trouvée", "message": "Je ne trouve aucune ampoule Yeelight sur le réseau. Vérifie que le « Contrôle LAN » est activé dans l'application Yeelight." });
        }
        let action = if js::gs(p, "action").is_empty() { "allumer".to_string() } else { js::gs(p, "action") };
        let valeur = p["valeur"].clone();
        let lampe = js::gs(p, "lampe");
        if action == "lister" {
            return json!({ "ok": true, "lampes": bulbs.iter().map(|b| json!({
                "nom": ([js::gs(b, "name"), js::gs(b, "model"), js::gs(b, "ip")].into_iter().find(|x| !x.is_empty()).unwrap_or_default()),
                "ip": b["ip"], "allumee": js::gs(b, "power") == "on"
            })).collect::<Vec<_>>() });
        }
        let targets = self.pick(&lampe);
        if targets.is_empty() {
            let names: Vec<String> = bulbs.iter().map(|b| if js::gs(b, "name").is_empty() { js::gs(b, "model") } else { js::gs(b, "name") }).collect();
            return json!({ "ok": false, "error": format!("aucune lampe « {lampe} »"), "message": format!("Je ne connais pas de lampe « {lampe} ». Tes lampes : {}.", names.join(", ")) });
        }
        let mut steps: Vec<(&str, J)> = vec![];
        let on = ("set_power", json!(["on", "smooth", 400]));
        match action.as_str() {
            "allumer" => steps.push(on.clone()),
            "eteindre" => steps.push(("set_power", json!(["off", "smooth", 400]))),
            "basculer" => steps.push(("toggle", json!([]))),
            "luminosite" => {
                let v = js::num(&valeur).filter(|v| *v != 0.0).unwrap_or(50.0).clamp(1.0, 100.0);
                steps.push(on.clone());
                steps.push(("set_bright", json!([v as i64, "smooth", 400])));
            }
            "plus" | "moins" => steps.push(("set_adjust", json!([if action == "plus" { "increase" } else { "decrease" }, "bright"]))),
            "couleur" => {
                let rgb = valeur.as_i64().or_else(|| color(&js::s(&valeur).to_lowercase()));
                let Some(rgb) = rgb else { return json!({ "ok": false, "error": format!("couleur inconnue : {}", js::s(&valeur)), "message": format!("Je ne connais pas la couleur {}.", js::s(&valeur)) }) };
                steps.push(on.clone());
                steps.push(("set_rgb", json!([rgb, "smooth", 400])));
            }
            "chaud" | "froid" => {
                steps.push(on.clone());
                steps.push(("set_ct_abx", json!([if action == "chaud" { 2700 } else { 6000 }, "smooth", 400])));
            }
            "ambiance" => {
                let key = fold(&js::s(&valeur));
                let Some(s) = SCENES.iter().find(|s| s.0 == key) else {
                    return json!({ "ok": false, "error": format!("ambiance inconnue : {}", js::s(&valeur)), "message": format!("Je connais les ambiances {}.", SCENES.iter().map(|s| s.1).collect::<Vec<_>>().join(", ")) });
                };
                steps.push(on.clone());
                steps.push(("set_bright", json!([s.2, "smooth", 400])));
                if s.4 != 0 {
                    steps.push(("set_rgb", json!([s.4, "smooth", 400])));
                } else {
                    steps.push(("set_ct_abx", json!([s.3, "smooth", 400])));
                }
            }
            _ => return json!({ "ok": false, "error": format!("action inconnue : {action}") }),
        }
        let mut n = 0;
        for b in &targets {
            let mut ok = true;
            for (m, prm) in &steps {
                if let Err(e) = self.send(b, m, prm.clone()).await {
                    (self.log)(format!("lumière {} : {e}", js::gs(b, "ip")));
                    ok = false;
                    break;
                }
            }
            if ok {
                n += 1;
            }
        }
        if n == 0 {
            return json!({ "ok": false, "error": "aucune ampoule n’a répondu", "message": "Les ampoules ne répondent pas. Vérifie le « Contrôle LAN » dans l'application Yeelight." });
        }
        json!({ "ok": true, "lampes": n })
    }
}
