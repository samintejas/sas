//! The sat project document.
//!
//! A project is one immutable, versioned [`Document`]. Every edit is an
//! [`Op`]; [`History`] applies ops in all-or-nothing transactions, keeps
//! earlier versions for undo, and reports what changed so the app knows
//! whether to send a parameter event or recompile the engine schedule.
//!
//! ```
//! use sat_doc::{Document, History, Op};
//!
//! let mut h = History::new(Document::new(1), 100);
//! let (bass, op) = Op::add_track("bass", None, 0);
//! h.apply("add track", [op]).unwrap();
//! assert_eq!(h.doc().tracks[&bass].name, "bass");
//! h.undo();
//! assert!(!h.doc().tracks.contains_key(&bass));
//! ```

#![warn(missing_docs)]

mod doc;
pub mod file;
mod history;
mod id;
mod model;
mod op;

pub use doc::{DEFAULT_BPM, Document};
pub use history::{History, Transaction};
pub use id::*;
pub use model::*;
pub use op::{Change, ClipProps, NewGrids, Op, OpError, TrackFlag, apply};

/// Re-exported so users of this crate name time the same way.
pub use sat_time;
