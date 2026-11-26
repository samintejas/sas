//! The host process side: loads one CLAP plugin, reports what it is on
//! stdout (one JSON line), then runs it on blocks from the shared-memory
//! region until asked to quit or the engine goes away. Between blocks it
//! answers commands that come as JSON lines on stdin (saving and loading
//! the plugin's state), one JSON line on stdout each.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::ptr;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE, clap_event_header, clap_event_param_value, clap_input_events,
    clap_output_events,
};
use clap_sys::ext::params::{CLAP_EXT_PARAMS, clap_param_info, clap_plugin_params};
use clap_sys::ext::state::{CLAP_EXT_STATE, clap_plugin_state};
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;

use crate::shm::{MAX_PARAMS, Shm};
use crate::{Command, Hello, ParamInfo, Reply};

static ORPHANED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The prefix that names a plugin built into the host.
pub const BUILTIN: &str = "builtin:";

/// Runs the host: `args` are the region's file and the plugin (a `.clap`
/// path or `builtin:crash-test`). Returns the exit code.
pub fn main(args: Vec<String>) -> i32 {
    let [shm, plugin] = args.as_slice() else {
        eprintln!("usage: sat-plughost <shared memory file> <plugin.clap | builtin:crash-test>");
        return 2;
    };
    let shm = match Shm::open(Path::new(shm)) {
        Ok(s) => s,
        Err(e) => return fail(&format!("cannot map {shm}: {e}")),
    };
    // SAFETY: loading a plugin runs its code; that is what this process is for.
    let loaded = match unsafe { Loaded::new(plugin, shm.rate(), shm.max_block()) } {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    // commands come on stdin; when it closes (the app quit or crashed)
    // there is nobody left to play for
    let (tx, commands) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufRead::lines(std::io::stdin().lock()) {
            let Ok(line) = line else { break };
            let cmd = serde_json::from_str::<Command>(line.trim()).map_err(|e| e.to_string());
            if tx.send(cmd).is_err() {
                break;
            }
        }
        ORPHANED.store(true, Ordering::Relaxed);
    });
    let hello = Hello { ok: true, error: None, name: loaded.name.clone(), params: loaded.params.clone() };
    say(&hello);
    run(&shm, loaded, &commands);
    0
}

/// Writes one JSON line to stdout, at once.
fn say<T: serde::Serialize>(msg: &T) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", serde_json::to_string(msg).unwrap_or_default());
    let _ = out.flush();
}

/// Carries out a command between blocks.
fn obey(p: &mut Loaded, cmd: Result<Command, String>) -> Reply {
    let res = cmd.and_then(|cmd| match cmd {
        // SAFETY: between blocks, on the thread that runs the plugin.
        Command::Save(path) => unsafe { p.save_state() }
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| format!("cannot write {path}: {e}"))),
        Command::Load(path) => std::fs::read(&path)
            .map_err(|e| format!("cannot read {path}: {e}"))
            .and_then(|bytes| unsafe { p.load_state(&bytes) }),
    });
    match res {
        Ok(()) => Reply { ok: true, error: None },
        Err(e) => Reply { ok: false, error: Some(e) },
    }
}

fn fail(e: &str) -> i32 {
    say(&Hello { ok: false, error: Some(e.into()), name: String::new(), params: Vec::new() });
    1
}

