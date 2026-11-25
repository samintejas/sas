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
project file end to end. Phase 3 (source grid) is done: notes are edited
in the piano roll, the drum sequencer or the euclid generator (with hand edits
over it) and heard while the set plays, and each track's source grid shows its
modules and the clip dock, where cables decide which module feeds each clip.
Phase 4 (instrument + fx grids) is done: each track's instrument grid is a
patchable voice (note in, vco, noise, mixer, filter, adsr, vca, lfo, voice out)
that every voice runs, a drum synth that plays drums by pitch, or a sampler
that plays a WAV repitched from its root note (gate, one shot or loop, with a
start point, a crossfaded loop point and an envelope); every
track's fx grid (eq, comp with side chain, drive, filter, delay, reverb,
limiter, split and mix) runs between its sound and its strip, buses and the
master included. Every grid has 16 macros: each drives one or more of its
grid's parameters over a range, and the bottom panel's macros page shows the
selected track's macros with its fader and pan. Knobs, macros and bypass
switches change the sound live while it plays. The demo song is made with
built-in modules only (its vocal is a sampler playing a generated "ah"),
which is the phase 4 gate. Sounds live in the bundle's `audio/` folder; an
imported sound is copied there when the project is saved. Phase 5 (session +
recording) has started with the session view: a clip launcher where scenes
are rows and tracks and groups are columns. Clips, scenes and groups launch
on the next bar (inside the loop when it is on), and a track that has
launched a clip follows the session until "back to arrangement". Audio
clips hold takes of the same passage and a comp that picks which take plays
where: each comp boundary is an equal-power crossfade (20 ms by default) and
the clip has its own fades. In the arrange view an audio clip shows its
comped waveform with its crossfades and fade handles, and its takes in lanes
underneath; swiping a lane comps that take in. The demo's vocal is three
generated takes, each flubbing a different note, comped into one. Recording
captures the default input onto the armed tracks: each pass becomes a take,
and with the loop on every time round the loop is another take of the same
clip, comped in where it was sung. Selecting an audio clip shows its comp
in the bottom panel with its gain, fades and crossfade time. Setting
`SAT_INPUT_WAV=some.wav` records from that file (played in real time)
instead of a device, for headless runs. A track hears its input through its
fx and strip when its monitor switch (in the header of an armed track) is
on "in", or on "auto" while it is armed and the transport is stopped or
recording. Recording adds a recorder module to the track's source grid
(its `input` picks both input channels or one) and wires new clips to it;
takes are shifted by the input and output latency the device reports, so
they line up with what was heard. Phase 5 is done. Phase 6 (generative +
export) has started with export: Ctrl+E (or `sat --export`) renders the song
offline to a WAV file, running on past the last clip until reverb and delay
tails fall silent, and Ctrl+Shift+E renders the loop range. The render runs
the engine exactly as playback does, so it matches what was heard sample for
sample whatever block size the audio device asked for (automation is
evaluated per sample, not per block). The turing generator is a looping
shift register of seeded random bits: `change` is the chance that the bit
coming round flips (0 locks the phrase, 1 inverts it each time round), and
the register picks pitches from a scale over a range and whether each step
plays (`dens`). Its seed comes from the project seed and the clip, so it
plays and exports the same every time; with `reseed` on, every time round a
looping clip plays a fresh phrase from (clip seed, loop count). Its step
editor shows each step's pitch, and hand edits, clear and bake work as on
the euclid. The demo has a reseeding turing track over its second half. The arrange view, piano roll and mixer from
the prototype are drawn with vello; the engine plays through the default
audio device and feeds live per-track meters.

