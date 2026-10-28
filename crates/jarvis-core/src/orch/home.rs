// Maison connectée au-delà des ampoules Yeelight (lights.rs) : lampes Philips Hue (pont local, API v1 en HTTP), et prises
// Shelly (HTTP) ou TP-Link Kasa (TCP 9999). Tout reste sur le réseau local : seules les adresses privées sont acceptées.
// Pont Hue : association par le bouton du pont (Administration › Maison), le jeton reste dans data/maison.json.

use std::cell::RefCell;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::js::{self, J};

const MAX_PLUGS: usize = 20;
const KINDS: &[&str] = &["shelly", "kasa"];

fn fold(s: &str) -> String {
    crate::re!(r"[^a-z0-9]+").replace_all(&js::fold_lower(s), " ").trim().to_string()
}

/// Adresse du réseau local seulement (jamais un hôte d'Internet).
pub fn lan_ip(text: &str) -> Option<Ipv4Addr> {
    match text.trim().parse::<IpAddr>().ok()? {
        IpAddr::V4(ip) if ip.is_private() || ip.is_loopback() || ip.is_link_local() => Some(ip),
        _ => None,
    }
}

/// Chiffrement « autokey XOR » du protocole local des prises Kasa (clé initiale 171), avec l'en-tête de longueur.
pub fn kasa_encrypt(payload: &str) -> Vec<u8> {
    let mut key = 171u8;
    let mut out = (payload.len() as u32).to_be_bytes().to_vec();
    for b in payload.bytes() {
        key ^= b;
        out.push(key);
    }
    out
}

pub fn kasa_decrypt(data: &[u8]) -> String {
    let mut key = 171u8;
    let plain: Vec<u8> = data
        .iter()
        .map(|&c| {
            let p = key ^ c;
            key = c;
            p
        })
        .collect();
    String::from_utf8_lossy(&plain).into_owned()
}

/// Luminosité en pourcentage → échelle Hue (1 à 254).
pub fn hue_bri(percent: f64) -> i64 {
    ((percent.clamp(1.0, 100.0) / 100.0) * 254.0).round().max(1.0) as i64
}

pub struct Home {
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    cfg: RefCell<J>,
}

