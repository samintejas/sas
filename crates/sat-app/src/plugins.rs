//! The plugin hosts of the open project: one host process per plugin
//! module, started when the module gets a plugin and kept across
//! recompiles, so editing the set never restarts a plugin.

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
}

/// The binary a plugin runs in: this one, as `sat --plughost`.
fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sat"))
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
    /// Starts hosts for plugin modules that need one and stops those whose
    /// module is gone or hosts another plugin now.
    pub fn sync(&mut self, doc: &Document, rate: f64, max_block: usize) {
        self.slots.retain(|id, s| doc.modules.get(id).and_then(|m| m.plugin.as_deref()) == Some(s.plugin.as_str()));
        for m in doc.modules.values().filter(|m| m.kind == sat_modules::fx::PLUGIN) {
            if let Some(path) = &m.plugin
                && !self.slots.contains_key(&m.id)
            {
                self.start(m.id, path, rate, max_block);
            }
        }
    }

    /// Starts (or restarts) `module`'s host for `plugin`.
    pub fn start(&mut self, module: ModuleId, plugin: &str, rate: f64, max_block: usize) {
        self.slots.remove(&module);
        let (host, error) = match PlugHost::spawn(&exe(), &["--plughost"], &library(plugin), rate, max_block) {
            Ok(h) => {
                eprintln!("plugin: {} ({} parameters)", h.hello.name, h.hello.params.len());
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
        Some(Box::new(h.processor()))
    }
}
