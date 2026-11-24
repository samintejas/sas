//! The project file.
//!
//! A project is a bundle folder `name.sat/`:
//!
//! - `project.sat`: the document, serde JSON in a versioned envelope, zstd compressed
//! - `audio/`: recordings; `freeze/`: frozen renders; `peaks/`: waveform caches
//! - `backups/`: autosaves
//!
//! Loading runs migrations from the file's schema version up to [`SCHEMA`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::doc::Document;

/// The schema version this build writes.
pub const SCHEMA: u32 = 1;
const FORMAT: &str = "sat-project";
const ZSTD_LEVEL: i32 = 3;

/// Subfolders of a project bundle.
pub const BUNDLE_DIRS: [&str; 4] = ["audio", "freeze", "peaks", "backups"];

/// Why a project could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// Disk trouble.
    #[error("io: {0}")]
    Io(#[from] io::Error),
    /// The data is not valid JSON or does not match the schema.
    #[error("bad project data: {0}")]
    Json(#[from] serde_json::Error),
    /// Not a sat project.
    #[error("not a sat project")]
    NotAProject,
    /// Written by a newer sat.
    #[error("project schema {0} is newer than this build ({SCHEMA})")]
    TooNew(u32),
    /// A migration step failed.
    #[error("migration from schema {0} failed: {1}")]
    Migration(u32, String),
}

#[derive(Serialize)]
struct EnvelopeOut<'a> {
    format: &'static str,
    schema: u32,
    doc: &'a Document,
}

#[derive(Deserialize)]
struct EnvelopeIn {
    format: String,
    schema: u32,
    doc: Value,
}

/// Encodes a document as compressed `project.sat` bytes.
pub fn to_bytes(doc: &Document) -> Result<Vec<u8>, FileError> {
    let json = serde_json::to_vec(&EnvelopeOut { format: FORMAT, schema: SCHEMA, doc })?;
    Ok(zstd::encode_all(&json[..], ZSTD_LEVEL)?)
}

/// Decodes `project.sat` bytes, migrating older schemas.
pub fn from_bytes(bytes: &[u8]) -> Result<Document, FileError> {
    let json = zstd::decode_all(bytes)?;
    let env: EnvelopeIn = serde_json::from_slice(&json)?;
    if env.format != FORMAT {
        return Err(FileError::NotAProject);
    }
    if env.schema > SCHEMA {
        return Err(FileError::TooNew(env.schema));
    }
    let mut doc = env.doc;
    let mut v = env.schema;
    while v < SCHEMA {
        doc = migrate(v, doc).map_err(|e| FileError::Migration(v, e))?;
        v += 1;
    }
    Ok(serde_json::from_value(doc)?)
}

/// Upgrades a document from schema `from` to `from + 1`.
fn migrate(from: u32, _doc: Value) -> Result<Value, String> {
    // Schema 1 is the first; add a match arm here for each new schema.
    Err(format!("no migration from schema {from}"))
}

/// Writes `file` atomically: to a temporary name, then renamed over.
fn write_atomic(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = file.with_extension("sat.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, file)
}

/// Saves a project bundle at `dir` (created if needed).
pub fn save_bundle(dir: &Path, doc: &Document) -> Result<(), FileError> {
    fs::create_dir_all(dir)?;
    for sub in BUNDLE_DIRS {
        fs::create_dir_all(dir.join(sub))?;
    }
    write_atomic(&dir.join("project.sat"), &to_bytes(doc)?)?;
    Ok(())
}

/// Loads the project in bundle `dir`.
pub fn load_bundle(dir: &Path) -> Result<Document, FileError> {
    from_bytes(&fs::read(dir.join("project.sat"))?)
}

/// Writes an autosave into `dir/backups/`, named by `stamp` (such as a
/// timestamp), and keeps only the newest `keep` backups. Call it off the
/// audio and UI threads; a document clone is all it needs.
pub fn write_backup(dir: &Path, doc: &Document, stamp: &str, keep: usize) -> Result<PathBuf, FileError> {
    let b = dir.join("backups");
    fs::create_dir_all(&b)?;
    let path = b.join(format!("project-{stamp}.sat"));
    write_atomic(&path, &to_bytes(doc)?)?;
    let mut all: Vec<PathBuf> = fs::read_dir(&b)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sat"))
        .collect();
    all.sort();
    let excess = all.len().saturating_sub(keep.max(1));
    for old in &all[..excess] {
        fs::remove_file(old)?;
    }
    Ok(path)
}
