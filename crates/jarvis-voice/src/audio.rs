// Capture et restitution audio (Capture, SystemAudioReference, PcLevelMeter et sortie de la voix
// de voice/engine.py) :
//
//   micro WASAPI ─► 48 kHz ─► anti-écho / bruit / gain (WebRTC APM, AEC3) ─► 16 kHz, blocs de 512
//        ▲ référence : tout le son du PC (capture « loopback »), sinon la voix de Jarvis elle-même
//
// Le son brut du micro est gardé en parallèle, bloc pour bloc (40 dernières secondes) : Whisper le
// reçoit quand rien n'a pu faire écho. Les flux cpal vivent dans un fil dédié (commandes).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use libwebrtc::native::apm::AudioProcessingModule;

use crate::util::{log, J};

pub const SR: usize = 16000;
pub const MIC_SR: usize = 48000;
pub const FRAME_48: usize = 480;
pub const BLOCK: usize = 512;
const RAW_RING: usize = (40 * SR) / BLOCK;

/// Filtre passe-bas (sinus cardinal fenêtré, 63 coefficients, coupure 7,2 kHz) puis un échantillon
/// sur trois : 48 kHz → 16 kHz, état conservé d'un bloc à l'autre (aucun clic aux jointures).
pub struct Decimator {
    h: Vec<f32>,
    state: Vec<f32>,
    phase: usize,
}

impl Default for Decimator {
    fn default() -> Self {
        Self::new(3, 7200.0, MIC_SR as f64)
    }
}

impl Decimator {
    const TAPS: usize = 63;

    pub fn new(factor: usize, cutoff: f64, rate: f64) -> Self {
        let _ = factor;
        let mid = (Self::TAPS - 1) as f64 / 2.0;
        let mut h: Vec<f64> = (0..Self::TAPS)
            .map(|i| {
                let n = i as f64 - mid;
                let x = 2.0 * cutoff / rate * n;
                let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
                let hamming = 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (Self::TAPS - 1) as f64).cos();
                sinc * hamming
            })
            .collect();
        let sum: f64 = h.iter().sum();
        h.iter_mut().for_each(|v| *v /= sum);
        Self { h: h.into_iter().map(|v| v as f32).collect(), state: vec![0.0; Self::TAPS - 1], phase: 0 }
    }

    pub fn process(&mut self, x: &[f32]) -> Vec<f32> {
        let mut buf = Vec::with_capacity(self.state.len() + x.len());
        buf.extend_from_slice(&self.state);
        buf.extend_from_slice(x);
        let taps = Self::TAPS;
        // np.convolve(buf, h, "valid") : y[k] = Σ buf[k + j] · h[taps - 1 - j]
        let n_out = buf.len() + 1 - taps;
        let mut out = vec![];
        let mut k = self.phase;
        while k < n_out {
            let mut acc = 0f32;
            for j in 0..taps {
                acc += buf[k + j] * self.h[taps - 1 - j];
            }
            out.push(acc);
            k += 3;
        }
        self.state = buf[buf.len() - (taps - 1)..].to_vec();
        self.phase = (self.phase as i64 - n_out as i64).rem_euclid(3) as usize;
        out
    }
}

/// Module WebRTC partagé entre la capture, la référence et la sortie de la voix.
pub struct Apm(pub AudioProcessingModule);
unsafe impl Send for Apm {}

pub type Tap = Box<dyn Fn(&[f32], Option<&[f32]>) + Send>;

/// État partagé de la capture (lu par l'écoute, la réunion, la voix).
pub struct CaptureShared {
    pub apm: Mutex<Option<Apm>>,
    pub aec: AtomicBool,
    pub reference: AtomicBool,
    pub config: Mutex<String>,
    pub input_name: Mutex<String>,
    start: Instant,
    last_echo_ms: AtomicU64,
    pub last_block_ms: AtomicU64,
    raw: Mutex<(VecDeque<Vec<f32>>, u64)>,
    pending: Mutex<(Vec<f32>, Vec<f32>)>,
    pub taps: Mutex<Vec<(u64, Tap)>>,
    pub file_input: Option<String>,
    /// Micro coupé (HUD, visio) : micro, référence et niveau du PC sont fermés, plus aucune capture ne tourne.
    pub suspended: AtomicBool,
    blocks: SyncSender<Vec<f32>>,
    pub debug_ref: Mutex<Option<Arc<crate::debug::AudioDebug>>>,
}

