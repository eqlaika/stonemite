use windows::Win32::Foundation::{POINT, RECT};

use super::layout::ResizeEdge;
use crate::eq_characters;

pub(super) struct MoveDragState {
    pub(super) pip_index: usize,
    pub(super) start_cursor: POINT,
    pub(super) start_rect: RECT,
}

pub(super) struct PipResizeDragState {
    pub(super) pip_index: usize,
    pub(super) edge: ResizeEdge,
    pub(super) start_cursor: POINT,
    pub(super) start_rect: RECT,
}

pub(super) struct StripResizeDragState {
    pub(super) start_pt: POINT,
    pub(super) start_size: i32,
}

pub(super) struct ReorderDragState {
    pub(super) from_index: usize,
    pub(super) start_pt: POINT,
    pub(super) dragging: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ContextMenuRequest {
    pub(super) target_pid: u32,
    pub(super) screen_point: POINT,
    pub(super) source_hwnd: windows::Win32::Foundation::HWND,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct ReorderCancellation {
    pub(super) dimmed_source: Option<usize>,
    pub(super) old_target: Option<usize>,
}

pub(super) fn take_reorder_cancellation(
    reorder_drag: &mut Option<ReorderDragState>,
    drop_target: &mut Option<usize>,
) -> ReorderCancellation {
    let dimmed_source = reorder_drag
        .take()
        .filter(|drag| drag.dragging)
        .map(|drag| drag.from_index);
    ReorderCancellation {
        dimmed_source,
        old_target: drop_target.take(),
    }
}

/// Transient user interaction state. Keeping it together prevents rendering,
/// client identity, and menu code from each growing independent gesture flags.
pub(super) struct InteractionState {
    pub(super) pending_context_menu: Option<ContextMenuRequest>,
    pub(super) context_menu_target_pid: Option<u32>,
    pub(super) context_menu_candidates: Vec<eq_characters::CharCandidate>,
    pub(super) context_menu_open: bool,
    pub(super) edit_mode: bool,
    pub(super) move_drag: Option<MoveDragState>,
    pub(super) pip_resize_drag: Option<PipResizeDragState>,
    pub(super) strip_resize_drag: Option<StripResizeDragState>,
    pub(super) reorder_drag: Option<ReorderDragState>,
    pub(super) drop_target: Option<usize>,
}

impl InteractionState {
    pub(super) fn new() -> Self {
        Self {
            pending_context_menu: None,
            context_menu_target_pid: None,
            context_menu_candidates: Vec::new(),
            context_menu_open: false,
            edit_mode: false,
            move_drag: None,
            pip_resize_drag: None,
            strip_resize_drag: None,
            reorder_drag: None,
            drop_target: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_cancellation_takes_source_and_target_atomically() {
        let mut drag = Some(ReorderDragState {
            from_index: 2,
            start_pt: POINT::default(),
            dragging: true,
        });
        let mut target = Some(4);
        assert_eq!(
            take_reorder_cancellation(&mut drag, &mut target),
            ReorderCancellation {
                dimmed_source: Some(2),
                old_target: Some(4),
            }
        );
        assert!(drag.is_none());
        assert!(target.is_none());

        let mut pending_click = Some(ReorderDragState {
            from_index: 1,
            start_pt: POINT::default(),
            dragging: false,
        });
        assert_eq!(
            take_reorder_cancellation(&mut pending_click, &mut None),
            ReorderCancellation {
                dimmed_source: None,
                old_target: None,
            }
        );
    }
}
