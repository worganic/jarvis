// Diffusion des événements du moteur vers le noyau (Hub de voice/service.py), depuis n'importe quel
// fil : les messages passent par un canal lu par le serveur WebSocket.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::util::J;

struct Inner {
    state: Mutex<J>,
    tx: broadcast::Sender<String>,
    drained: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

#[derive(Clone)]
pub struct Hub(Arc<Inner>);

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self(Arc::new(Inner {
            state: Mutex::new(json!({ "t": "state", "ready": false, "capturing": false, "speaking": false, "muted": false })),
            tx,
            drained: Mutex::new(None),
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.0.tx.subscribe()
    }

    pub fn emit(&self, obj: J) {
        let _ = self.0.tx.send(obj.to_string());
    }

    pub fn emit_state(&self, patch: J) {
        let snapshot = {
            let mut st = self.0.state.lock().unwrap();
            if let (Some(s), Some(p)) = (st.as_object_mut(), patch.as_object()) {
                for (k, v) in p {
                    s.insert(k.clone(), v.clone());
                }
            }
            st.clone()
        };
        self.emit(snapshot);
    }

    pub fn state(&self) -> J {
        self.0.state.lock().unwrap().clone()
    }

    pub fn state_flag(&self, key: &str) -> bool {
        self.0.state.lock().unwrap().get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    /// La sortie audio a vidé sa file (appelé par le flux de sortie).
    pub fn signal_drained(&self) {
        if let Some(f) = self.0.drained.lock().unwrap().as_ref() {
            f();
        }
    }

    pub fn on_drained(&self, f: Box<dyn Fn() + Send + Sync>) {
        *self.0.drained.lock().unwrap() = Some(f);
    }
}
