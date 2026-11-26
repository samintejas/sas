//! The plugin hosts of the open project: one host process per plugin
//! module, started when the module gets a plugin and kept across
//! recompiles, so editing the set never restarts a plugin.
//!
//! Each plugin's state is saved with the project, in the bundle's
//! `plugins/<module id>.state`, and given back to it when the project opens
//! and when a crashed plugin is reloaded. Export and freeze render through
//! a second set of hosts started for the render, which take the live
//! plugins' current state and wait for every block.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sat_doc::{Document, ModuleId};
use sat_plug::{PlugHost, Status};
use sat_ui::grid::{PluginState, PluginView, PluginViews};

/// One plugin module's host.
struct Slot {
    plugin: String,
    host: Option<PlugHost>,
    error: Option<String>,
}

/// The hosts, by plugin module.
#[derive(Default)]
pub struct Hosts {
    slots: HashMap<ModuleId, Slot>,
    /// The project bundle plugin states are saved in, once it has one.
    bundle: Option<PathBuf>,
    /// Hosts for an offline render: processors wait for every block.
    offline: bool,
}

/// The bundle folder plugin states are kept in.
pub const STATE_DIR: &str = "plugins";

/// Where `module`'s plugin state is kept in `bundle`.
pub fn state_path(bundle: &Path, module: ModuleId) -> PathBuf {
    bundle.join(STATE_DIR).join(format!("{module}.state"))
}

/// The binary a plugin runs in: this one, as `sat --plughost`.
fn exe() -> PathBuf {
    let me = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sat"));
    // unit tests run from target/<profile>/deps; the app is next to it
    if cfg!(test)
        && let Some(app) =
            me.parent().and_then(Path::parent).map(|d| d.join(format!("sat{}", std::env::consts::EXE_SUFFIX)))
    {
        return app;
    }
    me
}

