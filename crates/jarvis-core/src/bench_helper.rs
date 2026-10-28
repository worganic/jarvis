// Aide des bancs d'essai du moteur vocal (tools/route-texts.js et le vocabulaire lu par
// tools/bench-stt.py en v3) :
//   jarvis-core bench-helper vocab  → vocabulaire réellement envoyé à la voix (JSON)
//   jarvis-core bench-helper route  → lit des phrases (une par ligne, JSON) sur l'entrée standard et
//                                     rend l'intention locale de chacune (null = envoi à l'IA).
// Aucune action n'est exécutée : un orchestrateur est monté (sans démarrage) sur une copie
// temporaire de la mémoire et du vocabulaire de l'utilisateur.

use std::io::{BufRead, Write};
use std::rc::Rc;

use serde_json::{json, Value};

use crate::orch::{OrchOptions, Orchestrator};

pub fn main(args: &[String]) -> i32 {
    std::env::set_var("JARVIS_TEST", "1");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        let tmp = std::env::temp_dir().join(format!("jarvis-bench-{}", std::process::id()));
        let _ = std::fs::create_dir_all(tmp.join("history"));
        let real = crate::paths::data_dir();
        for f in ["memory.json", "vocabulary.json", "zones.json", "macros.json"] {
            let _ = std::fs::copy(real.join(f), tmp.join(f));
        }
        let orch = Orchestrator::new(OrchOptions {
            workspace: tmp.join("workspace"),
            data_dir: tmp.clone(),
            models_dir: Some(crate::paths::models_dir()),
            emit: Rc::new(|_| {}),
            log: Rc::new(|_| {}),
            secret_box: None,
            python: None,
            agy_config_path: None,
            notes_dir: Some(tmp.join("notes")),
            meetings_dir: Some(tmp.join("meetings")),
        });
        let code = match args.first().map(String::as_str) {
            Some("vocab") => {
                println!("{}", orch.vocabulary.build());
                0
            }
            Some("route") => {
                let stdin = std::io::stdin();
                let out: Vec<Value> = stdin
                    .lock()
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.trim().is_empty())
                    .map(|line| {
                        let text: String = serde_json::from_str(&line).unwrap_or_default();
                        match orch.router.route(&text, true) {
                            Some(i) => json!(i.kind),
                            None => Value::Null,
                        }
                    })
                    .collect();
                let mut o = std::io::stdout();
                let _ = write!(o, "{}", Value::Array(out));
                let _ = o.flush();
                0
            }
            _ => {
                eprintln!("usage : jarvis-core bench-helper vocab|route");
                1
            }
        };
        let _ = std::fs::remove_dir_all(&tmp);
        code
    })
}
