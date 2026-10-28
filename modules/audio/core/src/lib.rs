// Module Audio (partie noyau) : volumes, sourdines et périphériques par défaut de Windows, niveaux en direct.
// La card (modules/audio/ui/audio.js) appelle les méthodes déclarées ici ; le mélangeur tourne dans un
// processus à part (`jarvis-core audio-mixer serve`), lancé seulement pendant que la card est affichée.

mod mixer;

use std::cell::RefCell;
use std::rc::Rc;

use jarvis_core::js::{self, J};
use jarvis_core::{Module, ModuleCtx};
use serde_json::json;

use mixer::AudioMixer;

#[derive(Default)]
pub struct Audio {
    mixer: RefCell<Option<Rc<AudioMixer>>>,
}

pub fn module() -> Rc<dyn Module> {
    Rc::new(Audio::default())
}

fn arg(a: &[J], i: usize) -> J {
    a.get(i).cloned().unwrap_or(J::Null)
}

/// Réponse du mélangeur, complétée du volume propre de la voix de Jarvis (affiché dans la même card).
fn with_jarvis(ctx: &ModuleCtx, mut res: J) -> Result<J, String> {
    if res.is_object() {
        if let Some(core) = ctx.core() {
            res["jarvisVolume"] = json!(core.jarvis_volume());
        }
    }
    Ok(res)
}

impl Module for Audio {
    fn id(&self) -> &'static str {
        "audio"
    }

    fn subcommand(&self, cmd: &str, _args: &[String]) -> Option<i32> {
        (cmd == "audio-mixer").then(jarvis_native::audio::mixer_serve)
    }

    fn start(&self, ctx: Rc<ModuleCtx>) {
        let c = ctx.clone();
        let mixer = AudioMixer::new(
            Rc::new(move |ev: J| c.broadcast(&ev, Some(&["hud"]))),
            {
                let c = ctx.clone();
                Rc::new(move |m: String| c.log(&m))
            },
        );
        *self.mixer.borrow_mut() = Some(mixer.clone());
        const HUD: &[&str] = &["hud"];

        let (m, c) = (mixer.clone(), ctx.clone());
        ctx.register("audioWatch", HUD, move |a, _| {
            let (m, c) = (m.clone(), c.clone());
            Box::pin(async move {
                let r = m.watch(arg(&a, 0) == json!(true), arg(&a, 1) == json!(true)).await;
                with_jarvis(&c, r)
            })
        });
        let (m, c) = (mixer.clone(), ctx.clone());
        ctx.register("audioVolume", HUD, move |a, _| {
            let (m, c) = (m.clone(), c.clone());
            Box::pin(async move {
                let r = m.set_volume(&js::s(&arg(&a, 0)), &arg(&a, 1)).await;
                with_jarvis(&c, r)
            })
        });
        let (m, c) = (mixer.clone(), ctx.clone());
        ctx.register("audioMute", HUD, move |a, _| {
            let (m, c) = (m.clone(), c.clone());
            Box::pin(async move {
                let on = if js::s(&arg(&a, 1)) == "toggle" { json!("toggle") } else { json!(arg(&a, 1) == json!(true)) };
                let r = m.set_mute(&js::s(&arg(&a, 0)), &on).await;
                with_jarvis(&c, r)
            })
        });
        // Card musique : niveaux et volume de la seule application qui joue (Spotify, lecteur de Jarvis…).
        let m = mixer.clone();
        ctx.register("audioApp", HUD, move |a, _| {
            let m = m.clone();
            Box::pin(async move { Ok(m.app(&js::s(&arg(&a, 0))).await) })
        });
        let m = mixer.clone();
        ctx.register("audioAppVolume", HUD, move |a, _| {
            let m = m.clone();
            Box::pin(async move { Ok(m.app_volume(&arg(&a, 0)).await) })
        });
        let (m, c) = (mixer, ctx.clone());
        ctx.register("audioDefault", HUD, move |a, _| {
            let (m, c) = (m.clone(), c.clone());
            Box::pin(async move {
                let res = m.set_default(&js::s(&arg(&a, 0))).await;
                if js::gb(&res, "ok") {
                    if let Some(core) = c.core() {
                        // Nouvelle sortie ou nouveau micro : le moteur vocal rouvre ses flux (une seule fois).
                        core.audio_device_changed().await;
                        core.record(
                            "peripherique_audio_change",
                            json!({ "peripherique": js::gs(&res["out"], "name"), "micro": js::gs(&res["in"], "name"), "origine": "card audio" }),
                        );
                    }
                }
                with_jarvis(&c, res)
            })
        });
    }

    fn stop(&self) {
        if let Some(m) = self.mixer.borrow_mut().take() {
            m.stop();
        }
    }
}
