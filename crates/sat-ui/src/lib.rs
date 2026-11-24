//! sat's native UI.
//!
//! Views draw the document into a vello scene; text is laid out with parley
//! in the bundled Geist fonts; colours and sizes come from the Dots tokens in
//! `sat-theme`. The same scene renders to a window or, for tests and
//! screenshots, to an offscreen image.

#![warn(missing_docs)]

pub mod arrange;
pub mod demo;
pub mod gfx;
pub mod icons;
pub mod paint;
pub mod piano_roll;
pub mod shell;
