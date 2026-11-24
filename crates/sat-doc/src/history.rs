//! Transactions and undo.
//!
//! Because the document is persistent, undo just keeps earlier versions:
//! stepping back is swapping in the previous value. There is no history panel,
//! only undo and redo.

use crate::doc::Document;
use crate::op::{Change, Op, OpError, apply};

/// The result of one applied transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    /// Monotonic version number of the document after this transaction.
    pub version: u64,
    /// What changed, in op order.
    pub changes: Vec<Change>,
}

impl Transaction {
    /// Whether the engine schedule must be recompiled.
    pub fn is_structural(&self) -> bool {
        self.changes.iter().any(|c| matches!(c, Change::Structure))
    }
}

#[derive(Debug, Clone)]
struct Entry {
    doc: Document,
    label: String,
}

/// The current document plus undo and redo stacks.
#[derive(Debug, Clone)]
pub struct History {
    current: Document,
    version: u64,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Open gesture: the document before it started, and its label.
    gesture: Option<Entry>,
    limit: usize,
}

impl History {
    /// Starts a history at `doc`, keeping at most `limit` undo steps.
    pub fn new(doc: Document, limit: usize) -> History {
        History { current: doc, version: 0, undo: Vec::new(), redo: Vec::new(), gesture: None, limit: limit.max(1) }
    }

    /// The current document. Clone it freely; clones are cheap.
    pub fn doc(&self) -> &Document {
        &self.current
    }

    /// Version number, bumped by every applied transaction, undo and redo.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Applies `ops` as one all-or-nothing transaction and one undo step
    /// (or as part of the open gesture). On error nothing changes.
    pub fn apply(
        &mut self,
        label: impl Into<String>,
        ops: impl IntoIterator<Item = Op>,
    ) -> Result<Transaction, OpError> {
        let mut next = self.current.clone();
        let mut changes = Vec::new();
        for op in ops {
            changes.push(apply(&mut next, op)?);
        }
        if changes.is_empty() {
            return Ok(Transaction { version: self.version, changes });
        }
        let prev = std::mem::replace(&mut self.current, next);
        if self.gesture.is_none() {
            self.push_undo(Entry { doc: prev, label: label.into() });
        }
        self.redo.clear();
        self.version += 1;
        Ok(Transaction { version: self.version, changes })
    }

    /// Opens a gesture (a drag, a knob turn): every transaction until
    /// [`History::end_gesture`] becomes one undo step.
    pub fn begin_gesture(&mut self, label: impl Into<String>) {
        if self.gesture.is_none() {
            self.gesture = Some(Entry { doc: self.current.clone(), label: label.into() });
        }
    }

    /// Closes the open gesture, recording one undo step if anything changed.
    pub fn end_gesture(&mut self) {
        if let Some(e) = self.gesture.take()
            && e.doc != self.current
        {
            self.push_undo(e);
        }
    }

    fn push_undo(&mut self, e: Entry) {
        self.undo.push(e);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Label of the step [`History::undo`] would revert.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    /// Label of the step [`History::redo`] would reapply.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    /// Steps back one transaction. Returns its label.
    pub fn undo(&mut self) -> Option<String> {
        self.end_gesture();
        let e = self.undo.pop()?;
        let cur = std::mem::replace(&mut self.current, e.doc);
        self.redo.push(Entry { doc: cur, label: e.label.clone() });
        self.version += 1;
        Some(e.label)
    }

    /// Reapplies the last undone transaction. Returns its label.
    pub fn redo(&mut self) -> Option<String> {
        self.end_gesture();
        let e = self.redo.pop()?;
        let cur = std::mem::replace(&mut self.current, e.doc);
        self.undo.push(Entry { doc: cur, label: e.label.clone() });
        self.version += 1;
        Some(e.label)
    }
}