| Crate | What it does | State |
|---|---|---|
| `sat-time` | ticks (960 PPQ), tempo map with step and linear ramps, meter map, quantisation | done |
| `sat-doc` | entities, `Op` reducer, transactions, undo with gestures, project bundle with migrations | done |
| `sat-dsp` | oscillators (polyBLEP), state-variable filter, ADSR, noise, drum voices, gain and pan laws, biquads, eq, compressor, delay, reverb, saturation, limiter | in progress |
| `sat-engine` | flat schedule, mixer strips, test synth, patchable voice instrument, drum synth, sampler, audio clip playback with fades and crossfades, session clip launching, fx chains, automation, loop, lock-free commands, schedule swap with state carry-over, offline render | done (single-threaded) |
| `sat-compile` | document → schedule: track tree, solo, sends, placements and looping clips, audio clip comps, instrument patches, samplers and fx chains from their grids, live parameter mapping, offline render with tail | done |
| `sat-disk` | WAV import and export (hound), the open project's pool of decoded sounds, writing sounds into the bundle, waveform peaks | in progress |
| `sat-modules` | built-in modules and their parameter specs: source modules (which editor each opens, the euclid generator, the drum kit, the hand-edit layer), instrument and fx modules, the default voice patch, fx chains and insertion, patching rules | in progress |
| `sat-theme` | Dots design tokens (`assets/dots/tokens.json`) generated into a Rust `Palette`, plus spacing, radius and type scales | done |
| `sat-ui` | vello + parley painter, top bar, arrange view, source grid, piano roll, step editors, mixer, shared controls (faders, knobs, meters), hit-testing, headless GPU rendering | in progress |
| `sat-app` | the `sat` binary: winit window, vello surface, cpal output running the engine, mixing gestures with undo, open / save / autosave, WAV export (also headless) | in progress |

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
cargo run --release -p sat-app -- --export song.wav my.sat          # renders it without a window
cargo run --release -p sat-app -- --export loop.wav --loop --rate 44100  # the demo's loop range
```

Exports are 32-bit float WAV at 48 kHz unless `--rate` says otherwise (from
the app: the device's rate).

| Keys and mouse | Does |
|---|---|
| Space, Home, `l` | play / stop, back to the start, loop on / off |
| top bar: stop, play, record, loop | stop (again: back to the start), play / pause, record onto the armed tracks (arming the selected track if none is; starts the transport; again: stop recording, keep playing), loop on / off |
| an armed track's in / auto / off | monitor its input always, while stopped or recording, or never |
| audio clip panel: drag gain, fade in, fade out or xfade up / down, double-click | change it (shift for fine steps), reset it |
| Tab, or the panel tabs | switch the bottom panel between the clip and the mixer |
| drag a fader or knob | set level, pan or a send (Shift for fine steps, double-click to reset); one drag is one undo step |
| `m` / `s` buttons, or the keys | mute / solo (the keys act on the selected track); the round button arms recording |
| Ctrl+Z, Ctrl+Shift+Z | undo, redo |
| Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+N, Ctrl+Q | open, save, save as, new, quit (Cmd on macOS) |
| Ctrl+E, Ctrl+Shift+E | export the song (with its tail) or the loop range to WAV |
| drop a `.sat` folder on the window | open it |
| drop a `.wav` on the window | load it into the shown instrument grid's (or the selected track's) sampler, adding a sampler if there is none |
| click a clip, the ruler, a group's chevron | edit it in the piano roll, move the playhead, fold the group |
| piano roll: click empty space (draw tool) | draw a note; keep dragging to set its length |
| piano roll: drag a note, its right edge, the velocity lane | move it (time and pitch), resize it, set velocity; the selection moves together |
| piano roll: double-click a note, Delete, arrows, Ctrl+A, Esc | delete, delete the selection, nudge by the grid or transpose (Shift: octave), select all, clear |
| piano roll: select tool, shift-click, the grid and quantise buttons | box-select, add to the selection, change the grid (Alt drags off it), snap starts to the grid |
| top bar view buttons, "open source grid", "‹ arrange" | switch between the session, the arrangement and the selected track's source, instrument or fx grid |
| session: a clip's triangle, a scene, a group's cell | launch it on the next bar (starting the transport if stopped); a scene launches its row and stops tracks with an empty slot |
| session: a clip's name, an empty slot or stop button, stop all, back to arrangement | select it, stop that track (or group), stop everything, play the arrangement again |
| source grid: drag a module's header, drag a parameter row, drag the canvas or wheel | move the module, change the value, pan |
| source grid: drag from a notes out to a clip's inlet, drag a wire off an inlet | wire that clip to the module (replacing its feed), rewire or unplug it |
| source grid: + module, the dock's +, Delete, Esc | add a piano roll, drum seq or euclid; add a one-bar clip after the track's last one; remove the selected module; deselect |
| instrument grid: drag from an out to an in of the same colour, drag a cable off an in | patch them (replacing what fed that input; loops are refused), repatch or unplug it |
| instrument grid: drag a parameter row, + module, Delete | change the sound while it plays, add a module, remove the selected module |
| an audio clip's take lane: click, or drag along it | play that take over the comp segment under the click, or over the dragged range (snapped to sixteenths) |
| an audio clip's square fade handles | drag the fade in or out |
| sampler: click its waveform | load a WAV (white line: where notes start; amber: where the loop starts) |
| macros page: drag a macro, learn, click an empty macro, open | play it; map the next touched parameter (in any grid or the euclid editor) to a free macro, click a macro to clear it; start learning; show that grid |
| a parameter row with a macro badge (m1–m16) | drag it to play its macro |
| fx grid: + module, the power icon, dragging between ports | add an effect just before out (an empty grid gets its in and out), bypass it, reroute (split, mix, side chain) |
| drum seq: click a step, Shift-click it, the page buttons | toggle it, cycle its velocity (normal, accent, ghost), show another bar |
| euclid: drag a knob, click a trig | change steps, hits, rotation, accents, note, rate, gate or velocity; add or remove a hand edit over the generator |
| turing: drag a knob, click a step | change steps, change, low, range, scale, dens, rate, gate, vel or reseed; add or remove that step's note as a hand edit |
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
cargo run --release -p sat-ui --example screenshot -- ins.png 2 clip instrument  # its instrument grid
cargo run --release -p sat-ui --example screenshot -- fx.png 2 clip fx       # its fx grid
cargo run --release -p sat-ui --example screenshot -- smp.png 2 ah instrument  # the vox sampler
cargo run --release -p sat-ui --example screenshot -- mac.png 2 macros instrument  # macros page
cargo run --release -p sat-ui --example screenshot -- ses.png 2 clip session  # session view
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
