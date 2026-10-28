// Applications en arrière-plan masquées dans la card Bureau (data/bureau-masques.json) : un bouton les
// retire de la liste, un autre (👁) les réaffiche pour pouvoir les remettre.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::json;

use crate::js;

pub struct BgHidden {
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    hidden: RefCell<Vec<String>>,
    /// Les applications masquées sont affichées (grisées) pour pouvoir être remises.
    pub showing: Cell<bool>,
}

impl BgHidden {
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let hidden = js::read_json(&file).map(|d| js::arr(&d["hidden"]).iter().map(|v| js::s(v)).collect()).unwrap_or_default();
        Self { file, log, hidden: RefCell::new(hidden), showing: Cell::new(false) }
    }

    fn save(&self) {
        if let Err(e) = js::write_json_atomic(&self.file, &json!({ "hidden": *self.hidden.borrow() })) {
            (self.log)(format!("[bureau] écriture impossible : {e}"));
        }
    }

    pub fn is_hidden(&self, app: &str) -> bool {
        let a = app.to_lowercase();
        self.hidden.borrow().iter().any(|h| h.to_lowercase() == a)
    }

    pub fn count(&self) -> usize {
        self.hidden.borrow().len()
    }

    pub fn set(&self, app: &str, hide: bool) {
        {
            let mut h = self.hidden.borrow_mut();
            let a = app.to_lowercase();
            h.retain(|x| x.to_lowercase() != a);
            if hide {
                h.push(app.to_string());
            }
        }
        if self.hidden.borrow().is_empty() {
            self.showing.set(false);
        }
        self.save();
    }
}
