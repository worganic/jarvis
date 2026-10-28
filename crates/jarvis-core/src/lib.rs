// Noyau de Jarvis v4 (core/main.js de la v3), en bibliothèque : le binaire jarvis-core.exe (crates/jarvis-app)
// l'appelle avec la liste des modules liés (crates/jarvis-modules). Un module (modules/<id>/) peut ainsi
// dépendre du noyau sans que le noyau dépende de lui.

#![allow(dead_code)]

#[macro_use]
pub mod js;
pub mod app;
pub mod backup;
pub mod build_sweep;
pub mod bench_helper;
pub mod changelog;
pub mod code_watch;
pub mod gateway;
pub mod instance_lock;
pub mod logger;
pub mod metrics;
pub mod migrate;
pub mod mcp;
pub mod module_api;
pub mod module_studio;
pub mod modules;
pub mod net;
pub mod orch;
pub mod paths;
pub mod proc;
pub mod relaunch;
pub mod secret_box;
pub mod transfer;
pub mod update;
pub mod voice_link;
pub mod voice_supervisor;

use std::rc::Rc;

pub use module_api::{CoreAccess, Module, ModuleCtx};

/// Point d'entrée de jarvis-core.exe : sous-commandes (outils natifs, MCP…), sinon le noyau.
pub fn run(modules: Vec<Rc<dyn Module>>) -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Sous-commandes propres à un module (ex. « audio-mixer serve » du module Audio).
    if let Some(cmd) = args.first() {
        for m in &modules {
            if let Some(code) = m.subcommand(cmd, &args[1..]) {
                return code;
            }
        }
    }
    match args.first().map(String::as_str) {
        Some("mcp") => return mcp::main(&args[1..]),
        Some("mcp-proxy") => return mcp::proxy::main(&args[1..]),
        // Outils natifs (tools/*.exe de la v3), lancés par le noyau lui-même.
        Some("desktop-watch") => return jarvis_native::desktop::serve(),
        Some("media-control") => return jarvis_native::media::main(args.get(1).map(String::as_str).unwrap_or("status"), args.get(2).map(String::as_str)),
        Some("active-win") if args.iter().any(|a| a == "--watch") => return jarvis_native::window::watch_foreground(),
        Some("active-win") => {
            println!("{}", jarvis_native::window::foreground());
            return 0;
        }
        // Onglets de Windows Terminal : fenêtre hôte et couleur (processus à part : il se rattache aux consoles).
        Some("console-tabs") => return jarvis_native::console::main(&args[1..]),
        Some("migrate") => return migrate::main(&args[1..]),
        Some("bench-helper") => return bench_helper::main(&args[1..]),
        Some("relaunch") => return relaunch::main(&args[1..]),
        Some("restaurer") => return backup::main(&args[1..]),
        Some("donnees-migrer") => return paths::relocate_main(),
        Some("donnees") => return transfer::main(&args[1..]),
        _ => {}
    }
    app::main(modules)
}
