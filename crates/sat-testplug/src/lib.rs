//! The crash-test plugin (see `sat_plug::testplug`) built as a CLAP
//! library: rename `libsat_testplug.so` (or `.dylib`, `.dll`) to
//! `sat-crash-test.clap` and load it like any plugin.

use clap_sys::entry::clap_plugin_entry;

/// The entry point CLAP hosts look up.
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static clap_entry: clap_plugin_entry = sat_plug::testplug::ENTRY;