/// The block loop: wait for a request, process it, answer.
fn run(shm: &Shm, mut p: Loaded, commands: &std::sync::mpsc::Receiver<Result<Command, String>>) {
    let h = shm.header();
    // from `done`, not `req`: the engine may send a block before this starts
    let mut last = h.done.load(Ordering::Acquire);
    let mut seen_params = u32::MAX;
    let mut values = [f32::NAN; MAX_PARAMS];
    let mut idle = Instant::now();
    loop {
        h.heartbeat.fetch_add(1, Ordering::Relaxed);
        if h.quit.load(Ordering::Acquire) != 0 || ORPHANED.load(Ordering::Relaxed) {
            break;
        }
        let req = h.req.load(Ordering::Acquire);
        if req == last {
            if let Ok(cmd) = commands.try_recv() {
                let reply = obey(&mut p, cmd);
                say(&reply);
                continue;
            }
            // spin briefly, then yield, then nap while nothing is playing
            let waited = idle.elapsed();
            if waited < Duration::from_micros(200) {
                std::hint::spin_loop();
            } else if waited < Duration::from_millis(20) {
                std::thread::yield_now();
            } else {
                std::thread::sleep(Duration::from_micros(250));
            }
            continue;
        }
        // parameter changes become events at the start of the block
        p.events.clear();
        let seq = h.param_seq.load(Ordering::Acquire);
        if seq != seen_params {
            seen_params = seq;
            let n = (h.nparams.load(Ordering::Relaxed) as usize).min(p.params.len()).min(MAX_PARAMS);
            for (i, v) in values.iter_mut().enumerate().take(n) {
                let x = f32::from_bits(h.params[i].load(Ordering::Relaxed));
                // NaN: the engine has not sent this one
                if !x.is_nan() && x.to_bits() != v.to_bits() {
                    *v = x;
                    // the engine sends 0–1; the plugin wants its own range
                    let info = &p.params[i];
                    let plain = info.min + (x.clamp(0.0, 1.0) as f64) * (info.max - info.min);
                    p.events.push(param_event(info.id, plain));
                }
            }
        }
        let frames = (h.frames.load(Ordering::Relaxed) as usize).min(shm.max_block());
        // SAFETY: the engine wrote this block and waits until `done`.
        unsafe { p.process(shm, frames) };
        h.done.store(req, Ordering::Release);
        last = req;
        idle = Instant::now();
    }
    // SAFETY: the plugin is ours and no longer processing.
    unsafe { p.close() };
}

fn param_event(id: u32, value: f64) -> clap_event_param_value {
    clap_event_param_value {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_param_value>() as u32,
            time: 0,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_PARAM_VALUE,
            flags: 0,
        },
        param_id: id,
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    }
}

struct HostData;

static HOST_NAME: &CStr = c"sat";
static HOST: HostStatic = HostStatic(clap_host {
    clap_version: CLAP_VERSION,
    host_data: ptr::null_mut(),
    name: HOST_NAME.as_ptr(),
    vendor: c"sat".as_ptr(),
    url: c"https://github.com/samintejas/sas".as_ptr(),
    version: c"0.1.0".as_ptr(),
    get_extension: Some(host_extension),
    request_restart: Some(host_noop),
    request_process: Some(host_noop),
    request_callback: Some(host_noop),
});

struct HostStatic(clap_host);
// SAFETY: the host struct is read-only and its pointers are 'static.
unsafe impl Sync for HostStatic {}

unsafe extern "C" fn host_extension(_h: *const clap_host, _id: *const c_char) -> *const c_void {
    ptr::null()
}

unsafe extern "C" fn host_noop(_h: *const clap_host) {}

unsafe extern "C" fn events_size(list: *const clap_input_events) -> u32 {
    // SAFETY: ctx is the Vec this host passed in for this call.
    unsafe { (*((*list).ctx as *const Vec<clap_event_param_value>)).len() as u32 }
}

unsafe extern "C" fn events_get(list: *const clap_input_events, i: u32) -> *const clap_event_header {
    // SAFETY: as in events_size.
    let v = unsafe { &*((*list).ctx as *const Vec<clap_event_param_value>) };
    v.get(i as usize).map_or(ptr::null(), |e| &e.header as *const _)
}

unsafe extern "C" fn events_push(_list: *const clap_output_events, _e: *const clap_event_header) -> bool {
    true
}