impl CaptureShared {
    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn mark_echo(&self) {
        self.last_echo_ms.store(self.now_ms().max(1), Ordering::Relaxed);
    }

    /// Secondes depuis le dernier son (Jarvis, PC) qui a pu revenir dans le micro.
    pub fn since_echo(&self) -> f64 {
        let l = self.last_echo_ms.load(Ordering::Relaxed);
        if l == 0 {
            1e9
        } else {
            (self.now_ms().saturating_sub(l)) as f64 / 1000.0
        }
    }

    pub fn since_block(&self) -> f64 {
        (self.now_ms().saturating_sub(self.last_block_ms.load(Ordering::Relaxed))) as f64 / 1000.0
    }

    pub fn config(&self) -> String {
        self.config.lock().unwrap().clone()
    }

    /// Range des échantillons 16 kHz traités (et bruts) et émet les blocs de 512.
    pub fn emit16(&self, samples16: &[f32], raw16: Option<&[f32]>) {
        let mut out_blocks = vec![];
        {
            let mut p = self.pending.lock().unwrap();
            p.0.extend_from_slice(samples16);
            p.1.extend_from_slice(raw16.unwrap_or(samples16));
            let n = p.0.len().min(p.1.len()) / BLOCK;
            // Chaque bloc brut est rangé avant son jumeau traité : le bloc n° k lu par l'écoute a
            // toujours son brut n° k.
            for i in 0..n {
                let rb = p.1[i * BLOCK..(i + 1) * BLOCK].to_vec();
                {
                    let mut raw = self.raw.lock().unwrap();
                    raw.0.push_back(rb);
                    while raw.0.len() > RAW_RING {
                        raw.0.pop_front();
                    }
                    raw.1 += 1;
                }
                out_blocks.push(p.0[i * BLOCK..(i + 1) * BLOCK].to_vec());
            }
            p.0.drain(..n * BLOCK);
            p.1.drain(..n * BLOCK);
        }
        if !out_blocks.is_empty() {
            self.last_block_ms.store(self.now_ms(), Ordering::Relaxed);
        }
        for b in out_blocks {
            let _ = self.blocks.try_send(b);
        }
        for (_, tap) in self.taps.lock().unwrap().iter() {
            tap(samples16, raw16);
        }
    }

    /// Son brut des blocs n° first … first+count-1 (numérotés depuis le démarrage), ou None.
    pub fn raw_blocks(&self, first: u64, count: usize) -> Option<Vec<f32>> {
        let raw = self.raw.lock().unwrap();
        let oldest = raw.1 - raw.0.len() as u64;
        if count == 0 || first < oldest || first + count as u64 > raw.1 {
            return None;
        }
        let start = (first - oldest) as usize;
        Some(raw.0.iter().skip(start).take(count).flat_map(|b| b.iter().copied()).collect())
    }

    /// Voix de Jarvis jouée : référence d'écho quand le son du PC n'est pas capté.
    pub fn render_reference(&self, pcm48: &mut [i16]) {
        if pcm48.iter().any(|s| s.unsigned_abs() > 32) {
            self.mark_echo();
        }
        if self.reference.load(Ordering::Relaxed) || !self.aec.load(Ordering::Relaxed) {
            return;
        }
        if let Some(apm) = self.apm.lock().unwrap().as_mut() {
            let usable = pcm48.len() - pcm48.len() % FRAME_48;
            if usable > 0 {
                let _ = apm.0.process_reverse_stream(&mut pcm48[..usable], MIC_SR as i32, 1);
            }
        }
    }
}

/// Commandes du fil audio.
pub enum AudioCmd {
    Open,
    Reopen,
    /// true : ferme les captures (micro, anti-écho, niveau du PC) ; false : les rouvre.
    Suspend(bool),
    PcMeter(bool),
    EnsureOutput(Sender<bool>),
    ReleaseOutput,
    /// Son du PC en 16 kHz mono (réunion) ; None arrête. Répond le nom du périphérique.
    Loopback(Option<Box<dyn FnMut(&[f32]) + Send>>, Sender<Result<String, String>>),
}

