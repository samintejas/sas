//! CLAP plugin hosting.
//!
//! Plugins never load in the app. Each runs in a host process (the
//! `sat-plughost` binary, or the app re-run with `--plughost`) and swaps
//! audio with the engine through a shared-memory region in the same block,
//! so hosting adds no latency. The engine writes a block's input, wakes the
//! host and waits for its output within a budget; a host that dies or
//! hangs leaves its node silent and the rest of the set playing.
//!
//! Control goes as JSON lines: the host says what the plugin is called and
//! what parameters it has when it has loaded it, then answers commands on
//! its stdin (save or load the plugin's state through a file) between
//! blocks, one line each.
//!
//! Offline renders use a [`PlugHost::offline_processor`], which waits for
//! each block as long as the plugin needs instead of dropping it.
//!
//! [`testplug`] is a crash-test plugin built into the host (as
//! `builtin:crash-test`).

pub mod host;
pub mod shm;
pub mod testplug;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use shm::{MAX_PARAMS, Shm};

/// What a host reports once it has loaded its plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    /// Loaded and running.
    pub ok: bool,
    /// Why not.
    pub error: Option<String>,
    /// The plugin's name.
    pub name: String,
    /// Its parameters, in order.
    pub params: Vec<ParamInfo>,
}

/// A command to a host, sent as one JSON line on its stdin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Command {
    /// Write the plugin's state to this file.
    Save(String),
    /// Give the plugin the state in this file.
    Load(String),
}

/// A host's answer to a [`Command`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    /// Done.
    pub ok: bool,
    /// Why not.
    pub error: Option<String>,
}

/// One plugin parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamInfo {
    /// CLAP parameter id.
    pub id: u32,
    /// Name.
    pub name: String,
    /// Lowest value.
    pub min: f64,
    /// Highest value.
    pub max: f64,
    /// Default.
    pub default: f64,
}

/// How a host is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Running.
    Running,
    /// Stuck inside the plugin for over a second: its node is silent.
    Hung,
    /// The process ended.
    Crashed,
}

const RUNNING: u8 = 0;
const CRASHED: u8 = 2;

/// A host whose loop has not moved for this long is hung.
pub const HANG_TIME: Duration = Duration::from_secs(1);

/// How long a host may take to load its plugin.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a host may take to answer a command.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// How long an offline render waits for one block before giving up on the
/// plugin (it is hung).
pub const OFFLINE_BUDGET: Duration = Duration::from_secs(5);

/// A running host process, owned by the app (not the audio thread).
pub struct PlugHost {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    /// Lines from the host's stdout.
    lines: std::sync::mpsc::Receiver<String>,
    shm: Arc<Shm>,
    state: Arc<AtomicU8>,
    /// The heartbeat last seen, and when it last moved.
    beat: (u64, Instant),
    /// What the host reported.
    pub hello: Hello,
}

static NEXT: AtomicU32 = AtomicU32::new(0);

