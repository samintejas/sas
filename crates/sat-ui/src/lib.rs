//! sat's native UI.
//!
//! Views draw the document into a vello scene; text is laid out with parley
//! in the bundled Geist fonts; colours and sizes come from the Dots tokens in
//! `sat-theme`. The same scene renders to a window or, for tests and
//! screenshots, to an offscreen image.

#![warn(missing_docs)]

pub mod area;
pub mod arrange;
pub mod arrange_edit;
pub mod audio_edit;
pub mod automation;
pub mod bank;
pub mod browser;
pub mod chords;
pub mod controls;
pub mod demo;
pub mod dock;
pub mod gfx;
pub mod grid;
pub mod icons;
pub mod macros;
pub mod mixer;
pub mod outliner;
pub mod paint;
pub mod palette;
pub mod panel;
pub mod piano_roll;
pub mod record;
pub mod roll_edit;
pub mod session;
pub mod settings;
pub mod shell;
pub mod steps;
