// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! effectcraft-helper-lightwrap: an EffectCraft external effect helper that offers
//! mos_灯光混合 (Light Wrap Fantastic 复刻) as an effect.
//!
//! Protocol: docs/external-helpers.md (version 1). Requests are JSON lines on stdin, replies
//! are JSON lines on stdout prefixed with `@ec-helper `. Pixels live in the frame exchange
//! file named by `hello` as premultiplied RGBA f32.

mod blur;
mod lang;
mod lwf;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAGIC: &str = "@ec-helper ";
const PROTOCOL_VERSION: u32 = 1;
const HELPER_VERSION: &str = "1.4.0";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct ImageRef {
    offset: u64,
    width: u32,
    height: u32,
    #[serde(default)]
    origin: [f64; 2],
    #[serde(default = "one")]
    scale: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Req {
    Hello {
        version: u32,
        frames: String,
    },
    Describe,
    Load {
        path: String,
    },
    Padding {
        effect: String,
        instance: u64,
        time: f64,
        scale: f64,
        layer_size: [f64; 2],
        flat: Vec<f64>,
        #[serde(default)]
        texts: Vec<String>,
    },
    Render {
        effect: String,
        instance: u64,
        time: f64,
        frame: ImageRef,
        layer_size: [f64; 2],
        flat: Vec<f64>,
        #[serde(default)]
        texts: Vec<String>,
        #[serde(default)]
        frame_rate: f64,
        #[serde(default)]
        frame_range: Option<(f64, f64)>,
    },
    Image {
        image: Option<ImageRef>,
    },
    Params {
        #[serde(default)]
        values: Option<(Vec<f64>, Vec<String>)>,
    },
    Destroy {
        instance: u64,
    },
    Ping,
    Shutdown,
}

struct Host {
    /// Replies go through a lock, so the stdin reader thread can talk while a render runs.
    out: Arc<Mutex<std::io::Stdout>>,
    /// Requests, parsed by the stdin reader thread: waiting for one can time out.
    rx: Receiver<Result<Req, String>>,
    frames: Option<File>,
    /// Renders running right now (the outer one plus any nested ones).
    depth: u32,
    /// The last layer image, kept while the caller drags a slider at one time.
    cache: Option<LayerCache>,
    /// Set when a layer image never arrived; the running render reports it instead of silently
    /// rendering without the layer.
    layer_failure: Option<String>,
}

/// A layer image the host handed over: valid for `MOS_LIGHTWRAP_LAYER_CACHE_SECS` (default 3).
struct LayerCache {
    param: String,
    time: f64,
    at: Instant,
    width: usize,
    height: usize,
    px: Vec<f32>,
}

fn main() {
    log_line(&format!(
        "start: pid {}, layer timeout {} s, layer cache {} s",
        std::process::id(),
        layer_timeout().as_secs(),
        layer_cache_ttl().as_secs()
    ));
    let mut host = Host {
        out: Arc::new(Mutex::new(std::io::stdout())),
        rx: spawn_reader(),
        frames: None,
        depth: 0,
        cache: None,
        layer_failure: None,
    };
    loop {
        let req = match host.next_req() {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                log_line(&format!("bad request: {e}"));
                continue;
            }
            None => break,
        };
        match req {
            Req::Hello { version, frames } => {
                if version != PROTOCOL_VERSION {
                    host.send(json!({"op": "error", "message": format!("this helper speaks protocol {PROTOCOL_VERSION}, not {version}")}));
                    break;
                }
                match OpenOptions::new().read(true).write(true).open(&frames) {
                    Ok(f) => {
                        host.frames = Some(f);
                        log_line(&format!("hello: frames={frames}"));
                        host.send(json!({"op": "hello", "version": PROTOCOL_VERSION, "name": lang::effect_name(), "helper_version": HELPER_VERSION, "loads": []}));
                    }
                    Err(e) => {
                        host.send(json!({"op": "error", "message": format!("can't open the frame exchange file: {e}")}));
                        break;
                    }
                }
            }
            Req::Describe => {
                let mut manifest = lwf::manifest();
                lang::localize_manifest(&mut manifest);
                host.send(json!({
                    "op": "effects",
                    "effects": [{"manifest": manifest, "padding": true}],
                    "errors": []
                }));
            }
            Req::Load { path } => {
                log_line(&format!("load (ignored): {path}"));
                host.send(json!({"op": "effects", "effects": [], "errors": ["this helper does not load files"]}));
            }
            Req::Padding { flat, scale, .. } => host.answer_padding(&flat, scale),
            Req::Render { effect, time, frame, layer_size, flat, .. } => {
                if effect != "mos.lightwrap" {
                    host.send(json!({"op": "error", "message": format!("unknown effect `{effect}`")}));
                    continue;
                }
                match host.render_frame(&frame, time, layer_size, &flat) {
                    Ok(()) => host.send(json!({"op": "done"})),
                    Err(e) => {
                        log_line(&format!("render error: {e}"));
                        host.send(json!({"op": "error", "message": e}));
                    }
                }
            }
            Req::Image { .. } | Req::Params { .. } => {
                log_line("unexpected image/params reply");
            }
            Req::Destroy { .. } => {}
            Req::Ping => host.send(json!({"op": "pong"})),
            Req::Shutdown => break,
        }
    }
}

