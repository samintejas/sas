//! sat's native UI.
//!
//! Views draw the document into a vello scene; text is laid out with parley
//! in the bundled Geist fonts; colours and sizes come from the Dots tokens in
//! `sat-theme`. The same scene renders to a window or, for tests and
//! screenshots, to an offscreen image.

#![warn(missing_docs)]

pub mod arrange;
pub mod controls;
pub mod demo;
pub mod gfx;
pub mod grid;
pub mod icons;
pub mod macros;
pub mod mixer;
pub mod paint;
pub mod panel;
pub mod piano_roll;
pub mod roll_edit;
pub mod session;
pub mod shell;
pub mod steps;
