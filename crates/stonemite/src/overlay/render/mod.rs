//! Overlay rendering backends.

mod gdi;

pub(super) use gdi::{draw_label, measure_label_width};
