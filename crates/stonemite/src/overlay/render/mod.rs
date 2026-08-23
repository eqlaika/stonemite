//! Hardware Direct2D/DirectWrite rendering presented through DirectComposition.

mod compositor;
mod scene_d2d;

pub(super) use compositor::Compositor;
