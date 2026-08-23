use windows::Win32::Foundation::{POINT, RECT};

use crate::config;

const SNAP_DISTANCE: i32 = 12;
pub(super) const MAX_STRIP_WIDTH_FRACTION: f64 = 0.25;
pub(super) const MIN_STRIP_WIDTH_FRACTION: f64 = 0.05;

/// Runtime layout configuration and the latest computed strip geometry.
pub(super) struct LayoutState {
    pub(super) monitor_rect: RECT,
    pub(super) dpi_scale: f64,
    pub(super) pip_edge: config::PipEdge,
    pub(super) custom_strip_width: Option<i32>,
    pub(super) snap_grid: i32,
    pub(super) has_custom_positions: bool,
    pub(super) strip_width: i32,
    pub(super) strip_height: i32,
}

impl LayoutState {
    pub(super) fn new(cfg: &config::Config, dpi_scale: f64) -> Self {
        Self {
            monitor_rect: RECT::default(),
            dpi_scale,
            pip_edge: cfg.pip_edge,
            custom_strip_width: cfg.pip_strip_width.map(|width| width as i32),
            snap_grid: cfg.snap_grid as i32,
            has_custom_positions: !cfg.pip_positions.is_empty(),
            strip_width: 0,
            strip_height: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ResizeEdge {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

/// Detect a resize edge or corner from a client-area point.
pub(super) fn resize_edge_hit_test(
    point: POINT,
    width: i32,
    height: i32,
    zone: i32,
) -> Option<ResizeEdge> {
    let on_left = point.x < zone;
    let on_right = point.x >= width - zone;
    let on_top = point.y < zone;
    let on_bottom = point.y >= height - zone;
    match (on_left, on_right, on_top, on_bottom) {
        (true, _, true, _) => Some(ResizeEdge::NW),
        (true, _, _, true) => Some(ResizeEdge::SW),
        (_, true, true, _) => Some(ResizeEdge::NE),
        (_, true, _, true) => Some(ResizeEdge::SE),
        (true, _, _, _) => Some(ResizeEdge::W),
        (_, true, _, _) => Some(ResizeEdge::E),
        (_, _, true, _) => Some(ResizeEdge::N),
        (_, _, _, true) => Some(ResizeEdge::S),
        _ => None,
    }
}

/// Check whether a client-coordinate point is in the strip resize zone.
pub(super) fn strip_resize_hit_test(
    point: POINT,
    width: i32,
    height: i32,
    pip_edge: config::PipEdge,
    handle_width: i32,
) -> bool {
    match pip_edge {
        config::PipEdge::Right => point.x < handle_width,
        config::PipEdge::Left => point.x >= width - handle_width,
        config::PipEdge::Top => point.y >= height - handle_width,
        config::PipEdge::Bottom => point.y < handle_width,
    }
}

pub(super) struct MoveSnapInput {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) monitor: RECT,
    pub(super) grid: i32,
    pub(super) bypass: bool,
}

/// Snap a position to the configured grid, monitor edges, and other PiPs.
pub(super) fn snap_point(input: MoveSnapInput, others: &[RECT]) -> (i32, i32) {
    if input.bypass {
        return (input.x, input.y);
    }

    let mut snapped_x = input.x;
    let mut snapped_y = input.y;

    if input.grid > 0 {
        snapped_x = ((snapped_x as f64 / input.grid as f64).round() as i32) * input.grid;
        snapped_y = ((snapped_y as f64 / input.grid as f64).round() as i32) * input.grid;
    }

    if (snapped_x - input.monitor.left).abs() < SNAP_DISTANCE {
        snapped_x = input.monitor.left;
    }
    if (snapped_x + input.width - input.monitor.right).abs() < SNAP_DISTANCE {
        snapped_x = input.monitor.right - input.width;
    }
    if (snapped_y - input.monitor.top).abs() < SNAP_DISTANCE {
        snapped_y = input.monitor.top;
    }
    if (snapped_y + input.height - input.monitor.bottom).abs() < SNAP_DISTANCE {
        snapped_y = input.monitor.bottom - input.height;
    }

    for other in others {
        if (snapped_x - other.left).abs() < SNAP_DISTANCE {
            snapped_x = other.left;
        }
        if (snapped_x - other.right).abs() < SNAP_DISTANCE {
            snapped_x = other.right;
        }
        if (snapped_x + input.width - other.left).abs() < SNAP_DISTANCE {
            snapped_x = other.left - input.width;
        }
        if (snapped_x + input.width - other.right).abs() < SNAP_DISTANCE {
            snapped_x = other.right - input.width;
        }
        if (snapped_y - other.top).abs() < SNAP_DISTANCE {
            snapped_y = other.top;
        }
        if (snapped_y - other.bottom).abs() < SNAP_DISTANCE {
            snapped_y = other.bottom;
        }
        if (snapped_y + input.height - other.top).abs() < SNAP_DISTANCE {
            snapped_y = other.top - input.height;
        }
        if (snapped_y + input.height - other.bottom).abs() < SNAP_DISTANCE {
            snapped_y = other.bottom - input.height;
        }
    }

    (snapped_x, snapped_y)
}

fn aspect_height_for_width(cell_width: i32, border: i32) -> i32 {
    let thumbnail_width = cell_width - 2 * border;
    let thumbnail_height = (thumbnail_width as f64 * 9.0 / 16.0).round() as i32;
    thumbnail_height + 2 * border
}

fn aspect_width_for_height(cell_height: i32, border: i32) -> i32 {
    let thumbnail_height = cell_height - 2 * border;
    let thumbnail_width = (thumbnail_height as f64 * 16.0 / 9.0).round() as i32;
    thumbnail_width + 2 * border
}

/// Apply a resize while preserving the 16:9 thumbnail aspect ratio.
pub(super) fn snap_resize(
    edge: ResizeEdge,
    start_rect: RECT,
    dx: i32,
    dy: i32,
    grid: i32,
    border: i32,
    bypass: bool,
) -> RECT {
    let min_width = 80;
    let mut result = start_rect;

    match edge {
        ResizeEdge::E | ResizeEdge::NE | ResizeEdge::SE => result.right += dx,
        ResizeEdge::W | ResizeEdge::NW | ResizeEdge::SW => result.left += dx,
        _ => {}
    }
    match edge {
        ResizeEdge::S | ResizeEdge::SE | ResizeEdge::SW => result.bottom += dy,
        ResizeEdge::N | ResizeEdge::NE | ResizeEdge::NW => result.top += dy,
        _ => {}
    }

    if !bypass && grid > 0 {
        match edge {
            ResizeEdge::E | ResizeEdge::NE | ResizeEdge::SE => {
                result.right = ((result.right as f64 / grid as f64).round() as i32) * grid
            }
            ResizeEdge::W | ResizeEdge::NW | ResizeEdge::SW => {
                result.left = ((result.left as f64 / grid as f64).round() as i32) * grid
            }
            _ => {}
        }
        match edge {
            ResizeEdge::N => result.top = ((result.top as f64 / grid as f64).round() as i32) * grid,
            ResizeEdge::S => {
                result.bottom = ((result.bottom as f64 / grid as f64).round() as i32) * grid
            }
            _ => {}
        }
    }

    let width = result.right - result.left;
    if width < min_width {
        match edge {
            ResizeEdge::W | ResizeEdge::NW | ResizeEdge::SW => {
                result.left = result.right - min_width
            }
            _ => result.right = result.left + min_width,
        }
    }

    match edge {
        ResizeEdge::N | ResizeEdge::S => {
            let height = result.bottom - result.top;
            result.right = result.left + aspect_width_for_height(height, border).max(min_width);
        }
        _ => {
            let width = result.right - result.left;
            let new_height = aspect_height_for_width(width, border);
            match edge {
                ResizeEdge::NW | ResizeEdge::NE => result.top = result.bottom - new_height,
                _ => result.bottom = result.top + new_height,
            }
        }
    }

    result
}
/// Immutable inputs for one layout computation.
pub(super) struct LayoutInput<'a> {
    pub(super) dpi_scale: f64,
    pub(super) monitor_rect: RECT,
    pub(super) pip_count: usize,
    pub(super) pip_edge: config::PipEdge,
    pub(super) custom_strip_width: Option<i32>,
    pub(super) custom_positions: &'a [config::PipPosition],
    pub(super) gap: i32,
    pub(super) border: i32,
}

pub(super) struct LayoutPlan {
    pub(super) rects: Vec<RECT>,
    pub(super) strip_width: i32,
    pub(super) strip_height: i32,
}

/// Compute strip and custom PiP placement without reading configuration or Win32 state.
pub(super) fn compute(input: LayoutInput<'_>) -> LayoutPlan {
    let monitor_width = input.monitor_rect.right - input.monitor_rect.left;
    let monitor_height = input.monitor_rect.bottom - input.monitor_rect.top;
    let count = input.pip_count as i32;
    if count == 0 {
        return LayoutPlan {
            rects: Vec::new(),
            strip_width: 0,
            strip_height: 0,
        };
    }

    let vertical = matches!(
        input.pip_edge,
        config::PipEdge::Right | config::PipEdge::Left
    );
    let (strip_x, strip_y, cell_width, cell_height);

    if vertical {
        let max_strip_width = (monitor_width as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_width = (monitor_width as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
        let auto_max_thumbnail_width = max_strip_width - 2 * input.border;
        let auto_max_thumbnail_height =
            (auto_max_thumbnail_width as f64 * 9.0 / 16.0).round() as i32;
        let auto_max_cell_height = (monitor_height - (count - 1).max(0) * input.gap) / count;
        let auto_thumbnail_height = (auto_max_cell_height - 2 * input.border)
            .clamp(scale(40, input.dpi_scale), auto_max_thumbnail_height);
        let auto_thumbnail_width = (auto_thumbnail_height as f64 * 16.0 / 9.0).round() as i32;
        let auto_strip_width = auto_thumbnail_width + 2 * input.border;
        let effective_width = input.custom_strip_width.map_or(auto_strip_width, |width| {
            width.clamp(min_strip_width, max_strip_width)
        });
        let thumbnail_width = effective_width - 2 * input.border;
        let thumbnail_height = (thumbnail_width as f64 * 9.0 / 16.0).round() as i32;
        cell_width = effective_width;
        cell_height = thumbnail_height + 2 * input.border;
        strip_x = match input.pip_edge {
            config::PipEdge::Left => input.monitor_rect.left,
            _ => input.monitor_rect.right - cell_width,
        };
        strip_y = input.monitor_rect.top;
    } else {
        let max_strip_height = (monitor_height as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_height = (monitor_height as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
        let auto_max_thumbnail_height = max_strip_height - 2 * input.border;
        let auto_max_thumbnail_width =
            (auto_max_thumbnail_height as f64 * 16.0 / 9.0).round() as i32;
        let auto_max_cell_width = (monitor_width - (count - 1).max(0) * input.gap) / count;
        let auto_thumbnail_width = (auto_max_cell_width - 2 * input.border)
            .clamp(scale(60, input.dpi_scale), auto_max_thumbnail_width);
        let auto_thumbnail_height = (auto_thumbnail_width as f64 * 9.0 / 16.0).round() as i32;
        let auto_cell_height = auto_thumbnail_height + 2 * input.border;
        let effective_height = input.custom_strip_width.map_or(auto_cell_height, |height| {
            height.clamp(min_strip_height, max_strip_height)
        });
        let thumbnail_height = effective_height - 2 * input.border;
        let thumbnail_width = (thumbnail_height as f64 * 16.0 / 9.0).round() as i32;
        cell_width = thumbnail_width + 2 * input.border;
        cell_height = effective_height;
        let total_width = count * cell_width + (count - 1).max(0) * input.gap;
        strip_x = input.monitor_rect.right - total_width;
        strip_y = match input.pip_edge {
            config::PipEdge::Top => input.monitor_rect.top,
            _ => input.monitor_rect.bottom - cell_height,
        };
    }

    let mut rects = (0..count)
        .map(|index| {
            let (x_offset, y_offset) = if vertical {
                (0, index * (cell_height + input.gap))
            } else {
                (index * (cell_width + input.gap), 0)
            };
            RECT {
                left: strip_x + x_offset,
                top: strip_y + y_offset,
                right: strip_x + x_offset + cell_width,
                bottom: strip_y + y_offset + cell_height,
            }
        })
        .collect::<Vec<_>>();

    let strip_width = rects.last().map_or(0, |last| last.right - rects[0].left);
    let strip_height = rects.last().map_or(0, |last| last.bottom - rects[0].top);

    for position in input.custom_positions {
        if position.slot < rects.len() {
            rects[position.slot] = RECT {
                left: position.x,
                top: position.y,
                right: position.x + position.width as i32,
                bottom: position.y + position.height as i32,
            };
        }
    }

    LayoutPlan {
        rects,
        strip_width,
        strip_height,
    }
}

fn scale(value: i32, dpi_scale: f64) -> i32 {
    (value as f64 * dpi_scale).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(custom_positions: &[config::PipPosition]) -> LayoutInput<'_> {
        LayoutInput {
            dpi_scale: 1.0,
            monitor_rect: RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            pip_count: 2,
            pip_edge: config::PipEdge::Right,
            custom_strip_width: None,
            custom_positions,
            gap: 4,
            border: 3,
        }
    }

    #[test]
    fn strip_layout_is_deterministic_and_anchored_to_the_selected_edge() {
        let plan = compute(input(&[]));
        assert_eq!(plan.rects.len(), 2);
        assert_eq!(plan.rects[0].right, 1920);
        assert_eq!(plan.rects[1].right, 1920);
        assert!(plan.rects[1].top > plan.rects[0].top);
        assert!(plan.strip_width > 0);
        assert!(plan.strip_height > plan.rects[0].bottom - plan.rects[0].top);
    }

    #[test]
    fn automatic_layout_is_not_forced_to_configured_override_bounds() {
        let plan = compute(LayoutInput {
            dpi_scale: 1.0,
            monitor_rect: RECT {
                left: 0,
                top: 0,
                right: 100,
                bottom: 10_000,
            },
            pip_count: 1,
            pip_edge: config::PipEdge::Top,
            custom_strip_width: None,
            custom_positions: &[],
            gap: 4,
            border: 3,
        });

        let configured_minimum = (10_000.0 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
        assert!(plan.strip_height < configured_minimum);
    }

    #[test]
    fn configured_strip_size_is_clamped_for_both_orientations() {
        let vertical = compute(LayoutInput {
            custom_strip_width: Some(10_000),
            pip_count: 1,
            ..input(&[])
        });
        assert_eq!(vertical.strip_width, 480);

        let horizontal = compute(LayoutInput {
            pip_edge: config::PipEdge::Top,
            custom_strip_width: Some(10_000),
            pip_count: 1,
            ..input(&[])
        });
        assert_eq!(horizontal.strip_height, 270);
    }

    #[test]
    fn custom_positions_override_cells_without_changing_strip_metrics() {
        let automatic = compute(input(&[]));
        let custom = [config::PipPosition {
            slot: 0,
            x: 100,
            y: 200,
            width: 320,
            height: 180,
        }];
        let overridden = compute(input(&custom));

        assert_eq!(overridden.rects[0].left, 100);
        assert_eq!(overridden.rects[0].top, 200);
        assert_eq!(overridden.rects[0].right, 420);
        assert_eq!(overridden.rects[0].bottom, 380);
        assert_eq!(overridden.strip_width, automatic.strip_width);
        assert_eq!(overridden.strip_height, automatic.strip_height);
    }

    #[test]
    fn bypassed_move_snap_preserves_the_requested_position() {
        let snapped = snap_point(
            MoveSnapInput {
                x: 7,
                y: 11,
                width: 100,
                height: 50,
                monitor: RECT {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
                grid: 16,
                bypass: true,
            },
            &[],
        );
        assert_eq!(snapped, (7, 11));
    }
}