impl PlugHost {
    /// Starts `exe` (with `args` before the host's own, such as
    /// `--plughost`) hosting `plugin` for blocks of up to `max_block`
    /// frames at `rate`, and waits until it has loaded it.
    pub fn spawn(exe: &Path, args: &[&str], plugin: &str, rate: f64, max_block: usize) -> Result<PlugHost, String> {
        let name = format!("sat-plug-{}-{}.shm", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        let shm = Arc::new(Shm::create(&path, max_block, rate).map_err(|e| format!("shared memory: {e}"))?);
        let mut child = std::process::Command::new(exe)
            .args(args)
            .arg(&path)
            .arg(plugin)
            // the host quits when this pipe closes, even if the app crashed
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
        let out = child.stdout.take().ok_or("no stdout")?;
        let stdin = child.stdin.take();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let line = match rx.recv_timeout(LOAD_TIMEOUT) {
            Ok(l) => l,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{plugin} did not load within {} s", LOAD_TIMEOUT.as_secs()));
            }
        };
        let hello: Hello = match serde_json::from_str(line.trim()) {
            Ok(h) => h,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{plugin}: the host stopped while loading it"));
            }
        };
        if !hello.ok {
            let _ = child.wait();
            return Err(hello.error.unwrap_or_else(|| "the plugin failed to load".into()));
        }
        shm.header().nparams.store(hello.params.len().min(MAX_PARAMS) as u32, Ordering::Relaxed);
        let beat = (shm.header().heartbeat.load(Ordering::Relaxed), Instant::now());
        Ok(PlugHost { child, stdin, lines: rx, shm, state: Arc::new(AtomicU8::new(RUNNING)), beat, hello })
    }

    /// A handle for the engine to process audio through this host.
    pub fn processor(&self) -> Processor {
        Processor {
            shm: self.shm.clone(),
            state: self.state.clone(),
            sent: [f32::NAN; MAX_PARAMS],
            first: true,
            offline: false,
        }
    }

    /// A handle for offline renders: it waits up to [`OFFLINE_BUDGET`] for
    /// each block, so a slow plugin is rendered, not dropped.
    pub fn offline_processor(&self) -> Processor {
        Processor { offline: true, ..self.processor() }
    }

    /// Sends `cmd` and waits for the answer.
    fn command(&mut self, cmd: &Command) -> Result<(), String> {
        if self.status() == Status::Crashed {
            return Err(format!("{} has crashed", self.hello.name));
        }
        // an answer left over from a command that timed out
        while self.lines.try_recv().is_ok() {}
        let line = serde_json::to_string(cmd).map_err(|e| e.to_string())?;
        let stdin = self.stdin.as_mut().ok_or("the host has no stdin")?;
        writeln!(stdin, "{line}")
            .and_then(|_| stdin.flush())
            .map_err(|e| format!("the host stopped listening: {e}"))?;
        let answer = self
            .lines
            .recv_timeout(COMMAND_TIMEOUT)
            .map_err(|_| format!("{} did not answer within {} s", self.hello.name, COMMAND_TIMEOUT.as_secs()))?;
        let reply: Reply = serde_json::from_str(answer.trim()).map_err(|e| format!("a garbled answer: {e}"))?;
        if reply.ok { Ok(()) } else { Err(reply.error.unwrap_or_else(|| "the command failed".into())) }
    }

    /// Writes the plugin's state to `path`.
    pub fn save_state(&mut self, path: &Path) -> Result<(), String> {
        self.command(&Command::Save(path.to_string_lossy().into_owned()))
    }

    /// Gives the plugin the state saved in `path`.
    pub fn load_state(&mut self, path: &Path) -> Result<(), String> {
        self.command(&Command::Load(path.to_string_lossy().into_owned()))
    }

    /// How the host is doing: it crashed once its process ended, and hangs
    /// while its loop has not moved for [`HANG_TIME`] (stuck in the plugin).
    pub fn status(&mut self) -> Status {
        if self.state.load(Ordering::Relaxed) != CRASHED && !matches!(self.child.try_wait(), Ok(None)) {
            self.state.store(CRASHED, Ordering::Relaxed);
        }
        if self.state.load(Ordering::Relaxed) == CRASHED {
            return Status::Crashed;
        }
        let beat = self.shm.header().heartbeat.load(Ordering::Relaxed);
        if beat != self.beat.0 {
            self.beat = (beat, Instant::now());
        }
        if self.beat.1.elapsed() >= HANG_TIME { Status::Hung } else { Status::Running }
    }

    /// Blocks the host did not answer in time so far (each was a dropout
    /// in its track).
    pub fn dropouts(&self) -> u32 {
        self.shm.header().dropouts.load(Ordering::Relaxed)
    }

    /// The region's file (for tests).
    pub fn shm_path(&self) -> PathBuf {
        self.shm.path().to_path_buf()
    }
}