/// Sortie de la voix : blocs float32 mono à 48 kHz tirés par le flux de sortie.
pub struct VoiceOut {
    pub chunks: VecDeque<Vec<f32>>,
    pub pos: usize,
    pub pending: usize,
    pub gen: u64,
    /// Volume de la rampe de pause (0 = muet, 1 = plein).
    pub duck: f32,
    /// En pause : l'utilisateur parle, la file attend sans avancer.
    pub held: bool,
    held_at: Option<Instant>,
    pub gain: f32,
    pub gain_now: f32,
    pub progress_at: Option<Instant>,
    pub drained_flag: bool,
    rev: Vec<i16>,
}

impl Default for VoiceOut {
    fn default() -> Self {
        Self { chunks: VecDeque::new(), pos: 0, pending: 0, gen: 0, duck: 1.0, held: false, held_at: None, gain: 1.0, gain_now: 1.0, progress_at: None, drained_flag: false, rev: vec![] }
    }
}

const DUCK_RAMP: f32 = 0.08;
/// Pause plus longue : la phrase interrompue est redite depuis son début.
const REPEAT_AFTER: f64 = 1.0;

impl VoiceOut {
    /// Pause (l'utilisateur parle) ou reprise. Rien d'audible encore : la pause est immédiate, sans fondu.
    pub fn set_held(&mut self, on: bool) {
        if on && !self.held {
            self.held = true;
            self.held_at = Some(Instant::now());
            if self.pos == 0 {
                self.duck = 0.0;
            }
        } else if !on && self.held {
            self.held = false;
            if self.held_at.take().map(|t| t.elapsed().as_secs_f64() >= REPEAT_AFTER).unwrap_or(false) {
                self.pos = 0;
            }
        }
    }

    /// Remplit `out` (mono 48 kHz) ; rend true si la file vient de se vider.
    pub fn fill(&mut self, out: &mut [f32], cap: &CaptureShared) -> bool {
        let frames = out.len();
        self.play(out);
        // Volume propre de Jarvis, rampe sans clic.
        if self.gain_now != 1.0 || self.gain != 1.0 {
            let end = self.gain;
            for (i, s) in out.iter_mut().enumerate() {
                *s *= self.gain_now + (end - self.gain_now) * i as f32 / (frames.max(2) - 1) as f32;
            }
            self.gain_now = end;
        }
        let mut buf = std::mem::take(&mut self.rev);
        buf.extend(out.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16));
        let usable = buf.len() - buf.len() % FRAME_48;
        cap.render_reference(&mut buf[..usable]);
        self.rev = buf[usable..].to_vec();
        self.chunks.is_empty() && self.pending == 0
    }

    /// File de parole vers `out`, avec la rampe de pause (fondu court, AVANT la référence d'anti-écho :
    /// l'AEC3 reçoit le son joué). Une fois muette, la pause n'avance plus dans la file.
    fn play(&mut self, out: &mut [f32]) {
        let frames = out.len();
        let had = !self.chunks.is_empty();
        let target = if self.held { 0.0 } else { 1.0 };
        if !had {
            self.duck = target;
        }
        let mut filled = 0;
        if !(self.held && self.duck == 0.0) {
            while filled < frames {
                let Some(chunk) = self.chunks.front() else { break };
                let n = (frames - filled).min(chunk.len() - self.pos);
                out[filled..filled + n].copy_from_slice(&chunk[self.pos..self.pos + n]);
                filled += n;
                self.pos += n;
                if self.pos >= chunk.len() {
                    self.chunks.pop_front();
                    self.pos = 0;
                }
            }
        }
        out[filled..].iter_mut().for_each(|s| *s = 0.0);
        if had {
            // En pause aussi : le garde-fou « parole bloquée » ne doit pas vider la file.
            self.progress_at = Some(Instant::now());
        }
        if self.duck != target {
            let step = frames as f32 / (MIC_SR as f32 * DUCK_RAMP);
            let end = if (target - self.duck).abs() <= step { target } else if target > self.duck { self.duck + step } else { self.duck - step };
            for (i, s) in out.iter_mut().enumerate() {
                *s *= self.duck + (end - self.duck) * i as f32 / (frames.max(2) - 1) as f32;
            }
            self.duck = end;
        }
    }
}

pub struct Audio {
    pub shared: Arc<CaptureShared>,
    pub voice: Arc<Mutex<VoiceOut>>,
    cmd: Sender<AudioCmd>,
    pub opts_echo: bool,
}