impl Home {
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let cfg = js::read_json(&file).filter(|v| v.is_object()).unwrap_or_else(|| json!({ "hue": null, "prises": [] }));
        Self { file, log, cfg: RefCell::new(cfg) }
    }

    fn save(&self) {
        let _ = js::write_json(&self.file, &self.cfg.borrow());
    }

    fn hue(&self) -> Option<(String, String)> {
        let c = self.cfg.borrow();
        let (ip, user) = (js::gs(&c["hue"], "ip"), js::gs(&c["hue"], "user"));
        if ip.is_empty() || user.is_empty() { None } else { Some((ip, user)) }
    }

    fn hue_lights(&self) -> Vec<J> {
        js::arr(&self.cfg.borrow()["hue"]["lampes"])
    }

    fn plugs(&self) -> Vec<J> {
        js::arr(&self.cfg.borrow()["prises"])
    }

    /// Noms d'appareils connus (repliés), pour reconnaître « allume la prise du salon » avant les lumières Yeelight.
    pub fn names(&self) -> Vec<String> {
        self.hue_lights().iter().chain(self.plugs().iter()).map(|d| fold(&js::gs(d, "nom"))).filter(|n| !n.is_empty()).collect()
    }

    /// État pour l'administration : jamais le jeton du pont.
    pub fn state(&self) -> J {
        let c = self.cfg.borrow();
        json!({
            "ok": true,
            "hue": { "ip": c["hue"]["ip"], "associe": !js::gs(&c["hue"], "user").is_empty(), "lampes": js::arr(&c["hue"]["lampes"]).iter().map(|l| l["nom"].clone()).collect::<Vec<_>>() },
            "prises": c["prises"]
        })
    }

    // --- Pont Hue -------------------------------------------------------------------------------------

    /// Association : le bouton du pont doit avoir été pressé dans les 30 secondes qui précèdent.
    pub async fn hue_pair(&self, ip: &str) -> Result<J, String> {
        let ip = lan_ip(ip).ok_or("Adresse du pont invalide : une adresse du réseau local est attendue (ex. 192.168.1.20).")?.to_string();
        let r = crate::net::http()
            .post(format!("http://{ip}/api"))
            .timeout(Duration::from_secs(6))
            .body(json!({ "devicetype": "jarvis#pc" }).to_string())
            .send()
            .await
            .map_err(|e| format!("Pont injoignable : {e}"))?;
        let v: J = r.json().await.map_err(|e| format!("Réponse du pont illisible : {e}"))?;
        let first = v.get(0).cloned().unwrap_or(Value::Null);
        if let Some(user) = first["success"]["username"].as_str() {
            self.cfg.borrow_mut()["hue"] = json!({ "ip": ip, "user": user, "lampes": [] });
            self.save();
            self.hue_refresh().await?;
            return Ok(self.state());
        }
        let msg = js::gs(&first["error"], "description");
        Err(if first["error"]["type"] == json!(101) { "Appuie sur le bouton du pont Hue, puis réessaie dans les 30 secondes.".into() } else if msg.is_empty() { "Association refusée.".into() } else { msg })
    }

    pub fn hue_forget(&self) -> J {
        self.cfg.borrow_mut()["hue"] = Value::Null;
        self.save();
        self.state()
    }

    /// Relit les lampes du pont.
    pub async fn hue_refresh(&self) -> Result<usize, String> {
        let (ip, user) = self.hue().ok_or("Pont Hue non associé.")?;
        let r = crate::net::http().get(format!("http://{ip}/api/{user}/lights")).timeout(Duration::from_secs(6)).send().await.map_err(|e| format!("Pont injoignable : {e}"))?;
        let v: J = r.json().await.map_err(|e| e.to_string())?;
        let lights: Vec<J> = v.as_object().map(|m| m.iter().map(|(id, l)| json!({ "id": id, "nom": l["name"], "allumee": l["state"]["on"] })).collect()).unwrap_or_default();
        if lights.is_empty() && v.get(0).is_some() {
            return Err("Le pont refuse l'accès : associe-le de nouveau.".into());
        }
        let n = lights.len();
        self.cfg.borrow_mut()["hue"]["lampes"] = json!(lights);
        self.save();
        Ok(n)
    }

    async fn hue_set(&self, id: &str, body: J) -> Result<(), String> {
        let (ip, user) = self.hue().ok_or("Pont Hue non associé.")?;
        let r = crate::net::http().put(format!("http://{ip}/api/{user}/lights/{id}/state")).timeout(Duration::from_secs(5)).body(body.to_string()).send().await.map_err(|e| format!("pont injoignable : {e}"))?;
        let v: J = r.json().await.map_err(|e| e.to_string())?;
        match v.as_array().and_then(|a| a.iter().find(|x| x.get("error").is_some())) {
            Some(e) => Err(js::gs(&e["error"], "description")),
            None => Ok(()),
        }
    }

    // --- Prises -----------------------------------------------------------------------------------------

    pub fn add_plug(&self, nom: &str, kind: &str, ip: &str) -> Result<J, String> {
        let nom = js::slice_chars(nom.trim(), 40);
        if nom.is_empty() {
            return Err("Donne un nom à la prise (ex. « cafetière »).".into());
        }
        if !KINDS.contains(&kind) {
            return Err("Type de prise inconnu (shelly ou kasa).".into());
        }
        let ip = lan_ip(ip).ok_or("Adresse invalide : une adresse du réseau local est attendue (ex. 192.168.1.30).")?.to_string();
        {
            let mut c = self.cfg.borrow_mut();
            let mut plugs = js::arr(&c["prises"]);
            if plugs.len() >= MAX_PLUGS {
                return Err(format!("{MAX_PLUGS} prises au plus."));
            }
            if plugs.iter().any(|p| fold(&js::gs(p, "nom")) == fold(&nom)) {
                return Err("Une prise porte déjà ce nom.".into());
            }
            plugs.push(json!({ "nom": nom, "type": kind, "ip": ip }));
            c["prises"] = json!(plugs);
        }
        self.save();
        Ok(self.state())
    }

    pub fn remove_plug(&self, nom: &str) -> J {
        {
            let mut c = self.cfg.borrow_mut();
            let keep: Vec<J> = js::arr(&c["prises"]).into_iter().filter(|p| fold(&js::gs(p, "nom")) != fold(nom)).collect();
            c["prises"] = json!(keep);
        }
        self.save();
        self.state()
    }

    async fn plug_set(&self, plug: &J, on: Option<bool>) -> Result<(), String> {
        let ip = js::gs(plug, "ip");
        let ip = lan_ip(&ip).ok_or("adresse de prise invalide")?.to_string();
        let http = crate::net::http();
        match js::gs(plug, "type").as_str() {
            "shelly" => {
                let turn = match on { Some(true) => "on", Some(false) => "off", None => "toggle" };
                // Génération 1 (/relay/0), sinon génération 2 et plus (RPC).
                let g1 = http.get(format!("http://{ip}/relay/0?turn={turn}")).timeout(Duration::from_secs(4)).send().await.map_err(|e| format!("prise injoignable : {e}"))?;
                if g1.status().is_success() {
                    return Ok(());
                }
                let url = match on {
                    Some(v) => format!("http://{ip}/rpc/Switch.Set?id=0&on={v}"),
                    None => format!("http://{ip}/rpc/Switch.Toggle?id=0"),
                };
                let g2 = http.get(url).timeout(Duration::from_secs(4)).send().await.map_err(|e| format!("prise injoignable : {e}"))?;
                if g2.status().is_success() { Ok(()) } else { Err("la prise refuse la commande".into()) }
            }
            "kasa" => {
                let state = match on {
                    Some(v) => v,
                    None => {
                        let info = Self::kasa_call(&ip, r#"{"system":{"get_sysinfo":{}}}"#).await?;
                        info["system"]["get_sysinfo"]["relay_state"].as_i64() != Some(1)
                    }
                };
                let r = Self::kasa_call(&ip, &format!(r#"{{"system":{{"set_relay_state":{{"state":{}}}}}}}"#, state as i32)).await?;
                if r["system"]["set_relay_state"]["err_code"].as_i64().unwrap_or(0) == 0 { Ok(()) } else { Err("la prise refuse la commande".into()) }
            }
            other => Err(format!("type de prise inconnu : {other}")),
        }
    }

    async fn kasa_call(ip: &str, payload: &str) -> Result<J, String> {
        let run = async {
            let mut s = tokio::net::TcpStream::connect((ip, 9999)).await.map_err(|e| format!("prise injoignable : {e}"))?;
            s.write_all(&kasa_encrypt(payload)).await.map_err(|e| e.to_string())?;
            let mut len = [0u8; 4];
            s.read_exact(&mut len).await.map_err(|e| e.to_string())?;
            let n = (u32::from_be_bytes(len) as usize).min(16 * 1024);
            let mut buf = vec![0u8; n];
            s.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            serde_json::from_str::<J>(&kasa_decrypt(&buf)).map_err(|e| e.to_string())
        };
        tokio::time::timeout(Duration::from_secs(4), run).await.map_err(|_| "prise muette".to_string())?
    }

    // --- Commande (outil « maison ») ----------------------------------------------------------------

    /// { action : allumer | eteindre | basculer | luminosite | lister, nom, valeur }.
    pub async fn command(&self, p: &J) -> J {
        let action = if js::gs(p, "action").is_empty() { "allumer".to_string() } else { js::gs(p, "action") };
        let want = fold(&js::gs(p, "nom"));
        let lights = self.hue_lights();
        let plugs = self.plugs();
        if action == "lister" {
            if self.hue().is_some() {
                let _ = self.hue_refresh().await;
            }
            return json!({ "ok": true, "appareils": self.hue_lights().iter().map(|l| json!({ "nom": l["nom"], "genre": "lampe Hue", "allume": l["allumee"] }))
                .chain(plugs.iter().map(|p| json!({ "nom": p["nom"], "genre": format!("prise {}", js::gs(p, "type")) }))).collect::<Vec<_>>() });
        }
        let pick = |d: &&J| !want.is_empty() && (fold(&js::gs(d, "nom")) == want || want.contains(&fold(&js::gs(d, "nom"))) || fold(&js::gs(d, "nom")).contains(&want));
        let l_hit: Vec<&J> = lights.iter().filter(pick).collect();
        let p_hit: Vec<&J> = plugs.iter().filter(pick).collect();
        if l_hit.is_empty() && p_hit.is_empty() {
            return json!({ "ok": false, "error": "appareil inconnu", "message": format!("Je ne connais pas d'appareil « {} » dans la maison connectée.", js::gs(p, "nom")) });
        }
        let on = match action.as_str() { "allumer" | "luminosite" => Some(true), "eteindre" => Some(false), _ => None };
        let mut errors: Vec<String> = vec![];
        let mut done = 0;
        for l in &l_hit {
            let id = js::gs(l, "id");
            let body = match action.as_str() {
                "luminosite" => json!({ "on": true, "bri": hue_bri(js::num(&p["valeur"]).unwrap_or(50.0)) }),
                "basculer" => json!({ "on": !js::gb(l, "allumee") }),
                _ => json!({ "on": on.unwrap_or(true) }),
            };
            match self.hue_set(&id, body).await {
                Ok(()) => done += 1,
                Err(e) => errors.push(format!("{} : {e}", js::gs(l, "nom"))),
            }
        }
        for pl in &p_hit {
            if action == "luminosite" {
                errors.push(format!("{} : une prise n'a pas de luminosité", js::gs(pl, "nom")));
                continue;
            }
            match self.plug_set(pl, on).await {
                Ok(()) => done += 1,
                Err(e) => errors.push(format!("{} : {e}", js::gs(pl, "nom"))),
            }
        }
        if done == 0 {
            (self.log)(format!("Maison : {}", errors.join(" ; ")));
            return json!({ "ok": false, "error": errors.join(" ; "), "message": format!("Je n'ai pas pu commander l'appareil : {}.", errors.join(" ; ")) });
        }
        if self.hue().is_some() && !l_hit.is_empty() {
            let _ = self.hue_refresh().await;
        }
        json!({ "ok": true, "commandes": done, "erreurs": errors })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adresses_du_reseau_local_seulement() {
        assert!(lan_ip("192.168.1.20").is_some());
        assert!(lan_ip("10.0.0.5").is_some());
        assert!(lan_ip("127.0.0.1").is_some());
        assert!(lan_ip("8.8.8.8").is_none());
        assert!(lan_ip("exemple.com").is_none());
        assert!(lan_ip("::1").is_none());
    }

    #[test]
    fn chiffrement_kasa_aller_retour() {
        let msg = r#"{"system":{"set_relay_state":{"state":1}}}"#;
        let enc = kasa_encrypt(msg);
        assert_eq!(&enc[..4], &(msg.len() as u32).to_be_bytes());
        // Valeur connue du protocole : le « { » initial donne 0xD0.
        assert_eq!(enc[4], 0xd0);
        assert_eq!(kasa_decrypt(&enc[4..]), msg);
    }

    #[test]
    fn luminosite_hue() {
        assert_eq!(hue_bri(100.0), 254);
        assert_eq!(hue_bri(0.0), 3);
        assert_eq!(hue_bri(50.0), 127);
    }

    #[test]
    fn prises_nommees_et_retrouvees() {
        let file = std::env::temp_dir().join(format!("jarvis-test-maison-{}.json", js::now_ms()));
        let h = Home::new(file.clone(), Rc::new(|_| {}));
        assert!(h.add_plug("Cafetière", "shelly", "192.168.1.30").is_ok());
        assert!(h.add_plug("cafetiere", "shelly", "192.168.1.31").is_err(), "nom déjà pris");
        assert!(h.add_plug("Externe", "shelly", "8.8.8.8").is_err());
        assert!(h.add_plug("X", "zigbee", "192.168.1.32").is_err());
        assert_eq!(h.names(), vec!["cafetiere".to_string()]);
        let again = Home::new(file.clone(), Rc::new(|_| {}));
        assert_eq!(again.names(), vec!["cafetiere".to_string()], "relu depuis le disque");
        assert!(h.remove_plug("CAFETIERE")["prises"].as_array().unwrap().is_empty());
        let _ = std::fs::remove_file(file);
    }
}
