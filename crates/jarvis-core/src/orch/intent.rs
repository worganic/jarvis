// Intention locale (sans IA) rendue par le routeur et les modules : { kind, label, detail, execute }
// de la v3. `execute` rend { say, display, … } ; une intention « ignored » (bruit, fragment) ou
// « quiet » (politesse) n'affiche pas de card d'action.

use std::future::Future;
use std::rc::Rc;

use serde_json::Value;

use super::Fut;
use crate::js::J;

#[derive(Clone)]
pub struct Intent {
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub quiet: bool,
    pub ignored: bool,
    pub reason: String,
    /// Reprend un texte libre de la phrase : jamais réutilisée depuis une transcription partielle.
    pub free_text: bool,
    pub composite: bool,
    /// Intentions déjà couvertes par celle-ci (« dessine un mouton » ouvre déjà Paint).
    pub covers: Vec<String>,
    pub execute: Rc<dyn Fn() -> Fut<J>>,
}

impl Intent {
    pub fn new<F, Fu>(kind: &str, label: &str, detail: &str, f: F) -> Intent
    where
        F: Fn() -> Fu + 'static,
        Fu: Future<Output = J> + 'static,
    {
        Intent {
            kind: kind.to_string(),
            label: label.to_string(),
            detail: detail.to_string(),
            quiet: false,
            ignored: false,
            reason: String::new(),
            free_text: false,
            composite: false,
            covers: vec![],
            execute: Rc::new(move || Box::pin(f())),
        }
    }

    /// Intention qui rend immédiatement un résultat.
    pub fn now(kind: &str, label: &str, detail: &str, result: J) -> Intent {
        Intent::new(kind, label, detail, move || {
            let r = result.clone();
            async move { r }
        })
    }

    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    pub fn ignored(mut self, reason: &str) -> Self {
        self.ignored = true;
        self.quiet = true;
        self.reason = reason.to_string();
        self
    }

    pub fn free_text(mut self) -> Self {
        self.free_text = true;
        self
    }

    pub async fn run(&self) -> J {
        let r = (self.execute)().await;
        if r.is_null() {
            Value::Object(Default::default())
        } else {
            r
        }
    }
}