/// Périphérique nommé (réglage engine.micDevice / engine.outDevice : fragment du nom, carte son virtuelle comprise) ;
/// vide ou introuvable : le périphérique par défaut de Windows.
fn pick_device(host: &cpal::Host, want: &str, input: bool) -> Option<cpal::Device> {
    let want = want.trim().to_lowercase();
    if !want.is_empty() {
        let list = if input { host.input_devices() } else { host.output_devices() };
        if let Some(d) = list.ok().and_then(|mut l| l.find(|d| device_name(d).to_lowercase().contains(&want))) {
            return Some(d);
        }
        log(&format!("périphérique « {want} » introuvable : périphérique par défaut de Windows"));
    }
    if input { host.default_input_device() } else { host.default_output_device() }
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_default()
}

/// Capture d'un périphérique (micro, ou sortie en « loopback ») : rappel avec du mono 48 kHz.
fn open_input(device: &cpal::Device, output_side: bool, mut on_mono48: impl FnMut(&[f32]) + Send + 'static) -> Result<cpal::Stream, String> {
    let cfg = if output_side { device.default_output_config() } else { device.default_input_config() }.map_err(|e| e.to_string())?;
    let channels = cfg.channels().max(1) as usize;
    let rate = cfg.sample_rate() as usize;
    let config = cpal::StreamConfig { channels: cfg.channels(), sample_rate: cfg.sample_rate(), buffer_size: cpal::BufferSize::Default };
    let mut carry: Vec<f32> = vec![];
    let fmt = cfg.sample_format();
    let mut handle = move |mono: Vec<f32>| {
        if rate == MIC_SR {
            on_mono48(&mono);
        } else {
            // Rééchantillonnage linéaire vers 48 kHz (rare : la plupart des périphériques sont à 48 kHz).
            carry.extend(mono);
            let n = carry.len() / 10 * 10;
            if n > 0 {
                let out = crate::util::interp_resample(&carry[..n], rate, MIC_SR);
                carry.drain(..n);
                on_mono48(&out);
            }
        }
    };
    let err = |e: cpal::Error| log(&format!("flux audio : {e}"));
    let stream = match fmt {
        cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            config,
            move |data: &[f32], _| handle(data.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect()),
            err,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
            config,
            move |data: &[i16], _| handle(data.chunks(channels).map(|c| c.iter().map(|s| *s as f32 / 32768.0).sum::<f32>() / channels as f32).collect()),
            err,
            None,
        ),
        other => return Err(format!("format audio non géré : {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

impl Audio {
    /// Démarre le fil audio ; `blocks` reçoit les blocs traités (16 kHz, 512).
    pub fn start(opts: J, hub: crate::hub::Hub) -> (Self, Receiver<Vec<f32>>) {
        let (btx, brx) = std::sync::mpsc::sync_channel::<Vec<f32>>(400);
        let shared = Arc::new(CaptureShared {
            apm: Mutex::new(None),
            aec: AtomicBool::new(false),
            reference: AtomicBool::new(false),
            config: Mutex::new("haut-parleurs".into()),
            input_name: Mutex::new(String::new()),
            start: Instant::now(),
            last_echo_ms: AtomicU64::new(0),
            last_block_ms: AtomicU64::new(0),
            raw: Mutex::new((VecDeque::new(), 0)),
            pending: Mutex::new((vec![], vec![])),
            taps: Mutex::new(vec![]),
            file_input: std::env::var("JARVIS_ENGINE_INPUT").ok().filter(|s| !s.is_empty()),
            suspended: AtomicBool::new(false),
            blocks: btx,
            debug_ref: Mutex::new(None),
        });
        let voice = Arc::new(Mutex::new(VoiceOut::default()));
        let (tx, rx) = std::sync::mpsc::channel::<AudioCmd>();
        let (s2, v2, o2) = (shared.clone(), voice.clone(), opts.clone());
        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || audio_thread(rx, s2, v2, o2, hub))
            .expect("fil audio");
        (Self { shared, voice, cmd: tx, opts_echo: opts["echoCancellation"] != J::Bool(false) }, brx)
    }

    pub fn send(&self, c: AudioCmd) {
        let _ = self.cmd.send(c);
    }

    /// Capture du son du PC (16 kHz mono) pour l'enregistrement d'une réunion ; None l'arrête.
    pub fn loopback(&self, on_samples: Option<Box<dyn FnMut(&[f32]) + Send>>) -> Result<String, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send(AudioCmd::Loopback(on_samples, tx));
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Err("délai dépassé".into()))
    }

    /// Ouvre (ou rouvre) la sortie de la voix ; false si aucune sortie n'est utilisable.
    pub fn ensure_output(&self) -> bool {
        if std::env::var("JARVIS_ENGINE_OUTPUT").is_ok() {
            return true;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.send(AudioCmd::EnsureOutput(tx));
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false)
    }
}