impl Host {
    fn send(&self, v: serde_json::Value) {
        if let Ok(mut out) = self.out.lock() {
            let _ = writeln!(out, "{MAGIC}{v}");
            let _ = out.flush();
        }
    }

    /// Answer `padding` with the pixels it asks for at the resolution it asks for.
    fn answer_padding(&self, flat: &[f64], scale: f64) {
        self.send(json!({"op": "padding", "pixels": lwf::padding(flat, scale)}));
    }

    fn next_req(&mut self) -> Option<Result<Req, String>> {
        match self.rx.recv() {
            Ok(r) => Some(r),
            Err(_) => None,
        }
    }

    fn render_frame(&mut self, frame: &ImageRef, time: f64, layer_size: [f64; 2], flat: &[f64]) -> Result<(), String> {
        self.depth += 1;
        let result = self.render_inner(frame, time, layer_size, flat);
        self.depth -= 1;
        result
    }

    fn render_inner(&mut self, frame: &ImageRef, time: f64, layer_size: [f64; 2], flat: &[f64]) -> Result<(), String> {
        let started = Instant::now();
        let w = frame.width as usize;
        let h = frame.height as usize;
        let mut px = self.read_image(frame)?;
        log_line(&format!(
            "render {}x{} origin={:?} scale={} layer_size={:?} flat={:?}",
            w, h, frame.origin, frame.scale, layer_size, flat
        ));
        let dump = if debug_on() && std::env::temp_dir().join("lwf_dump").exists() {
            let dir = std::env::temp_dir().join("lwf-frames");
            let _ = std::fs::create_dir_all(&dir);
            dump_pam(&dir.join("in.pam"), w, h, &px);
            Some(dir)
        } else {
            None
        };
        lwf::render(self, &mut px, w, h, frame.origin, layer_size, frame.scale, time, flat)?;
        if let Some(msg) = self.layer_failure.take() {
            log_line(&format!("render failed: {msg}"));
            return Err(msg);
        }
        if let Some(dir) = dump {
            dump_pam(&dir.join("out.pam"), w, h, &px);
        }
        self.write_image(frame, &px)?;
        log_line(&format!("render {w}x{h} done in {} ms", started.elapsed().as_millis()));
        Ok(())
    }

