//! The crash-test plugin: a CLAP gain effect that can also panic, abort or
//! hang on request, to prove a misbehaving plugin cannot take the set down.
//!
//! It is built into `sat-plughost` (load it as `builtin:crash-test`) and is
//! also what the `sat-testplug` library exports as its `clap_entry`, so the
//! same code is loaded through `dlopen` too.
//!
//! Parameters: 0 `gain` (dB, -60 to 12), 1 `mode` (0 run, 1 panic, 2 abort,
//! 3 hang), 2 `leak` (MB to leak per block, 0 to 8).

use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE, clap_event_param_value, clap_input_events, clap_output_events,
};
use clap_sys::ext::params::{CLAP_EXT_PARAMS, clap_param_info, clap_plugin_params};
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{CLAP_PROCESS_CONTINUE, CLAP_PROCESS_ERROR, clap_process, clap_process_status};
use clap_sys::version::CLAP_VERSION;

/// The plugin's id.
pub const ID: &CStr = c"sat.test.crash";

/// Parameter ids.
pub const GAIN: clap_id = 0;
/// See [`GAIN`].
pub const MODE: clap_id = 1;
/// See [`GAIN`].
pub const LEAK: clap_id = 2;

/// Modes.
pub const RUN: f64 = 0.0;
/// See [`RUN`].
pub const PANIC: f64 = 1.0;
/// See [`RUN`].
pub const ABORT: f64 = 2.0;
/// See [`RUN`].
pub const HANG: f64 = 3.0;

struct Features([*const c_char; 3]);
unsafe impl Sync for Features {}
static FEATURES: Features = Features([c"audio-effect".as_ptr(), c"utility".as_ptr(), ptr::null()]);

static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: ID.as_ptr(),
    name: c"sat crash test".as_ptr(),
    vendor: c"sat".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"1.0.0".as_ptr(),
    description: c"A gain that can panic, abort, hang or leak on request".as_ptr(),
    features: &FEATURES.0 as *const _ as *const *const c_char,
};

/// The entry point a host looks up.
pub static ENTRY: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_get_factory),
};

static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_info),
    get_value: Some(params_value),
    value_to_text: None,
    text_to_value: None,
    flush: Some(params_flush),
};

const INFO: [(&CStr, f64, f64, f64); 3] =
    [(c"gain", -60.0, 12.0, 0.0), (c"mode", 0.0, 3.0, 0.0), (c"leak", 0.0, 8.0, 0.0)];

struct Plugin {
    clap: clap_plugin,
    values: [f64; 3],
    leaked: Vec<Vec<u8>>,
}

unsafe extern "C" fn entry_init(_path: *const c_char) -> bool {
    true
}

unsafe extern "C" fn entry_deinit() {}