struct Streams {
    mic: Option<cpal::Stream>,
    reference: Option<cpal::Stream>,
    meter: Option<cpal::Stream>,
    meeting: Option<cpal::Stream>,
    output: Option<cpal::Stream>,
    output_dead: Arc<AtomicBool>,
}

fn audio_thread(rx: Receiver<AudioCmd>, shared: Arc<CaptureShared>, voice: Arc<Mutex<VoiceOut>>, opts: J, hub: crate::hub::Hub) {
    let mut st = Streams { mic: None, reference: None, meter: None, meeting: None, output: None, output_dead: Arc::new(AtomicBool::new(false)) };
    let open = |st: &mut Streams| {
        st.mic = None;
        st.reference = None;
        if let Some(f) = shared.file_input.clone() {
            let s = shared.clone();
            std::thread::spawn(move || feed_files(&f, s));
            return;
        }
        // Les périphériques ont pu changer (casque branché) : la sortie de la voix est rouverte.
        st.output = None;
        let host = cpal::default_host();
        let out_name = host.default_output_device().map(|d| device_name(&d)).unwrap_or_default();
        let Some(mic) = pick_device(&host, opts["micDevice"].as_str().unwrap_or(""), true) else {
            log("aucun micro disponible");
            return;
        };
        *shared.input_name.lock().unwrap() = device_name(&mic);
        let config = crate::speaker::audio_config(&out_name).to_string();
        *shared.config.lock().unwrap() = config.clone();
        // Au casque, aucun écho possible : l'AEC est coupé (il n'apporterait que des artefacts).
        let aec = opts["echoCancellation"] != J::Bool(false) && config != "casque";
        shared.aec.store(aec, Ordering::Relaxed);
        *shared.apm.lock().unwrap() = Some(Apm(AudioProcessingModule::new(
            aec,
            opts["autoGainControl"] != J::Bool(false),
            true,
            opts["noiseSuppression"] != J::Bool(false),
        )));
        shared.reference.store(false, Ordering::Relaxed);
        if aec && opts["systemAudioReference"] != J::Bool(false) {
            if let Some(out) = host.default_output_device() {
                let s = shared.clone();
                let mut pend: Vec<f32> = vec![];
                match open_input(&out, true, move |mono| {
                    if mono.iter().any(|x| x.abs() > 0.001) {
                        s.mark_echo();
                    }
                    if let Some(d) = s.debug_ref.lock().unwrap().as_ref() {
                        d.add_ref(mono);
                    }
                    pend.extend_from_slice(mono);
                    let usable = pend.len() - pend.len() % FRAME_48;
                    if usable == 0 {
                        return;
                    }
                    let mut pcm: Vec<i16> = pend[..usable].iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
                    pend.drain(..usable);
                    if let Some(apm) = s.apm.lock().unwrap().as_mut() {
                        let _ = apm.0.process_reverse_stream(&mut pcm, MIC_SR as i32, 1);
                    }
                }) {
                    Ok(stream) => {
                        st.reference = Some(stream);
                        shared.reference.store(true, Ordering::Relaxed);
                        log(&format!("anti-écho : référence = tout le son du PC (« {out_name} »)"));
                    }
                    Err(e) => log(&format!("référence son du PC indisponible ({e}) : anti-écho sur la seule voix de Jarvis")),
                }
            }
        }
        let s = shared.clone();
        let mut dec = Decimator::default();
        let mut dec_raw = Decimator::default();
        let mut pend: Vec<f32> = vec![];
        match open_input(&mic, false, move |mono| {
            pend.extend_from_slice(mono);
            let usable = pend.len() - pend.len() % FRAME_48;
            if usable == 0 {
                return;
            }
            let frame: Vec<f32> = pend.drain(..usable).collect();
            let mut pcm: Vec<i16> = frame.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
            if let Some(apm) = s.apm.lock().unwrap().as_mut() {
                let _ = apm.0.set_stream_delay_ms(50);
                let _ = apm.0.process_stream(&mut pcm, MIC_SR as i32, 1);
            }
            let processed: Vec<f32> = pcm.iter().map(|v| *v as f32 / 32768.0).collect();
            let raw16 = dec_raw.process(&frame);
            let p16 = dec.process(&processed);
            s.emit16(&p16, Some(&raw16));
        }) {
            Ok(stream) => st.mic = Some(stream),
            Err(e) => log(&format!("micro « {} » indisponible : {e}", device_name(&mic))),
        }
        shared.last_block_ms.store(shared.now_ms(), Ordering::Relaxed);
        log(&format!(
            "micro « {} » · sortie « {out_name} » ({config}) · anti-écho {}{}",
            shared.input_name.lock().unwrap(),
            if aec { "actif" } else { "coupé (casque)" },
            if shared.reference.load(Ordering::Relaxed) { " + son du PC" } else { "" }
        ));
    };
    let ensure_output = |st: &mut Streams| -> bool {
        if st.output.is_some() && !st.output_dead.load(Ordering::Relaxed) {
            return true;
        }
        st.output = None;
        let host = cpal::default_host();
        let Some(dev) = pick_device(&host, opts["outDevice"].as_str().unwrap_or(""), false) else { return false };
        let channels = dev.default_output_config().map(|c| c.channels()).unwrap_or(2).max(1);
        let config = cpal::StreamConfig { channels, sample_rate: MIC_SR as u32, buffer_size: cpal::BufferSize::Default };
        let (v, s, h) = (voice.clone(), shared.clone(), hub.clone());
        let dead = Arc::new(AtomicBool::new(false));
        let dead2 = dead.clone();
        let ch = channels as usize;
        let mut mono = vec![];
        let res = dev.build_output_stream::<f32, _, _>(
            config,
            move |data: &mut [f32], _| {
                let frames = data.len() / ch;
                mono.resize(frames, 0.0);
                let drained = {
                    let mut vo = v.lock().unwrap();
                    let was_playing = !vo.chunks.is_empty();
                    let d = vo.fill(&mut mono, &s);
                    if d && (was_playing || !vo.drained_flag) {
                        vo.drained_flag = true;
                    }
                    d && was_playing
                };
                for (i, f) in data.chunks_mut(ch).enumerate() {
                    f.iter_mut().for_each(|x| *x = mono[i]);
                }
                if drained {
                    h.signal_drained();
                }
            },
            move |e: cpal::Error| {
                log(&format!("sortie audio perdue ({e}) : réouverture"));
                dead2.store(true, Ordering::Relaxed);
            },
            None,
        );
        match res.and_then(|s| s.play().map(|_| s)) {
            Ok(stream) => {
                st.output = Some(stream);
                st.output_dead = dead;
                true
            }
            Err(e) => {
                log(&format!("sortie audio indisponible : {e}"));
                false
            }
        }
    };
    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCmd::Open if shared.suspended.load(Ordering::SeqCst) => {}
            AudioCmd::Reopen if shared.suspended.load(Ordering::SeqCst) => {}
            AudioCmd::Suspend(on) => {
                let was = shared.suspended.swap(on, Ordering::SeqCst);
                if on && !was {
                    st.mic = None;
                    st.reference = None;
                    st.meter = None;
                    shared.reference.store(false, Ordering::Relaxed);
                    log("micro coupé : captures fermées (micro, anti-écho, niveau du PC)");
                } else if !on && was {
                    log("micro rétabli : captures rouvertes");
                    open(&mut st);
                }
            }
            AudioCmd::Open => open(&mut st),
            AudioCmd::Reopen => {
                log("réouverture de l'audio (périphérique changé ou flux muet)");
                open(&mut st);
            }
            AudioCmd::EnsureOutput(reply) => {
                let ok = ensure_output(&mut st);
                let _ = reply.send(ok);
            }
            AudioCmd::ReleaseOutput => st.output = None,
            AudioCmd::Loopback(cb, reply) => {
                st.meeting = None;
                let Some(mut cb) = cb else {
                    let _ = reply.send(Ok(String::new()));
                    continue;
                };
                let Some(out) = cpal::default_host().default_output_device() else {
                    let _ = reply.send(Err("aucune sortie audio".into()));
                    continue;
                };
                let name = device_name(&out);
                let mut dec = Decimator::default();
                match open_input(&out, true, move |mono| {
                    let s16 = dec.process(mono);
                    if !s16.is_empty() {
                        cb(&s16);
                    }
                }) {
                    Ok(s) => {
                        st.meeting = Some(s);
                        let _ = reply.send(Ok(name));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            AudioCmd::PcMeter(true) if shared.suspended.load(Ordering::SeqCst) => {}
            AudioCmd::PcMeter(on) => {
                if !on {
                    st.meter = None;
                    hub.emit(serde_json::json!({ "t": "pc_level", "v": 0.0 }));
                    continue;
                }
                if st.meter.is_some() {
                    continue;
                }
                let Some(out) = cpal::default_host().default_output_device() else { continue };
                let h = hub.clone();
                let mut acc: Vec<f32> = vec![];
                let mut level = 0f32;
                // Niveau du son joué par le PC, 50 fois par seconde (animation de la note du cœur ; le tempo de la musique, ui/music-style.js).
                match open_input(&out, true, move |mono| {
                    acc.extend_from_slice(mono);
                    while acc.len() >= 960 {
                        let chunk: Vec<f32> = acc.drain(..960).collect();
                        let rms = crate::util::rms(&chunk);
                        let target = ((20.0 * rms.max(1e-6).log10() + 50.0) / 40.0).clamp(0.0, 1.0);
                        level = if target > level { target } else { level + (target - level) * 0.16 }; // même retombée qu'à 20 Hz
                        h.emit(serde_json::json!({ "t": "pc_level", "v": (level as f64 * 1000.0).round() / 1000.0 }));
                    }
                }) {
                    Ok(s) => st.meter = Some(s),
                    Err(e) => log(&format!("niveau du son du PC indisponible : {e}")),
                }
            }
        }
    }
}

/// Entrée fichier (tests) : JARVIS_ENGINE_INPUT=<fichier.wav|liste;…>, au rythme réel.
fn feed_files(list: &str, shared: Arc<CaptureShared>) {
    std::thread::sleep(Duration::from_millis(500));
    let speed: f64 = std::env::var("JARVIS_ENGINE_SPEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    for path in list.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let Some((audio, sr)) = crate::util::read_wav(std::path::Path::new(path)) else {
            log(&format!("entrée fichier illisible : {path}"));
            continue;
        };
        let a = crate::util::interp_resample(&audio, sr, SR);
        let mut full = vec![0f32; SR / 2];
        full.extend(a);
        full.extend(vec![0f32; SR * 3 / 2]);
        for chunk in full.chunks(BLOCK) {
            shared.emit16(chunk, None);
            std::thread::sleep(Duration::from_secs_f64(BLOCK as f64 / SR as f64 / speed));
        }
    }
    log("entrée fichier terminée");
    loop {
        shared.emit16(&[0f32; BLOCK], None);
        std::thread::sleep(Duration::from_secs_f64(BLOCK as f64 / SR as f64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_pause_n_avance_pas_et_reprend_au_debut_de_la_phrase() {
        let mut vo = VoiceOut::default();
        vo.chunks.push_back(vec![0.5; 48_000]);
        let mut out = vec![0.0; 480];
        vo.play(&mut out);
        assert_eq!(vo.pos, 480);
        vo.set_held(true);
        // Fondu de 80 ms, puis silence sans avancer.
        for _ in 0..10 {
            vo.play(&mut out);
        }
        let pos = vo.pos;
        vo.play(&mut out);
        assert_eq!(vo.pos, pos);
        assert!(out.iter().all(|s| *s == 0.0));
        // Pause courte : reprise sur place.
        vo.set_held(false);
        assert_eq!(vo.pos, pos);
        // Pause longue : la phrase est redite.
        vo.set_held(true);
        vo.held_at = Some(Instant::now() - std::time::Duration::from_secs(2));
        vo.set_held(false);
        assert_eq!(vo.pos, 0);
    }

    #[test]
    fn une_phrase_en_attente_ne_laisse_rien_entendre() {
        let mut vo = VoiceOut::default();
        vo.set_held(true);
        vo.chunks.push_back(vec![0.5; 4_800]);
        let mut out = vec![1.0; 480];
        vo.play(&mut out);
        assert_eq!(vo.pos, 0);
        assert!(out.iter().all(|s| *s == 0.0));
    }
}