unsafe extern "C" fn stream_write(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64 {
    // SAFETY: ctx is the Vec save_state passed in, and the plugin gives
    // `size` readable bytes.
    unsafe {
        let out = &mut *((*stream).ctx as *mut Vec<u8>);
        out.extend_from_slice(std::slice::from_raw_parts(buffer as *const u8, size as usize));
    }
    size as i64
}

/// A saved state being read back.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

unsafe extern "C" fn stream_read(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    // SAFETY: ctx is the Reader load_state passed in, and the plugin gives
    // room for `size` bytes.
    unsafe {
        let r = &mut *((*stream).ctx as *mut Reader);
        let n = (size as usize).min(r.bytes.len() - r.at);
        ptr::copy_nonoverlapping(r.bytes[r.at..].as_ptr(), buffer as *mut u8, n);
        r.at += n;
        n as i64
    }
}

/// A loaded, active plugin.
struct Loaded {
    _lib: Option<libloading::Library>,
    entry: *const clap_plugin_entry,
    plugin: *const clap_plugin,
    name: String,
    params: Vec<ParamInfo>,
    events: Vec<clap_event_param_value>,
    steady: i64,
    _data: HostData,
}

impl Loaded {
    /// Loads `spec` and activates its first plugin.
    ///
    /// # Safety
    /// Runs the plugin's code.
    unsafe fn new(spec: &str, rate: f64, max_block: usize) -> Result<Loaded, String> {
        let (lib, entry): (Option<libloading::Library>, *const clap_plugin_entry) = match spec.strip_prefix(BUILTIN) {
            Some("crash-test") => (None, &crate::testplug::ENTRY),
            Some(other) => return Err(format!("no built-in plugin {other}")),
            None => {
                // SAFETY: per the caller.
                let lib = unsafe { libloading::Library::new(spec) }.map_err(|e| format!("cannot load {spec}: {e}"))?;
                let sym: *const clap_plugin_entry = unsafe {
                    *lib.get::<*const clap_plugin_entry>(b"clap_entry\0")
                        .map_err(|e| format!("{spec} is not a CLAP plugin: {e}"))?
                };
                (Some(lib), sym)
            }
        };
        if entry.is_null() {
            return Err("the plugin has no entry".into());
        }
        let path = CString::new(spec).unwrap_or_default();
        // SAFETY: a CLAP entry, used as the spec says (init, factory, create).
        unsafe {
            let e = &*entry;
            if !e.init.is_some_and(|f| f(path.as_ptr())) {
                return Err("the plugin failed to initialise".into());
            }
            let factory =
                e.get_factory.map_or(ptr::null(), |f| f(CLAP_PLUGIN_FACTORY_ID.as_ptr())) as *const clap_plugin_factory;
            if factory.is_null() {
                return Err("the plugin has no plugin factory".into());
            }
            let f = &*factory;
            if f.get_plugin_count.map_or(0, |c| c(factory)) == 0 {
                return Err("the plugin file holds no plugins".into());
            }
            let desc = f.get_plugin_descriptor.map_or(ptr::null(), |d| d(factory, 0));
            if desc.is_null() {
                return Err("the plugin has no descriptor".into());
            }
            let name = cstr((*desc).name);
            let plugin = f.create_plugin.map_or(ptr::null(), |c| c(factory, &HOST.0, (*desc).id));
            if plugin.is_null() {
                return Err(format!("{name} could not be created"));
            }
            let pl = &*plugin;
            if !pl.init.is_some_and(|i| i(plugin)) {
                return Err(format!("{name} failed to initialise"));
            }
            let params = read_params(plugin);
            if !pl.activate.is_some_and(|a| a(plugin, rate, 1, max_block as u32)) {
                return Err(format!("{name} failed to activate"));
            }
            if let Some(s) = pl.start_processing {
                s(plugin);
            }
            Ok(Loaded {
                _lib: lib,
                entry,
                plugin,
                name,
                params,
                events: Vec::with_capacity(MAX_PARAMS),
                steady: 0,
                _data: HostData,
            })
        }
    }

    /// Runs one block through the plugin, from the region's input buffers
    /// to its output buffers.
    ///
    /// # Safety
    /// The engine must have written the input and be waiting.
    unsafe fn process(&mut self, shm: &Shm, frames: usize) {
        let mut ins = [shm.buffer(0), shm.buffer(1)];
        let mut outs = [shm.buffer(2), shm.buffer(3)];
        let input = clap_audio_buffer {
            data32: ins.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let mut output = clap_audio_buffer {
            data32: outs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let in_events = clap_input_events {
            ctx: &self.events as *const _ as *mut c_void,
            size: Some(events_size),
            get: Some(events_get),
        };
        let out_events = clap_output_events { ctx: ptr::null_mut(), try_push: Some(events_push) };
        let process = clap_process {
            steady_time: self.steady,
            frames_count: frames as u32,
            transport: ptr::null(),
            audio_inputs: &input,
            audio_outputs: &mut output,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &in_events,
            out_events: &out_events,
        };
        // SAFETY: an active plugin, buffers of `frames` frames.
        unsafe {
            if let Some(p) = (*self.plugin).process {
                p(self.plugin, &process);
            }
        }
        self.steady += frames as i64;
    }

    /// The plugin's state extension, if it has one.
    ///
    /// # Safety
    /// The plugin must be live.
    unsafe fn state_ext(&self) -> Option<&'static clap_plugin_state> {
        // SAFETY: per the caller; an extension lives as long as the plugin.
        unsafe {
            let ext = (*self.plugin).get_extension.map_or(ptr::null(), |g| g(self.plugin, CLAP_EXT_STATE.as_ptr()))
                as *const clap_plugin_state;
            ext.as_ref()
        }
    }

    /// The plugin's state, as it saves it.
    ///
    /// # Safety
    /// Between blocks, on the thread that runs the plugin.
    unsafe fn save_state(&mut self) -> Result<Vec<u8>, String> {
        // SAFETY: per the caller.
        let Some(save) = unsafe { self.state_ext() }.and_then(|e| e.save) else {
            return Err(format!("{} has no state to save", self.name));
        };
        let mut bytes: Vec<u8> = Vec::new();
        let stream = clap_ostream { ctx: &mut bytes as *mut _ as *mut c_void, write: Some(stream_write) };
        // SAFETY: a live plugin and a stream valid for the call.
        if unsafe { save(self.plugin, &stream) } {
            Ok(bytes)
        } else {
            Err(format!("{} could not save its state", self.name))
        }
    }

    /// Gives the plugin a state it saved.
    ///
    /// # Safety
    /// As [`Loaded::save_state`].
    unsafe fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: per the caller.
        let Some(load) = unsafe { self.state_ext() }.and_then(|e| e.load) else {
            return Err(format!("{} cannot load a state", self.name));
        };
        let mut reader = Reader { bytes, at: 0 };
        let stream = clap_istream { ctx: &mut reader as *mut _ as *mut c_void, read: Some(stream_read) };
        // SAFETY: a live plugin and a stream valid for the call.
        if unsafe { load(self.plugin, &stream) } {
            Ok(())
        } else {
            Err(format!("{} rejected the saved state", self.name))
        }
    }

    /// Stops, deactivates and destroys the plugin.
    ///
    /// # Safety
    /// No further calls into the plugin.
    unsafe fn close(self) {
        // SAFETY: the plugin and entry are live until here.
        unsafe {
            let pl = &*self.plugin;
            if let Some(s) = pl.stop_processing {
                s(self.plugin);
            }
            if let Some(d) = pl.deactivate {
                d(self.plugin);
            }
            if let Some(d) = pl.destroy {
                d(self.plugin);
            }
            if let Some(d) = (*self.entry).deinit {
                d();
            }
        }
    }
}

/// The plugin's parameters, through its params extension.
///
/// # Safety
/// `plugin` must be initialised.
unsafe fn read_params(plugin: *const clap_plugin) -> Vec<ParamInfo> {
    // SAFETY: per the caller.
    unsafe {
        let ext = (*plugin).get_extension.map_or(ptr::null(), |g| g(plugin, CLAP_EXT_PARAMS.as_ptr()))
            as *const clap_plugin_params;
        if ext.is_null() {
            return Vec::new();
        }
        let e = &*ext;
        let (Some(count), Some(get)) = (e.count, e.get_info) else { return Vec::new() };
        (0..count(plugin))
            .filter_map(|i| {
                let mut info: clap_param_info = std::mem::zeroed();
                get(plugin, i, &mut info).then(|| ParamInfo {
                    id: info.id,
                    name: cstr(info.name.as_ptr()),
                    min: info.min_value,
                    max: info.max_value,
                    default: info.default_value,
                })
            })
            .collect()
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: CLAP strings are nul-terminated.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}
