//! The plugin host process: `sat-plughost <shared memory file> <plugin>`.
fn main() {
    std::process::exit(sat_plug::host::main(std::env::args().skip(1).collect()));
}
