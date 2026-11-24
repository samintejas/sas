# sat

A digital audio workstation built around grids: every track owns a source grid,
an instrument grid and fx grids, and arrange and session play the same clip
objects. Written in Rust.

The design lives in two documents: the architecture doc (*sat — architecture
(Rust)*) and the clickable prototype canvas (*DAW – first design*). This
repository follows the architecture doc's crate layout and build phases.

## Status

Phase 1 (headless core) is done; its gate passes single-threaded (see below).
Phase 2 (shell + arrange) is done: the app opens, plays, mixes and saves a
project file end to end. Phase 3 (source grid) has started: notes are edited
in the piano roll, the drum sequencer or the euclid generator (with hand edits
over it) and heard while the set plays, and each track's source grid shows its
modules and the clip dock, where cables decide which module feeds each clip. The arrange view, piano roll and mixer from
the prototype are drawn with vello; the engine plays through the default
audio device and feeds live per-track meters.

| Crate | What it does | State |
|---|---|---|
| `sat-time` | ticks (960 PPQ), tempo map with step and linear ramps, meter map, quantisation | done |
| `sat-doc` | entities, `Op` reducer, transactions, undo with gestures, project bundle with migrations | done |
| `sat-engine` | flat schedule, mixer strips, test synth, automation, loop, lock-free commands, schedule swap with state carry-over, offline render | done (single-threaded) |
| `sat-compile` | document → schedule: track tree, solo, sends, placements and looping clips, live parameter mapping | done |
| `sat-modules` | built-in source modules: which editor each opens, the euclid generator, the drum kit, the hand-edit layer (toggle, clear, bake) | in progress |
| `sat-theme` | Dots design tokens (`assets/dots/tokens.json`) generated into a Rust `Palette`, plus spacing, radius and type scales | done |
| `sat-ui` | vello + parley painter, top bar, arrange view, source grid, piano roll, step editors, mixer, shared controls (faders, knobs, meters), hit-testing, headless GPU rendering | in progress |
| `sat-app` | the `sat` binary: winit window, vello surface, cpal output running the engine, mixing gestures with undo, open / save / autosave | in progress |

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

| Keys and mouse | Does |
|---|---|
| Space, Home, `l` | play / stop, back to the start, loop on / off |
| Tab, or the panel tabs | switch the bottom panel between the clip and the mixer |
| drag a fader or knob | set level, pan or a send (Shift for fine steps, double-click to reset); one drag is one undo step |
| `m` / `s` buttons, or the keys | mute / solo (the keys act on the selected track); the round button arms recording |
| Ctrl+Z, Ctrl+Shift+Z | undo, redo |
| Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+N, Ctrl+Q | open, save, save as, new, quit (Cmd on macOS) |
| drop a `.sat` folder on the window | open it |
| click a clip, the ruler, a group's chevron | edit it in the piano roll, move the playhead, fold the group |
| piano roll: click empty space (draw tool) | draw a note; keep dragging to set its length |
| piano roll: drag a note, its right edge, the velocity lane | move it (time and pitch), resize it, set velocity; the selection moves together |
| piano roll: double-click a note, Delete, arrows, Ctrl+A, Esc | delete, delete the selection, nudge by the grid or transpose (Shift: octave), select all, clear |
| piano roll: select tool, shift-click, the grid and quantise buttons | box-select, add to the selection, change the grid (Alt drags off it), snap starts to the grid |
| top bar view buttons, "open source grid", "‹ arrange" | switch between the arrangement and the selected track's source grid |
| source grid: drag a module's header, drag a parameter row, drag the canvas or wheel | move the module, change the value, pan |
| source grid: drag from a notes out to a clip's inlet, drag a wire off an inlet | wire that clip to the module (replacing its feed), rewire or unplug it |
| source grid: + module, the dock's +, Delete, Esc | add a piano roll, drum seq or euclid; add a one-bar clip after the track's last one; remove the selected module; deselect |
| drum seq: click a step, Shift-click it, the page buttons | toggle it, cycle its velocity (normal, accent, ghost), show another bar |
| euclid: drag a knob, click a trig | change steps, hits, rotation, accents, note, rate, gate or velocity; add or remove a hand edit over the generator |
| euclid: clear, bake, generative / fixed | drop the hand edits, fix generator and edits into plain steps, switch between them |
| wheel, Shift+wheel, Ctrl+wheel | scroll tracks (or the mixer), scroll time, zoom |

Edits are autosaved every two minutes, and on quit or before opening another
project, into the bundle's `backups/` folder (the last 10 are kept); a project
that was never saved autosaves under your data folder (`sat/autosave/`). On
Linux you need `libasound2-dev` and `libxkbcommon-dev` to build; the file
dialogs use the desktop portal, with `zenity` as a fallback.

To render a frame without a window (any Vulkan, Metal or DX12 adapter,
including Mesa's lavapipe):

```sh
cargo run --release -p sat-ui --example screenshot -- arrange.png 2         # clip panel
cargo run --release -p sat-ui --example screenshot -- mixer.png 2 mixer     # mixer panel
cargo run --release -p sat-ui --example screenshot -- euclid.png 2 euclid   # a clip by name
cargo run --release -p sat-ui --example screenshot -- grid.png 2 clip grid  # its source grid
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
