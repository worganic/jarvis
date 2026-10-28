// Compétences à déposer (orchestrator/skills.js) : chaque fichier .js du dossier skills/ devient une
// commande locale (sans IA), rechargée à chaud, listée dans l'aide. Le format de la v3 est gardé tel
// quel (module.exports = { name, examples, match, run }) : les fichiers sont exécutés par un moteur
// JavaScript embarqué (boa, 100 % Rust). `run` reçoit { text, raw, match, tool, System, orch } :
//   - System.<action>(p) → actions Windows de Jarvis (orch/system.rs), en promesse ;
//   - tool(nom, params, exec) → exécution sous la politique de sécurité (journal, code à 4 chiffres) ;
//   - orch.config.get/update, orch.record, orch.emit, orch.notify ;
//   - setTimeout ; pas de require() (aucun module Node).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use boa_engine::{js_string, Context, JsString, JsValue, Source};
use serde_json::{json, Value};

use super::intent::Intent;
use super::{Fut, OrchRef, Up};
use crate::js::{self, Timer, J};

const PRELUDE: &str = r#"
var __reqs = [], __waiters = {}, __nid = 0, __tasks = {}, __tid = 0, __execs = {}, __eid = 0;
var __skills = [], __matches = {}, __mid = 0, __config = {};
function __call(op, args) { return new Promise(function (res, rej) { var id = ++__nid; __waiters[id] = { res: res, rej: rej }; __reqs.push({ id: id, op: op, args: args === undefined ? null : args }); }); }
function __post(op, args) { __reqs.push({ id: 0, op: op, args: args === undefined ? null : args }); }
function __take() { var r = JSON.stringify(__reqs); __reqs = []; return r; }
function __settle(id, ok, json) { var w = __waiters[id]; if (!w) return; delete __waiters[id]; var v = JSON.parse(json); if (ok) w.res(v); else w.rej(new Error(String(v))); }
function __start(p) {
  var id = ++__tid; __tasks[id] = { state: 'pending' };
  Promise.resolve(p).then(function (v) { __tasks[id] = { state: 'ok', value: v === undefined ? null : v }; },
    function (e) { __tasks[id] = { state: 'err', value: String((e && e.message) || e) }; });
  return id;
}
function __task(id) { var t = __tasks[id] || { state: 'err', value: 'tâche inconnue' }; if (t.state !== 'pending') delete __tasks[id]; return JSON.stringify(t); }
function setTimeout(f, ms) { __call('sleep', Number(ms) || 0).then(function () { f(); }); return 0; }
var System = new Proxy({}, { get: function (_, action) { return function (p) { return __call('system', { action: String(action), p: p || {} }); }; } });
var orch = {
  config: {
    get: function (k) { return k ? __config[k] : __config; },
    update: function (p) { __post('config_update', p); for (var k in p) __config[k] = Object.assign({}, __config[k] || {}, p[k]); return __config; }
  },
  record: function (t, d) { __post('record', { type: t, data: d }); },
  emit: function (e) { __post('emit', e); },
  notify: function (n) { __post('notify', n); }
};
function tool(name, params, exec) { var id = ++__eid; __execs[id] = exec; return __call('tool', { name: name, params: params || {}, exec: id }); }
function __runExec(id, json) { var f = __execs[id]; delete __execs[id]; var p = JSON.parse(json); return __start(Promise.resolve().then(function () { return f ? f(p) : null; })); }
function __reset() { __skills = []; __matches = {}; }
function __load(file, code) {
  var module = { exports: {} };
  (new Function('module', 'exports', 'require', code))(module, module.exports, function (n) { throw new Error('require(' + n + ') indisponible dans Jarvis 4'); });
  var s = module.exports;
  if (!s || typeof s.run !== 'function' || !(s.match instanceof RegExp || typeof s.match === 'function')) throw new Error('il faut exporter { name, match (RegExp ou fonction), run }');
  __skills.push({ file: file, s: s });
  return JSON.stringify({ name: String(s.name || ''), examples: [].concat(s.examples || []).map(String) });
}
function __match(i, t, raw) {
  var s = __skills[i].s, m = null;
  try { m = s.match instanceof RegExp ? t.match(s.match) : s.match(t, raw); } catch (e) { m = null; }
  if (!m) return 0;
  __matches[++__mid] = m;
  return __mid;
}
function __run(i, mid, t, raw, config) {
  var s = __skills[i].s, m = __matches[mid]; delete __matches[mid];
  __config = JSON.parse(config);
  return __start(Promise.resolve().then(function () { return s.run({ text: t, raw: raw, match: m, tool: tool, System: System, orch: orch }); }));
}
"#;