impl Drop for PlugHost {
    fn drop(&mut self) {
        self.shm.header().quit.store(1, Ordering::Release);
        // a well-behaved host quits within a few ms; a hung one is killed
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(200) {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The engine's handle on a host: runs a block through it within a time
/// budget, or writes silence. It never allocates.
pub struct Processor {
    shm: Arc<Shm>,
    state: Arc<AtomicU8>,
    sent: [f32; MAX_PARAMS],
    first: bool,
    offline: bool,
}

impl Processor {
    /// How long a block may take: half its own length, within 0.5–20 ms
    /// (offline, [`OFFLINE_BUDGET`]).
    fn budget(&self, frames: usize) -> Duration {
        if self.offline {
            return OFFLINE_BUDGET;
        }
        let secs = frames as f64 / self.shm.rate().max(1.0) * 0.5;
        Duration::from_secs_f64(secs.clamp(0.0005, 0.020))
    }

    /// Runs `l` and `r` through the plugin in place, sending changed
    /// `params` first. Returns false and leaves silence when the host could
    /// not answer within the budget: a late block is a dropout, the host
    /// carries on and the next block tries again once it has caught up, and
    /// a dead host costs no waiting after its first missed block.
    pub fn run(&mut self, l: &mut [f32], r: &mut [f32], params: &[f32]) -> bool {
        let n = l.len().min(r.len());
        let h = self.shm.header();
        // a crashed host, or one still busy with an earlier block: silence
        if self.state.load(Ordering::Relaxed) != RUNNING
            || h.done.load(Ordering::Acquire) != h.req.load(Ordering::Relaxed)
        {
            l.fill(0.0);
            r.fill(0.0);
            return false;
        }
        let max = self.shm.max_block();
        let nparams = (h.nparams.load(Ordering::Relaxed) as usize).min(params.len());
        let mut changed = self.first;
        for (i, (v, sent)) in params.iter().zip(self.sent.iter_mut()).take(nparams).enumerate() {
            if *v != *sent {
                *sent = *v;
                h.params[i].store(v.to_bits(), Ordering::Relaxed);
                changed = true;
            }
        }
        if changed {
            self.first = false;
            h.param_seq.fetch_add(1, Ordering::Release);
        }
        let mut done = 0;
        while done < n {
            let k = (n - done).min(max);
            // SAFETY: the host only reads the input after `req` changes and
            // only writes the output before setting `done`; we checked it
            // has finished the last block, and wait for this one.
            unsafe {
                std::ptr::copy_nonoverlapping(l[done..].as_ptr(), self.shm.buffer(0), k);
                std::ptr::copy_nonoverlapping(r[done..].as_ptr(), self.shm.buffer(1), k);
            }
            h.frames.store(k as u32, Ordering::Relaxed);
            let req = h.req.load(Ordering::Relaxed).wrapping_add(1);
            h.req.store(req, Ordering::Release);
            let deadline = Instant::now() + self.budget(k);
            let mut spins = 0u32;
            while h.done.load(Ordering::Acquire) != req {
                if Instant::now() >= deadline {
                    h.dropouts.fetch_add(1, Ordering::Relaxed);
                    l.fill(0.0);
                    r.fill(0.0);
                    return false;
                }
                spins += 1;
                if spins < 4096 {
                    std::hint::spin_loop();
                } else if self.offline {
                    std::thread::sleep(Duration::from_micros(20));
                } else {
                    std::thread::yield_now();
                }
            }
            // SAFETY: the host has finished this block.
            unsafe {
                std::ptr::copy_nonoverlapping(self.shm.buffer(2), l[done..].as_mut_ptr(), k);
                std::ptr::copy_nonoverlapping(self.shm.buffer(3), r[done..].as_mut_ptr(), k);
            }
            done += k;
        }
        true
    }
}

impl sat_engine::External for Processor {
    fn process(&mut self, l: &mut [f32], r: &mut [f32], params: &[f32]) {
        self.run(l, r, params);
    }

    fn boxed_clone(&self) -> Box<dyn sat_engine::External> {
        Box::new(Processor {
            shm: self.shm.clone(),
            state: self.state.clone(),
            sent: self.sent,
            first: true,
            offline: self.offline,
        })
    }
}
