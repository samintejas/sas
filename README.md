# sat

A digital audio workstation built around grids: every track owns a source grid,
an instrument grid and fx grids, and arrange and session play the same clip
objects. Written in Rust.

The design lives in two documents: the architecture doc (*sat — architecture
(Rust)*) and the clickable prototype canvas (*DAW – first design*). This
repository follows the architecture doc's crate layout and build phases.

## Status

Phase 1 (headless core) is done; its gate passes single-threaded (see below).
Phase 2 has started: the app opens a window with the arrange view and piano
roll from the prototype, drawn with vello, and plays the set through the
default audio device.

| Crate | What it does | State |
|---|---|---|
| `sat-time` | ticks (960 PPQ), tempo map with step and linear ramps, meter map, quantisation | done |
| `sat-doc` | entities, `Op` reducer, transactions, undo with gestures, project bundle with migrations | done |
| `sat-engine` | flat schedule, mixer strips, test synth, automation, loop, lock-free commands, schedule swap with state carry-over, offline render | done (single-threaded) |
| `sat-compile` | document → schedule: track tree, solo, sends, placements and looping clips, live parameter mapping | done |
| `sat-theme` | Dots design tokens (`assets/dots/tokens.json`) generated into a Rust `Palette`, plus spacing, radius and type scales | done |
| `sat-ui` | vello + parley painter, icons, top bar, arrange view, piano roll, hit-testing, headless GPU rendering | in progress |
| `sat-app` | the `sat` binary: winit window, vello surface, cpal output running the engine | in progress |

## Phase 1 gate

`cargo run --release -p sat-compile --example budget` renders a 64-track set
(8 groups, two returns, every track playing 4-note chords through the test
synth with gain automation and two sends) at 48 kHz / 128 samples. On a
4-core cloud VM it averages about 14% of the 2.7 ms block (target ≤ 70%) and
compiles in under 2 ms (target ≤ 10 ms); CI runs it on every push. The same
set runs under the no-allocation check. The engine is still single-threaded:
the worker pool from the architecture doc comes in when real modules make
tracks heavy enough to need it.

## Build phases

Each phase ends with a gate you can run.

1. **Core model + engine** (headless): `sat-doc`, `sat-time`, `sat-engine`, golden-render tests.
   Gate: 64 test tracks inside budget, zero allocations.
2. **Shell + arrange**: app shell, timeline, mixer, groups, loop, autosave. Gate: open, play and mix a project file end to end.
3. **Source grid**: modules, clip dock, patterns, piano roll, drum seq, euclid. Gate: edit a clip in the bottom panel and hear it change.
4. **Instrument + fx grids**: built-in synth, sampler and fx modules, macros. Gate: a full song made with built-in modules only.
5. **Session + recording**: scenes, recorder module, takes, comps, fades, crossfades. Gate: record, comp and crossfade a vocal.
6. **Generative + export**: seeded generators, hand-edit layer, freeze, offline render. Gate: export matches the last playback sample for sample.
7. **CLAP hosting**. Gate: a plugin crash leaves the set playing.

## The document in one paragraph

A project is one immutable `Document` held in persistent maps (`im`), so a
clone is free and undo just keeps earlier versions. Every change is an `Op`
that carries the ids it creates, so op lists replay exactly. `History::apply`
runs a list of ops as one all-or-nothing transaction and reports a `Change`
per op: value changes go to the engine as parameter events, structural ones
trigger a recompile. Projects save as a `name.sat/` bundle whose
`project.sat` is versioned, zstd-compressed JSON.

## Running the app

```sh
cargo run --release -p sat-app            # opens the demo set
cargo run --release -p sat-app -- my.sat  # opens a project bundle
```

Space plays and stops, Home returns to the start, `l` toggles the loop, `m`
and `s` mute and solo the selected track, and Ctrl+Z / Ctrl+Shift+Z undo and
redo. Click a clip to show it in the piano roll, click the ruler to move the
playhead, and click a group's chevron to fold it. Ctrl+scroll zooms and
Shift+scroll moves through time. On Linux you need `libasound2-dev` and
`libxkbcommon-dev` to build.

To render a frame without a window (any Vulkan, Metal or DX12 adapter,
including Mesa's lavapipe):

```sh
cargo run --release -p sat-ui --example screenshot -- arrange.png 2
```

## Listening

```sh
cargo run -p sat-compile --example render_reference -- reference_song.wav
```

renders the reference song that the golden tests null-test against.

## Developing

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Real-time rule: `sat-engine` never allocates, locks or does I/O while
processing. `crates/sat-compile/tests/no_alloc.rs` enforces it by aborting on
any allocation inside the audio path. Golden renders live in
`crates/sat-compile/tests/golden/`; after an intentional sound change,
regenerate them with `SAT_BLESS=1 cargo test -p sat-compile --test golden`.
