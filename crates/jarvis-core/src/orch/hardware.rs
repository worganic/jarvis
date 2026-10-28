// Sondes matérielles des cards HUD (orchestrator/hardware.js) : GPU (nvidia-smi, si une carte NVIDIA
// est là) et disques (capacité totale et occupée). Mesures lues en tâche de fond, gardées en cache.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::{json, Value};

use crate::js::{self, Timer, J};

const GPU_EVERY_MS: u64 = 3000;
const DISKS_EVERY_MS: u64 = 60000;
const GB: f64 = 1073741824.0;

pub struct HardwareProbe {
    gpu: RefCell<Value>,
    disks: RefCell<Vec<J>>,
    timers: (Timer, Timer),
    gpu_busy: Cell<bool>,
    gpu_absent: Cell<bool>,
    started: Cell<bool>,
}

impl HardwareProbe {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            gpu: RefCell::new(Value::Null),
            disks: RefCell::new(vec![]),
            timers: (Timer::new(), Timer::new()),
            gpu_busy: Cell::new(false),
            gpu_absent: Cell::new(false),
            started: Cell::new(false),
        })
    }

    pub fn start(self: &Rc<Self>) {
        if self.started.get() {
            return;
        }
        self.started.set(true);
        self.read_disks();
        self.read_gpu();
        let me = Rc::downgrade(self);
        self.timers.0.every(DISKS_EVERY_MS, move || {
            if let Some(s) = me.upgrade() {
                s.read_disks();
            }
        });
        let me = Rc::downgrade(self);
        self.timers.1.every(GPU_EVERY_MS, move || {
            if let Some(s) = me.upgrade() {
                s.read_gpu();
            }
        });
    }

    pub fn stop(&self) {
        self.timers.0.clear();
        self.timers.1.clear();
        self.started.set(false);
    }

    fn read_disks(&self) {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut list = vec![];
        for c in b'C'..=b'Z' {
            let root = format!("{}:\\", c as char);
            let w = jarvis_native::wide(&root);
            let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
            let ok = unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), Some(&mut total), Some(&mut total_free)) }.is_ok();
            if !ok || total == 0 {
                continue;
            }
            let used = total - total_free;
            list.push(json!({
                "name": format!("{}:", c as char),
                "usedGb": (used as f64 / GB).round() as i64,
                "totalGb": (total as f64 / GB).round() as i64,
                "pct": (used as f64 / total as f64 * 100.0).round() as i64
            }));
        }
        *self.disks.borrow_mut() = list;
    }

    fn read_gpu(self: &Rc<Self>) {
        if self.gpu_busy.get() || self.gpu_absent.get() {
            return;
        }
        self.gpu_busy.set(true);
        let me = self.clone();
        js::spawn(async move {
            let args: Vec<String> = ["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"].iter().map(|s| s.to_string()).collect();
            let res = crate::proc::run("nvidia-smi", &args, None, 4000).await;
            me.gpu_busy.set(false);
            match res {
                Err(e) => {
                    if e.contains("program not found") || e.contains("introuvable") || e.contains("cannot find") || e.contains("(os error 2)") {
                        me.gpu_absent.set(true);
                    }
                    *me.gpu.borrow_mut() = Value::Null;
                }
                Ok(o) if o.code != 0 => *me.gpu.borrow_mut() = Value::Null,
                Ok(o) => {
                    let line = o.stdout.lines().next().unwrap_or("").to_string();
                    let f: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
                    let g = |i: usize| f.get(i).cloned().unwrap_or_default();
                    let used: f64 = g(2).parse().unwrap_or(f64::NAN);
                    let total: f64 = g(3).parse().unwrap_or(f64::NAN);
                    let name = crate::re!(r"(?i)^NVIDIA\s+(GeForce\s+)?").replace(&if g(0).is_empty() { "GPU".to_string() } else { g(0) }, "").into_owned();
                    let temp: Option<f64> = g(4).parse().ok();
                    *me.gpu.borrow_mut() = json!({
                        "name": name,
                        "pct": g(1).parse::<f64>().unwrap_or(0.0),
                        "vramUsedGb": js::round1(used / 1024.0),
                        "vramTotalGb": js::round1(total / 1024.0),
                        "vramPct": if total > 0.0 { (used / total * 100.0).round() as i64 } else { 0 },
                        "temp": temp
                    });
                }
            }
        });
    }

    pub fn telemetry(&self) -> J {
        json!({ "gpu": *self.gpu.borrow(), "disks": *self.disks.borrow() })
    }
}