#[derive(Clone)]
pub struct SkillItem {
    pub file: PathBuf,
    pub name: String,
    pub examples: Vec<String>,
}

pub struct Skills {
    o: OrchRef,
    pub dir: PathBuf,
    log: Rc<dyn Fn(String)>,
    ctx: RefCell<Context>,
    pub items: RefCell<Vec<SkillItem>>,
    pub errors: RefCell<Vec<(PathBuf, String)>>,
    watcher: RefCell<Option<notify::RecommendedWatcher>>,
    reload: Timer,
    dirty: Rc<Cell<bool>>,
}

fn jstr(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

fn call_fn(ctx: &mut Context, name: &str, args: &[JsValue]) -> Result<JsValue, String> {
    let f = ctx.global_object().get(JsString::from(name), ctx).map_err(|e| e.to_string())?;
    let f = f.as_callable().ok_or_else(|| format!("{name} absent"))?.clone();
    f.call(&JsValue::undefined(), args, ctx).map_err(|e| e.to_string())
}

fn as_text(v: &JsValue) -> String {
    v.as_string().map(|s| s.to_std_string_escaped()).unwrap_or_default()
}

fn new_context() -> Context {
    let mut ctx = Context::default();
    if let Err(e) = ctx.eval(Source::from_bytes(PRELUDE)) {
        crate::logger::logger().error("competences", &format!("prélude JS : {e}"));
    }
    ctx
}

impl Skills {
    pub fn new(o: OrchRef, dir: PathBuf, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let me = Rc::new(Self {
            o,
            dir,
            log,
            ctx: RefCell::new(new_context()),
            items: RefCell::new(vec![]),
            errors: RefCell::new(vec![]),
            watcher: RefCell::new(None),
            reload: Timer::new(),
            dirty: Rc::new(Cell::new(false)),
        });
        me.load();
        me
    }

    pub fn load(&self) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        let n = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        n.to_lowercase().ends_with(".js") && !n.starts_with('_')
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let mut items = vec![];
        let mut errors = vec![];
        // Contexte neuf à chaque chargement : une compétence retirée ne laisse rien derrière elle.
        let mut ctx = new_context();
        let _ = call_fn(&mut ctx, "__reset", &[]);
        for file in files {
            let code = match std::fs::read_to_string(&file) {
                Ok(c) => c,
                Err(e) => {
                    errors.push((file.clone(), e.to_string()));
                    continue;
                }
            };
            let fname = file.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            match call_fn(&mut ctx, "__load", &[jstr(&file.to_string_lossy()), jstr(&code)]) {
                Ok(v) => {
                    let info: J = serde_json::from_str(&as_text(&v)).unwrap_or(Value::Null);
                    let stem = Path::new(&fname).file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                    let name = js::gs(&info, "name");
                    items.push(SkillItem { file: file.clone(), name: if name.is_empty() { stem } else { name }, examples: js::str_list(&info["examples"]) });
                }
                Err(e) => {
                    (self.log)(format!("compétence {fname} ignorée : {e}"));
                    errors.push((file.clone(), e));
                }
            }
        }
        *self.ctx.borrow_mut() = ctx;
        *self.items.borrow_mut() = items;
        *self.errors.borrow_mut() = errors;
    }

    /// Rechargement à chaud (surveillance du dossier, regroupée sur 800 ms).
    pub fn watch(self: &Rc<Self>) {
        use notify::{RecursiveMode, Watcher};
        let _ = std::fs::create_dir_all(&self.dir);
        let dirty = self.dirty.clone();
        let flag = Rc::downgrade(&dirty);
        drop(flag);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let watcher = notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
            let _ = tx.send(());
        });
        match watcher {
            Ok(mut w) => {
                if let Err(e) = w.watch(&self.dir, RecursiveMode::NonRecursive) {
                    (self.log)(format!("compétences : surveillance impossible ({e})"));
                    return;
                }
                *self.watcher.borrow_mut() = Some(w);
            }
            Err(e) => {
                (self.log)(format!("compétences : surveillance impossible ({e})"));
                return;
            }
        }
        let me = Rc::downgrade(self);
        js::spawn(async move {
            while rx.recv().await.is_some() {
                let Some(s) = me.upgrade() else { break };
                let me2 = me.clone();
                s.reload.set(800, move || {
                    if let Some(s) = me2.upgrade() {
                        let before = s.items.borrow().len();
                        s.load();
                        (s.log)(format!("compétences rechargées : {} (avant : {before})", s.items.borrow().len()));
                    }
                });
            }
        });
    }

    pub fn stop(&self) {
        self.watcher.borrow_mut().take();
        self.reload.clear();
    }

    /// Intention du routeur pour une phrase repliée, ou None.
    pub fn match_(self: &Rc<Self>, t: &str, raw: &str) -> Option<Intent> {
        let items = self.items.borrow().clone();
        for (i, s) in items.iter().enumerate() {
            let mid = {
                let mut ctx = self.ctx.borrow_mut();
                call_fn(&mut ctx, "__match", &[JsValue::from(i as i32), jstr(t), jstr(raw)]).ok().and_then(|v| v.as_number()).unwrap_or(0.0) as i64
            };
            if mid <= 0 {
                continue;
            }
            let me = self.clone();
            let (t, raw, name) = (t.to_string(), raw.to_string(), s.name.clone());
            let fname = s.file.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let fname2 = fname.clone();
            return Some(Intent::new("skill_user", &s.name, &fname, move || {
                let (me, t, raw, name, fname) = (me.clone(), t.clone(), raw.clone(), name.clone(), fname2.clone());
                async move { me.run(i, mid, &t, &raw, &name, &fname).await }
            }));
        }
        None
    }

    async fn run(self: &Rc<Self>, i: usize, mid: i64, t: &str, raw: &str, name: &str, fname: &str) -> J {
        let o = self.o.up();
        o.record("competence", json!({ "nom": name, "fichier": fname, "texte": raw }));
        let config = o.config.all();
        let task = {
            let mut ctx = self.ctx.borrow_mut();
            call_fn(&mut ctx, "__run", &[JsValue::from(i as i32), JsValue::from(mid as f64), jstr(t), jstr(raw), jstr(&config.to_string())])
        };
        let out = match task {
            Ok(id) => self.drive(id.as_number().unwrap_or(0.0)).await,
            Err(e) => Err(e),
        };
        match out {
            Ok(v) if v.is_object() => json!({ "say": js::gs(&v, "say"), "display": if js::truthy(&v["display"]) { js::gs(&v, "display") } else { name.to_string() } }),
            Ok(v) => json!({ "say": if js::truthy(&v) { js::s(&v) } else { String::new() }, "display": name }),
            Err(e) => {
                (self.log)(format!("compétence {name} : {e}"));
                json!({ "say": format!("La compétence {name} a rencontré une erreur."), "display": "Erreur de compétence" })
            }
        }
    }

    /// Fait avancer une tâche JS jusqu'à son résultat, en servant ses demandes (System, tool, minuteries).
    fn drive(self: &Rc<Self>, task: f64) -> Fut<Result<J, String>> {
        let me = self.clone();
        Box::pin(async move {
            for _ in 0..10_000 {
                let (state, reqs) = {
                    let mut ctx = me.ctx.borrow_mut();
                    let _ = ctx.run_jobs();
                    let reqs = call_fn(&mut ctx, "__take", &[]).map(|v| as_text(&v)).unwrap_or_else(|_| "[]".into());
                    let state = call_fn(&mut ctx, "__task", &[JsValue::from(task)]).map(|v| as_text(&v)).unwrap_or_default();
                    (state, reqs)
                };
                let reqs: Vec<J> = serde_json::from_str(&reqs).unwrap_or_default();
                let state: J = serde_json::from_str(&state).unwrap_or(json!({ "state": "err", "value": "état illisible" }));
                let pending = js::gs(&state, "state") == "pending";
                if pending && reqs.is_empty() {
                    return Err("la compétence attend une promesse qui ne se résout jamais".into());
                }
                for r in reqs {
                    me.serve(r).await;
                }
                match js::gs(&state, "state").as_str() {
                    "ok" => return Ok(state["value"].clone()),
                    "err" => return Err(js::gs(&state, "value")),
                    _ => {}
                }
            }
            Err("compétence trop longue".into())
        })
    }

    async fn serve(self: &Rc<Self>, r: J) {
        let id = r["id"].as_i64().unwrap_or(0);
        let a = &r["args"];
        let o = self.o.up();
        let res: Result<J, String> = match js::gs(&r, "op").as_str() {
            "sleep" => {
                js::sleep_ms(js::num(a).unwrap_or(0.0).clamp(0.0, 600_000.0) as u64).await;
                Ok(Value::Null)
            }
            "system" => {
                let action = js::gs(a, "action");
                match crate::orch::system::call(&action, &a["p"]).await {
                    Some(r) => r,
                    None => Err(format!("System.{action} n'existe pas")),
                }
            }
            "tool" => {
                let exec = a["exec"].as_f64().unwrap_or(0.0);
                let me = self.clone();
                let r = o
                    .local_tool(
                        &js::gs(a, "name"),
                        a["params"].clone(),
                        Box::new(move |p: J| {
                            Box::pin(async move {
                                let task = {
                                    let mut ctx = me.ctx.borrow_mut();
                                    call_fn(&mut ctx, "__runExec", &[JsValue::from(exec), jstr(&p.to_string())])
                                };
                                match task {
                                    Ok(t) => match me.drive(t.as_number().unwrap_or(0.0)).await {
                                        Ok(v) => v,
                                        Err(e) => json!({ "ok": false, "error": e }),
                                    },
                                    Err(e) => json!({ "ok": false, "error": e }),
                                }
                            })
                        }),
                    )
                    .await;
                Ok(r)
            }
            "config_update" => {
                o.config.update(a);
                Ok(Value::Null)
            }
            "record" => {
                o.record(&js::gs(a, "type"), a["data"].clone());
                Ok(Value::Null)
            }
            "emit" => {
                o.emit(a.clone());
                Ok(Value::Null)
            }
            "notify" => {
                o.notify(a.clone());
                Ok(Value::Null)
            }
            other => Err(format!("opération inconnue : {other}")),
        };
        if id > 0 {
            let (ok, v) = match res {
                Ok(v) => (true, v),
                Err(e) => (false, json!(e)),
            };
            let mut ctx = self.ctx.borrow_mut();
            let _ = call_fn(&mut ctx, "__settle", &[JsValue::from(id as f64), JsValue::from(ok), jstr(&v.to_string())]);
        }
    }

    /// Entrées pour l'aide aux commandes (command_help).
    pub fn help_entries(&self) -> Vec<J> {
        let mut out = vec![];
        for s in self.items.borrow().iter() {
            let ex: Vec<String> = if s.examples.is_empty() { vec![s.name.clone()] } else { s.examples.iter().take(2).cloned().collect() };
            for (i, e) in ex.iter().enumerate() {
                out.push(json!({
                    "cat": "Mes compétences", "say": js::capitalize(e),
                    "what": if i > 0 { format!("{} (variante)", s.name) } else { s.name.clone() },
                    "keys": format!("{} {e}", s.name)
                }));
            }
        }
        out
    }
}

#[allow(unused)]
fn _k() -> JsValue {
    JsValue::from(js_string!(""))
}