/// What to load for a plugin path: on macOS a `.clap` is a bundle folder
/// with the library inside.
fn library(path: &str) -> String {
    let p = Path::new(path);
    if p.is_dir()
        && let Some(stem) = p.file_stem()
    {
        let inner = p.join("Contents").join("MacOS").join(stem);
        if inner.exists() {
            return inner.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

impl Hosts {
    /// Hosts for an offline render of `doc` (blocks of
    /// [`sat_compile::RENDER_BLOCK`]), with states from `bundle`.
    pub fn for_render(doc: &Document, rate: f64, bundle: Option<&Path>) -> Hosts {
        let mut h = Hosts { bundle: bundle.map(Path::to_path_buf), offline: true, ..Hosts::default() };
        h.sync(doc, rate, sat_compile::RENDER_BLOCK);
        h
    }

    /// Hosts for an offline render of `doc` that start where these live
    /// plugins are now (their current state, not the last saved one).
    pub fn render_copy(&mut self, doc: &Document, rate: f64) -> Hosts {
        let mut r = Hosts::for_render(doc, rate, self.bundle.as_deref());
        let tmp = std::env::temp_dir().join(format!("sat-render-{}.state", std::process::id()));
        for (id, slot) in &mut self.slots {
            let Some(live) = slot.host.as_mut() else { continue };
            if live.status() != Status::Running {
                continue;
            }
            let Some(copy) = r.slots.get_mut(id).and_then(|s| s.host.as_mut()) else { continue };
            // a plugin without state support renders from its knobs alone
            if live.save_state(&tmp).is_ok()
                && let Err(e) = copy.load_state(&tmp)
            {
                eprintln!("plugin {}: {e}", live.hello.name);
            }
        }
        let _ = std::fs::remove_file(&tmp);
        r
    }

    /// Sets the bundle plugin states are saved in and restored from.
    pub fn set_bundle(&mut self, bundle: Option<&Path>) {
        self.bundle = bundle.map(Path::to_path_buf);
    }

    /// Saves every running plugin's state into `bundle` (and makes it the
    /// bundle from now on), and removes states of plugins no longer there.
    /// Returns what could not be saved.
    pub fn save_states(&mut self, bundle: &Path) -> Vec<String> {
        self.bundle = Some(bundle.to_path_buf());
        let dir = bundle.join(STATE_DIR);
        let mut errors = Vec::new();
        if !self.slots.is_empty()
            && let Err(e) = std::fs::create_dir_all(&dir)
        {
            return vec![format!("cannot create {}: {e}", dir.display())];
        }
        let mut kept = std::collections::HashSet::new();
        for (id, slot) in &mut self.slots {
            let path = state_path(bundle, *id);
            let running = slot.host.as_mut().map(|h| h.status() == Status::Running);
            match (slot.host.as_mut(), running) {
                (Some(h), Some(true)) => match h.save_state(&path) {
                    Ok(()) => {
                        kept.insert(path);
                    }
                    // no state support: nothing to keep
                    Err(e) => errors.push(format!("{}: {e}", h.hello.name)),
                },
                // a crashed plugin keeps the state it was last saved with
                _ => {
                    kept.insert(path);
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if !kept.contains(&e.path()) && e.path().extension().is_some_and(|x| x == "state") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        errors
    }

    /// Starts hosts for plugin modules that need one and stops those whose
    /// module is gone or hosts another plugin now.
    pub fn sync(&mut self, doc: &Document, rate: f64, max_block: usize) {
        self.slots.retain(|id, s| doc.modules.get(id).and_then(|m| m.plugin.as_deref()) == Some(s.plugin.as_str()));
        for m in doc.modules.values().filter(|m| m.kind == sat_modules::fx::PLUGIN) {
            if let Some(path) = &m.plugin
                && !self.slots.contains_key(&m.id)
            {
                self.start(m.id, path, rate, max_block, true);
            }
        }
    }

    /// Starts (or restarts) `module`'s host for `plugin`, with the state
    /// saved in the project if `restore` (a newly picked plugin starts
    /// fresh).
    pub fn start(&mut self, module: ModuleId, plugin: &str, rate: f64, max_block: usize, restore: bool) {
        self.slots.remove(&module);
        let (host, error) = match PlugHost::spawn(&exe(), &["--plughost"], &library(plugin), rate, max_block) {
            Ok(mut h) => {
                if !self.offline {
                    eprintln!("plugin: {} ({} parameters)", h.hello.name, h.hello.params.len());
                }
                let saved = self.bundle.as_deref().map(|b| state_path(b, module)).filter(|p| restore && p.exists());
                if let Some(path) = saved
                    && let Err(e) = h.load_state(&path)
                {
                    eprintln!("plugin {}: {e}", h.hello.name);
                }
                (Some(h), None)
            }
            Err(e) => {
                eprintln!("plugin {plugin}: {e}");
                (None, Some(e))
            }
        };
        self.slots.insert(module, Slot { plugin: plugin.into(), host, error });
    }

    /// The plugin's default parameter values, 0–1 across each range, for
    /// the module's knobs.
    pub fn defaults(&self, module: ModuleId) -> Vec<f32> {
        let Some(h) = self.slots.get(&module).and_then(|s| s.host.as_ref()) else { return Vec::new() };
        h.hello
            .params
            .iter()
            .map(|p| if p.max > p.min { ((p.default - p.min) / (p.max - p.min)) as f32 } else { 0.0 })
            .collect()
    }

    /// Why `module`'s plugin did not load, if it did not.
    pub fn error(&self, module: ModuleId) -> Option<&str> {
        self.slots.get(&module).and_then(|s| s.error.as_deref())
    }

    /// Is `module`'s host down (crashed, hung or never started)?
    pub fn down(&mut self, module: ModuleId) -> bool {
        match self.slots.get_mut(&module).and_then(|s| s.host.as_mut()) {
            Some(h) => h.status() != Status::Running,
            None => true,
        }
    }

    /// What each plugin module shows.
    pub fn views(&mut self) -> PluginViews {
        self.slots
            .iter_mut()
            .map(|(id, s)| {
                let view = match &mut s.host {
                    Some(h) => PluginView {
                        name: h.hello.name.clone(),
                        params: h.hello.params.iter().map(|p| p.name.clone()).collect(),
                        state: match h.status() {
                            Status::Running => PluginState::Running,
                            Status::Hung => PluginState::Hung,
                            Status::Crashed => PluginState::Crashed,
                        },
                    },
                    None => PluginView {
                        name: String::new(),
                        params: Vec::new(),
                        state: PluginState::Failed(s.error.clone().unwrap_or_default()),
                    },
                };
                (*id, view)
            })
            .collect()
    }
}

impl sat_compile::Plugins for Hosts {
    fn processor(&self, module: ModuleId) -> Option<Box<dyn sat_engine::External>> {
        let h = self.slots.get(&module)?.host.as_ref()?;
        Some(Box::new(if self.offline { h.offline_processor() } else { h.processor() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sat_doc::{Op, apply};

    /// A document with the crash-test plugin in a fx grid.
    fn doc_with_plugin() -> (Document, ModuleId) {
        let mut doc = sat_ui::demo::demo().doc;
        let keys = doc.tracks.values().find(|t| t.name == "keys").unwrap();
        let grid = *keys.fx_grids.front().unwrap();
        let (m, mut ops) = sat_modules::fx::insert(&doc, grid, sat_modules::fx::PLUGIN);
        ops.push(Op::SetModulePlugin { module: m, plugin: Some("builtin:crash-test".into()) });
        for op in ops {
            apply(&mut doc, op).unwrap();
        }
        (doc, m)
    }

    #[test]
    fn states_are_saved_in_the_bundle_and_restored() {
        if !exe().exists() {
            eprintln!("skipped: {} is not built", exe().display());
            return;
        }
        let bundle = std::env::temp_dir().join(format!("sat-app-states-{}.sat", std::process::id()));
        let (doc, m) = doc_with_plugin();
        let mut live = Hosts::default();
        live.sync(&doc, 48_000.0, 256);
        assert!(!live.down(m));
        assert!(live.save_states(&bundle).is_empty());
        let saved = state_path(&bundle, m);
        assert!(saved.exists());
        // a stale state of a module that is gone is cleared on the next save
        let stale = bundle.join(STATE_DIR).join("gone.state");
        std::fs::write(&stale, [0u8; 24]).unwrap();
        assert!(live.save_states(&bundle).is_empty());
        assert!(!stale.exists() && saved.exists());
        // a project opened from the bundle restores it (a broken state file
        // is reported, and the plugin still runs)
        std::fs::write(&saved, b"broken").unwrap();
        let mut opened = Hosts::default();
        opened.set_bundle(Some(&bundle));
        opened.sync(&doc, 48_000.0, 256);
        assert!(!opened.down(m));
        // renders get hosts of their own, offline
        let mut render = opened.render_copy(&doc, 48_000.0);
        assert!(render.offline && !render.down(m));
        std::fs::remove_dir_all(&bundle).ok();
    }
}