    fn read_image(&mut self, r: &ImageRef) -> Result<Vec<f32>, String> {
        let n = r.width as usize * r.height as usize * 4;
        let file = self.frames.as_mut().ok_or("no frame exchange file")?;
        let mut buf = vec![0u8; n * 4];
        file.seek(SeekFrom::Start(r.offset)).map_err(|e| e.to_string())?;
        file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        let mut out = vec![0.0f32; n];
        for i in 0..n {
            out[i] = f32::from_le_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]);
        }
        Ok(out)
    }

    fn write_image(&mut self, r: &ImageRef, px: &[f32]) -> Result<(), String> {
        let mut buf = vec![0u8; px.len() * 4];
        for (i, v) in px.iter().enumerate() {
            buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let file = self.frames.as_mut().ok_or("no frame exchange file")?;
        file.seek(SeekFrom::Start(r.offset)).map_err(|e| e.to_string())?;
        file.write_all(&buf).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Record a layer that never arrived: the render then fails with a readable message instead of
    /// quietly drawing nothing where the layer should be.
    fn layer_timeout_failed(&mut self, param: &str, limit: Duration) {
        log_line(&format!("layer `{param}`: no answer within {} s", limit.as_secs()));
        self.layer_failure = Some(lang::layer_timeout(param, limit.as_secs()));
    }

    /// Ask the host for a layer parameter's pixels, serving nested renders while waiting. A layer
    /// image is cached for a moment, so dragging a slider at one time does not ask again for every
    /// frame. Waiting gives up after `MOS_LIGHTWRAP_LAYER_TIMEOUT` seconds: a host that stops
    /// answering fails this render instead of wedging the frame pipeline for good.
    fn ask_layer(&mut self, param: &str, time: f64) -> Option<(usize, usize, Vec<f32>)> {
        let ttl = layer_cache_ttl();
        if !ttl.is_zero() {
            let hit = self.cache.as_ref().filter(|c| c.param == param && c.time == time && c.at.elapsed() < ttl);
            if let Some(c) = hit {
                log_line(&format!("layer `{param}` cached {}x{}", c.width, c.height));
                return Some((c.width, c.height, c.px.clone()));
            }
        }
        let started = Instant::now();
        let limit = layer_timeout();
        self.send(json!({"op": "layer_input", "param": param, "time": time}));
        loop {
            let left = match limit.checked_sub(started.elapsed()) {
                Some(left) if !left.is_zero() => left,
                _ => {
                    self.layer_timeout_failed(param, limit);
                    return None;
                }
            };
            let req = match self.rx.recv_timeout(left) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => {
                    self.layer_timeout_failed(param, limit);
                    return None;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.layer_failure = Some(lang::layer_closed(param));
                    return None;
                }
            };
            match req.ok()? {
                Req::Image { image } => {
                    let r = image?;
                    let (w, h) = (r.width as usize, r.height as usize);
                    let px = match self.read_image(&r) {
                        Ok(p) => p,
                        Err(e) => {
                            log_line(&format!("layer image read failed: {e}"));
                            return None;
                        }
                    };
                    log_line(&format!(
                        "layer `{param}` {w}x{h} origin={:?} scale={} after {} ms",
                        r.origin,
                        r.scale,
                        started.elapsed().as_millis()
                    ));
                    if !ttl.is_zero() {
                        self.cache = Some(LayerCache {
                            param: param.to_string(),
                            time,
                            at: Instant::now(),
                            width: w,
                            height: h,
                            px: px.clone(),
                        });
                    }
                    return Some((w, h, px));
                }
                Req::Render { effect, time, frame, layer_size, flat, .. } => {
                    let out = if effect == "mos.lightwrap" {
                        self.render_frame(&frame, time, layer_size, &flat)
                    } else {
                        Err(format!("unknown effect `{effect}`"))
                    };
                    match out {
                        Ok(()) => self.send(json!({"op": "done"})),
                        Err(e) => self.send(json!({"op": "error", "message": e})),
                    }
                }
                // The protocol lets EffectCraft ask `padding` while the helper waits for an answer:
                // answer it, or the host waits out its own timeout for a reply that never comes.
                Req::Padding { flat, scale, .. } => self.answer_padding(&flat, scale),
                Req::Ping => self.send(json!({"op": "pong"})),
                Req::Shutdown => return None,
                other => log_line(&format!("ignored while waiting: {other:?}")),
            }
        }
    }
}

impl lwf::Layers for Host {
    fn layer_image(&mut self, param: &str, time: f64) -> Option<(usize, usize, Vec<f32>)> {
        self.ask_layer(param, time)
    }
}

/// Read stdin on its own thread: the render thread then waits for a request *with a timeout*,
/// which a bare `read_line` cannot do.
fn spawn_reader() -> Receiver<Result<Req, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if tx.send(serde_json::from_str::<Req>(t).map_err(|e| format!("{e} in {t}"))).is_err() {
                break;
            }
        }
    });
    rx
}

/// A setting in seconds; a missing or unreadable value gives `default`.
fn env_secs(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map_or(default, |v| v as u64)
}

/// How long to wait for the host to hand over a layer image (`MOS_LIGHTWRAP_LAYER_TIMEOUT`).
fn layer_timeout() -> Duration {
    Duration::from_secs(env_secs("MOS_LIGHTWRAP_LAYER_TIMEOUT", 20).max(1))
}

/// How long a layer image stays valid for reuse (`MOS_LIGHTWRAP_LAYER_CACHE_SECS`, 0 = never).
fn layer_cache_ttl() -> Duration {
    Duration::from_secs(env_secs("MOS_LIGHTWRAP_LAYER_CACHE_SECS", 3))
}

fn log_line(msg: &str) {
    if !debug_on() {
        return;
    }
    let path = std::env::temp_dir().join("effectcraft-lightwrap.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{msg}");
    }
}

/// Debug logging and frame dumps are only active while `%TEMP%\lwf_dump` exists.
fn debug_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::temp_dir().join("lwf_dump").exists())
}

/// 8-bit PAM dump with straight alpha, for looking at buffers while debugging.
fn dump_pam(path: &std::path::Path, w: usize, h: usize, px: &[f32]) {
    let mut out = Vec::with_capacity(w * h * 4 + 64);
    out.extend_from_slice(format!("P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n").as_bytes());
    for i in 0..w * h {
        let a = px[i * 4 + 3].clamp(0.0, 1.0);
        for c in 0..3 {
            let v = if a > 1e-4 { px[i * 4 + c] / a } else { 0.0 };
            out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        out.push((a * 255.0).round() as u8);
    }
    let _ = std::fs::write(path, out);
}

trait UnwrapReq {
    fn ok(self) -> Option<Req>;
}

impl UnwrapReq for Result<Req, String> {
    fn ok(self) -> Option<Req> {
        match self {
            Ok(r) => Some(r),
            Err(e) => {
                log_line(&format!("bad request while waiting: {e}"));
                None
            }
        }
    }
}

