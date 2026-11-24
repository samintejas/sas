//! Where the open project lives on disk: open, save, save as, and autosave.
//!
//! Autosaves never block the UI or audio: the document (a cheap persistent
//! clone) goes to a background thread that writes it into the bundle's
//! `backups/` folder. Projects that were never saved autosave into the
//! user's data folder instead.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sat_doc::{Document, file};

/// How often an edited project is autosaved.
pub const AUTOSAVE_EVERY: Duration = Duration::from_secs(120);
/// Autosaves kept per project.
pub const BACKUPS_KEPT: usize = 10;
/// The bundle extension.
pub const EXT: &str = "sat";

/// Adds the bundle extension if `p` lacks it.
pub fn bundle_path(p: &Path) -> PathBuf {
    if p.extension().is_some_and(|e| e == EXT) { p.to_path_buf() } else { p.with_extension(EXT) }
}

/// A display name for a bundle path.
pub fn name_of(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into())
}

/// A sortable timestamp for backup names.
fn stamp() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{:012}-{:03}", t.as_secs(), t.subsec_millis())
}

struct Job {
    dir: PathBuf,
    doc: Document,
}

/// Writes autosaves on a background thread.
pub struct Autosaver {
    tx: Option<mpsc::Sender<Job>>,
    worker: Option<JoinHandle<()>>,
}

impl Autosaver {
    /// Starts the writer thread.
    pub fn start() -> Autosaver {
        let (tx, rx) = mpsc::channel::<Job>();
        let worker = std::thread::Builder::new()
            .name("sat-autosave".into())
            .spawn(move || {
                for job in rx {
                    match file::write_backup(&job.dir, &job.doc, &stamp(), BACKUPS_KEPT) {
                        Ok(p) => eprintln!("autosaved {}", p.display()),
                        Err(e) => eprintln!("autosave to {} failed: {e}", job.dir.display()),
                    }
                }
            })
            .expect("autosave thread");
        Autosaver { tx: Some(tx), worker: Some(worker) }
    }

    fn send(&self, dir: PathBuf, doc: Document) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job { dir, doc });
        }
    }

    /// Waits for queued autosaves to finish and stops the thread.
    pub fn finish(&mut self) {
        self.tx.take();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl Drop for Autosaver {
    fn drop(&mut self) {
        self.finish();
    }
}

/// The open project's place on disk and its save state.
pub struct Project {
    /// The bundle, once saved or opened.
    pub path: Option<PathBuf>,
    /// Display name.
    pub name: String,
    /// The history version last written to `path`.
    saved: u64,
    /// The history version last autosaved, and when.
    autosaved: (u64, Instant),
    /// Where never-saved projects autosave.
    scratch: PathBuf,
    autosaver: Autosaver,
}

impl Project {
    /// A project at `path` (or unsaved, with `name`), whose document is at `version`.
    pub fn new(path: Option<PathBuf>, name: String, version: u64) -> Project {
        let scratch = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("sat").join("autosave");
        Project {
            path,
            name,
            saved: version,
            autosaved: (version, Instant::now()),
            scratch,
            autosaver: Autosaver::start(),
        }
    }

    /// Unsaved changes?
    pub fn edited(&self, version: u64) -> bool {
        version != self.saved
    }

    /// Opens the bundle at `path`.
    pub fn open(&mut self, path: &Path, version: u64) -> Result<Document, file::FileError> {
        let doc = file::load_bundle(path)?;
        self.path = Some(path.to_path_buf());
        self.name = name_of(path);
        self.saved = version;
        self.autosaved = (version, Instant::now());
        Ok(doc)
    }

    /// Saves to the current bundle. Errors if the project has none yet.
    pub fn save(&mut self, doc: &Document, version: u64) -> Result<PathBuf, file::FileError> {
        let path = self.path.clone().ok_or_else(|| {
            file::FileError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "project has no file yet"))
        })?;
        file::save_bundle(&path, doc)?;
        self.saved = version;
        Ok(path)
    }

    /// Saves to a new bundle at `path` (the extension is added if missing).
    pub fn save_as(&mut self, path: &Path, doc: &Document, version: u64) -> Result<PathBuf, file::FileError> {
        let path = bundle_path(path);
        file::save_bundle(&path, doc)?;
        self.name = name_of(&path);
        self.path = Some(path.clone());
        self.saved = version;
        Ok(path)
    }

    /// Where autosaves for this project go.
    pub fn autosave_dir(&self) -> PathBuf {
        match &self.path {
            Some(p) => p.clone(),
            None => self.scratch.join(format!("{}.{EXT}", self.name)),
        }
    }

    /// Queues an autosave if the document changed since the last one and
    /// either `force` is set or the interval has passed. Returns whether it queued.
    pub fn autosave(&mut self, doc: &Document, version: u64, force: bool) -> bool {
        let changed = version != self.autosaved.0 && version != self.saved;
        if !changed || !(force || self.autosaved.1.elapsed() >= AUTOSAVE_EVERY) {
            return false;
        }
        self.autosaver.send(self.autosave_dir(), doc.clone());
        self.autosaved = (version, Instant::now());
        true
    }

    /// When the next autosave check is due, if one could be.
    pub fn next_autosave(&self, version: u64) -> Option<Instant> {
        (version != self.autosaved.0 && version != self.saved).then(|| self.autosaved.1 + AUTOSAVE_EVERY)
    }

    /// Flushes pending autosaves (call on quit).
    pub fn finish(&mut self) {
        self.autosaver.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sat_doc::{History, Op, ParamTarget};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sat-app-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn save_as_adds_the_extension_and_round_trips() {
        let dir = tmp("saveas");
        let mut h = History::new(Document::new(7), 10);
        let master = h.doc().master;
        h.apply("fader", [Op::SetValue { target: ParamTarget::Gain(master), value: -4.5 }]).unwrap();
        let mut p = Project::new(None, "untitled".into(), 0);
        assert!(p.edited(h.version()));
        assert!(p.save(h.doc(), h.version()).is_err(), "nowhere to save yet");
        let path = p.save_as(&dir.join("song"), h.doc(), h.version()).unwrap();
        assert_eq!(path.extension().unwrap(), "sat");
        assert_eq!(p.name, "song");
        assert!(!p.edited(h.version()));
        let mut q = Project::new(None, "x".into(), 0);
        let back = q.open(&path, 0).unwrap();
        assert_eq!(back.tracks[&master].mixer.gain_db, -4.5);
        assert_eq!(q.name, "song");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_writes_backups_only_when_changed() {
        let dir = tmp("autosave");
        let mut h = History::new(Document::new(7), 10);
        let mut p = Project::new(None, "a".into(), h.version());
        let bundle = p.save_as(&dir.join("a"), h.doc(), h.version()).unwrap();
        // nothing changed since the save: no autosave even when forced
        assert!(!p.autosave(h.doc(), h.version(), true));
        let master = h.doc().master;
        h.apply("fader", [Op::SetValue { target: ParamTarget::Gain(master), value: -2.0 }]).unwrap();
        // changed, but the interval has not passed
        assert!(!p.autosave(h.doc(), h.version(), false));
        assert!(p.next_autosave(h.version()).is_some());
        assert!(p.autosave(h.doc(), h.version(), true));
        // the same version is not written twice
        assert!(!p.autosave(h.doc(), h.version(), true));
        p.finish();
        let backups: Vec<_> = std::fs::read_dir(bundle.join("backups")).unwrap().collect();
        assert_eq!(backups.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