unsafe extern "C" fn entry_get_factory(id: *const c_char) -> *const c_void {
    // SAFETY: the host passes a nul-terminated id.
    if !id.is_null() && unsafe { CStr::from_ptr(id) } == CLAP_PLUGIN_FACTORY_ID {
        &FACTORY as *const _ as *const c_void
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn factory_count(_f: *const clap_plugin_factory) -> u32 {
    1
}

unsafe extern "C" fn factory_descriptor(_f: *const clap_plugin_factory, index: u32) -> *const clap_plugin_descriptor {
    if index == 0 { &DESCRIPTOR } else { ptr::null() }
}

unsafe extern "C" fn factory_create(
    _f: *const clap_plugin_factory,
    _host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    // SAFETY: the host passes a nul-terminated id.
    if id.is_null() || unsafe { CStr::from_ptr(id) } != ID {
        return ptr::null();
    }
    let p = Box::new(Plugin {
        clap: clap_plugin {
            desc: &DESCRIPTOR,
            plugin_data: ptr::null_mut(),
            init: Some(plugin_init),
            destroy: Some(plugin_destroy),
            activate: Some(plugin_activate),
            deactivate: Some(plugin_noop),
            start_processing: Some(plugin_start),
            stop_processing: Some(plugin_noop),
            reset: Some(plugin_noop),
            process: Some(plugin_process),
            get_extension: Some(plugin_extension),
            on_main_thread: Some(plugin_noop),
        },
        values: INFO.map(|i| i.3),
        leaked: Vec::new(),
    });
    let raw = Box::into_raw(p);
    // SAFETY: just allocated; clap is the first field, so the plugin
    // pointer and the clap_plugin pointer are the same.
    unsafe {
        (*raw).clap.plugin_data = raw as *mut c_void;
        &(*raw).clap
    }
}

/// The plugin behind a `clap_plugin` pointer.
///
/// # Safety
/// `p` must come from [`factory_create`] and not be destroyed.
unsafe fn this<'a>(p: *const clap_plugin) -> &'a mut Plugin {
    // SAFETY: per the caller; plugin_data points at the boxed Plugin.
    unsafe { &mut *((*p).plugin_data as *mut Plugin) }
}

unsafe extern "C" fn plugin_init(_p: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn plugin_destroy(p: *const clap_plugin) {
    // SAFETY: created by factory_create with Box::into_raw.
    unsafe { drop(Box::from_raw((*p).plugin_data as *mut Plugin)) };
}

unsafe extern "C" fn plugin_activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}

unsafe extern "C" fn plugin_start(_p: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn plugin_noop(_p: *const clap_plugin) {}

unsafe extern "C" fn plugin_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    // SAFETY: the host passes a nul-terminated id.
    if !id.is_null() && unsafe { CStr::from_ptr(id) } == CLAP_EXT_PARAMS {
        &PARAMS as *const _ as *const c_void
    } else {
        ptr::null()
    }
}

/// Applies the parameter events in `list`.
///
/// # Safety
/// `list` must be null or a valid host event list.
unsafe fn read_events(pl: &mut Plugin, list: *const clap_input_events) {
    if list.is_null() {
        return;
    }
    // SAFETY: per the caller.
    let l = unsafe { &*list };
    let (Some(size), Some(get)) = (l.size, l.get) else { return };
    // SAFETY: the host's own list callbacks.
    for i in 0..unsafe { size(list) } {
        let e = unsafe { get(list, i) };
        if e.is_null() {
            continue;
        }
        // SAFETY: a header the host owns for this call.
        let h = unsafe { &*e };
        if h.space_id == CLAP_CORE_EVENT_SPACE_ID && h.type_ == CLAP_EVENT_PARAM_VALUE {
            // SAFETY: the type says it is a param value event.
            let v = unsafe { &*(e as *const clap_event_param_value) };
            if let Some(x) = pl.values.get_mut(v.param_id as usize) {
                let (_, lo, hi, _) = INFO[v.param_id as usize];
                *x = v.value.clamp(lo, hi);
            }
        }
    }
}

unsafe extern "C" fn plugin_process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    // SAFETY: the host calls with its own plugin and a valid process struct.
    let (pl, pr) = unsafe { (this(p), &*process) };
    unsafe { read_events(pl, pr.in_events) };
    match pl.values[MODE as usize].round() {
        m if m == PANIC => panic!("crash test: panic in process"),
        m if m == ABORT => std::process::abort(),
        m if m == HANG => loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        },
        _ => {}
    }
    let leak = pl.values[LEAK as usize].round() as usize;
    if leak > 0 {
        pl.leaked.push(vec![1u8; leak << 20]);
    }
    if pr.audio_inputs_count < 1 || pr.audio_outputs_count < 1 {
        return CLAP_PROCESS_ERROR;
    }
    let g = 10f64.powf(pl.values[GAIN as usize] / 20.0) as f32;
    let n = pr.frames_count as usize;
    // SAFETY: the host gives one input and one output buffer of `n` frames.
    let (input, output): (&clap_audio_buffer, &clap_audio_buffer) = unsafe { (&*pr.audio_inputs, &*pr.audio_outputs) };
    let ch = input.channel_count.min(output.channel_count) as usize;
    for c in 0..ch {
        // SAFETY: as above, `ch` channels of `n` frames each.
        let (a, b) = unsafe { (*input.data32.add(c), *output.data32.add(c)) };
        for k in 0..n {
            unsafe { *b.add(k) = *a.add(k) * g };
        }
    }
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn params_count(_p: *const clap_plugin) -> u32 {
    INFO.len() as u32
}

unsafe extern "C" fn params_info(_p: *const clap_plugin, index: u32, info: *mut clap_param_info) -> bool {
    let Some((name, lo, hi, def)) = INFO.get(index as usize) else { return false };
    // SAFETY: the host passes a writable info struct.
    let i = unsafe { &mut *info };
    i.id = index;
    i.flags = 0;
    i.cookie = ptr::null_mut();
    i.name = [0; clap_sys::string_sizes::CLAP_NAME_SIZE];
    for (d, s) in i.name.iter_mut().zip(name.to_bytes()) {
        *d = *s as c_char;
    }
    i.module = [0; clap_sys::string_sizes::CLAP_PATH_SIZE];
    i.min_value = *lo;
    i.max_value = *hi;
    i.default_value = *def;
    true
}

unsafe extern "C" fn params_value(p: *const clap_plugin, id: clap_id, out: *mut f64) -> bool {
    // SAFETY: the host's plugin and a writable value.
    let pl = unsafe { this(p) };
    match pl.values.get(id as usize) {
        Some(v) => {
            unsafe { *out = *v };
            true
        }
        None => false,
    }
}

unsafe extern "C" fn params_flush(
    p: *const clap_plugin,
    input: *const clap_input_events,
    _out: *const clap_output_events,
) {
    // SAFETY: the host's plugin and event list.
    unsafe { read_events(this(p), input) };
}
