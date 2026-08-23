use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::clients::apply_preferred_box_order;
use super::control_bridge::publish;
use super::edit_mode::toggle_inner as toggle_edit_mode_inner;
use super::event_loop::publish_log_sources_inner;
use super::hosts::{rebuild_thumbnails, update_active_label};
use super::runtime::try_with_state_mut;
use super::state::OverlayState;
use super::surfaces::update_visibility;
use crate::{config, eq_characters};

const IDM_CHAR_BASE: u32 = 5000;
const IDM_NUMBER_BASE: u32 = 6000;
const IDM_EDGE_BASE: u32 = 7000;
const IDM_HIDE_OVERLAY: u32 = 7100;
const IDM_EDIT_MODE: u32 = 7200;
const IDM_RESET_LAYOUT: u32 = 7300;
const IDM_BROADCAST_TOGGLE: u32 = 7400;
const IDM_SETTINGS: u32 = 7500;

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

pub(super) unsafe fn show_char_menu(
    s: &mut OverlayState,
    target_pid: u32,
    screen_pt: POINT,
    owner_hwnd: HWND,
) {
    let cfg = config::Config::load();
    let eq_dir = cfg.eq_directory();
    let candidates = eq_characters::find_active_characters(&eq_dir, Duration::from_secs(86400));

    let Ok(hmenu) = CreatePopupMenu() else { return };

    // Character assignment submenu, grouped by server.
    if let Ok(char_menu) = CreatePopupMenu() {
        let mut servers: Vec<String> = Vec::new();
        for c in &candidates {
            if !servers.contains(&c.server) {
                servers.push(c.server.clone());
            }
        }

        if servers.len() == 1 {
            for (i, c) in candidates.iter().enumerate() {
                let label = format!("{}\0", c.character);
                let wide: Vec<u16> = label.encode_utf16().collect();
                let _ = AppendMenuW(
                    char_menu,
                    MF_STRING,
                    (IDM_CHAR_BASE + i as u32) as usize,
                    windows::core::PCWSTR(wide.as_ptr()),
                );
            }
        } else {
            for server in &servers {
                let Ok(server_menu) = CreatePopupMenu() else {
                    continue;
                };
                for (i, c) in candidates.iter().enumerate() {
                    if c.server != *server {
                        continue;
                    }
                    let label = format!("{}\0", c.character);
                    let wide: Vec<u16> = label.encode_utf16().collect();
                    let _ = AppendMenuW(
                        server_menu,
                        MF_STRING,
                        (IDM_CHAR_BASE + i as u32) as usize,
                        windows::core::PCWSTR(wide.as_ptr()),
                    );
                }
                let server_label = format!("{server}\0");
                let wide: Vec<u16> = server_label.encode_utf16().collect();
                let _ = AppendMenuW(
                    char_menu,
                    MF_POPUP,
                    server_menu.0 as usize,
                    windows::core::PCWSTR(wide.as_ptr()),
                );
            }
        }

        if !candidates.is_empty() {
            let assign_label: Vec<u16> = "Assign character\0".encode_utf16().collect();
            let _ = AppendMenuW(
                hmenu,
                MF_POPUP,
                char_menu.0 as usize,
                windows::core::PCWSTR(assign_label.as_ptr()),
            );
        }
    }

    // Number reassignment submenu.
    if let Ok(num_menu) = CreatePopupMenu() {
        for n in 1..=s.clients.windows.len() {
            let label = format!("#{n}\0");
            let wide: Vec<u16> = label.encode_utf16().collect();
            let _ = AppendMenuW(
                num_menu,
                MF_STRING,
                (IDM_NUMBER_BASE + n as u32) as usize,
                windows::core::PCWSTR(wide.as_ptr()),
            );
        }
        let num_label: Vec<u16> = "Assign number\0".encode_utf16().collect();
        let _ = AppendMenuW(
            hmenu,
            MF_POPUP,
            num_menu.0 as usize,
            windows::core::PCWSTR(num_label.as_ptr()),
        );
    }

    // PiP Edge submenu.
    let Ok(edge_menu) = CreatePopupMenu() else {
        let _ = DestroyMenu(hmenu);
        return;
    };
    let edge_options = [
        (config::PipEdge::Right, "Right"),
        (config::PipEdge::Left, "Left"),
        (config::PipEdge::Top, "Top"),
        (config::PipEdge::Bottom, "Bottom"),
    ];
    for (i, (edge, label)) in edge_options.iter().enumerate() {
        let text = format!("{label}\0");
        let wide: Vec<u16> = text.encode_utf16().collect();
        let flags = if *edge == s.layout.pip_edge {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(
            edge_menu,
            flags,
            (IDM_EDGE_BASE + i as u32) as usize,
            windows::core::PCWSTR(wide.as_ptr()),
        );
    }
    let edge_label: Vec<u16> = "PiP edge\0".encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_POPUP,
        edge_menu.0 as usize,
        windows::core::PCWSTR(edge_label.as_ptr()),
    );

    // Edit/Lock layout toggle.
    let edit_label = if s.interaction.edit_mode {
        "Lock layout\0"
    } else {
        "Edit layout\0"
    };
    let edit_wide: Vec<u16> = edit_label.encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_EDIT_MODE as usize,
        windows::core::PCWSTR(edit_wide.as_ptr()),
    );

    // Reset to auto layout (only when custom positions exist).
    if s.layout.has_custom_positions {
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            IDM_RESET_LAYOUT as usize,
            w!("Reset to auto layout"),
        );
    }

    // Broadcasting toggle (only shown if trusik is enabled).
    if cfg.trusik {
        let bc_label = if crate::broadcast::is_active() {
            format!("Broadcasting: on\t{}\0", cfg.broadcast_hotkey)
        } else {
            format!("Broadcasting: off\t{}\0", cfg.broadcast_hotkey)
        };
        let bc_wide: Vec<u16> = bc_label.encode_utf16().collect();
        let bc_flag = if crate::broadcast::is_active() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | bc_flag,
            IDM_BROADCAST_TOGGLE as usize,
            windows::core::PCWSTR(bc_wide.as_ptr()),
        );
    }

    // Hide overlay item with hotkey hint.
    let hide_label = format!("Hide overlay\t{}\0", cfg.hide_hotkey);
    let hide_wide: Vec<u16> = hide_label.encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_HIDE_OVERLAY as usize,
        windows::core::PCWSTR(hide_wide.as_ptr()),
    );

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
    let settings_label: Vec<u16> = "Settings...\0".encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_SETTINGS as usize,
        windows::core::PCWSTR(settings_label.as_ptr()),
    );

    s.interaction.context_menu_target_pid = Some(target_pid);
    s.interaction.context_menu_candidates = candidates;
    s.interaction.context_menu_open = true;

    // TrackPopupMenu runs a nested message loop. The runtime transaction stays
    // busy while the selected command is returned instead of dispatching a
    // re-entrant WM_COMMAND against this state borrow.
    let _ = SetForegroundWindow(owner_hwnd);
    update_visibility(s);
    let selected = TrackPopupMenu(
        hmenu,
        TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
        screen_pt.x,
        screen_pt.y,
        0,
        owner_hwnd,
        None,
    )
    .0 as u32;

    if selected != 0 {
        apply_menu_command(s, selected);
    }
    s.interaction.context_menu_open = false;
    update_visibility(s);
    let _ = DestroyMenu(hmenu);
    let _ = PostMessageW(owner_hwnd, WM_NULL, WPARAM(0), LPARAM(0));
}

pub(super) unsafe fn handle_menu_command(cmd_id: u32) {
    let _ = try_with_state_mut(|state| unsafe { apply_menu_command(state, cmd_id) });
}

pub(super) unsafe fn apply_menu_command(s: &mut OverlayState, cmd_id: u32) {
    if cmd_id == IDM_BROADCAST_TOGGLE {
        let _ = crate::broadcast::toggle();
        update_active_label(s);
        update_visibility(s);
        publish(s);
    } else if cmd_id == IDM_HIDE_OVERLAY {
        s.hidden_by_user = true;
        update_visibility(s);
    } else if cmd_id == IDM_SETTINGS {
        crate::settings_dialog::show();
    } else if cmd_id == IDM_EDIT_MODE {
        toggle_edit_mode_inner(s);
    } else if cmd_id == IDM_RESET_LAYOUT {
        let mut cfg = config::Config::load();
        cfg.pip_positions.clear();
        let _ = cfg.save();
        s.layout.has_custom_positions = false;
        s.interaction.edit_mode = false;
        rebuild_thumbnails(s);
        update_visibility(s);
    } else if (IDM_EDGE_BASE..IDM_EDGE_BASE + 4).contains(&cmd_id) {
        handle_edge_assign(s, cmd_id);
    } else if (IDM_NUMBER_BASE..IDM_NUMBER_BASE + 100).contains(&cmd_id) {
        let number = (cmd_id - IDM_NUMBER_BASE) as usize;
        handle_number_assign(s, number);
    } else if (IDM_CHAR_BASE..IDM_CHAR_BASE + 100).contains(&cmd_id) {
        handle_char_assign(s, cmd_id);
    }
}

unsafe fn handle_char_assign(s: &mut OverlayState, cmd_id: u32) {
    let char_idx = (cmd_id - IDM_CHAR_BASE) as usize;
    let Some(target_pid) = s.interaction.context_menu_target_pid.take() else {
        return;
    };
    let candidates = std::mem::take(&mut s.interaction.context_menu_candidates);

    let Some(candidate) = candidates.get(char_idx) else {
        return;
    };

    if let Some(w) = s.clients.windows.iter_mut().find(|w| w.pid == target_pid) {
        w.class = s
            .telemetry
            .character_cache
            .get_class(&candidate.server, &candidate.character)
            .map(String::from);
        w.character = Some(candidate.character.clone());
        w.server = Some(candidate.server.clone());
    }
    s.telemetry
        .character_cache
        .remember(&candidate.server, &candidate.character);
    s.telemetry.character_cache.save();

    if apply_preferred_box_order(&mut s.clients.windows, &s.clients.preferred_order)
        && config::Config::load().auto_order
    {
        s.clients.apply_auto_order();
    }
    publish_log_sources_inner(s);
    rebuild_thumbnails(s);
    publish(s);
}

unsafe fn handle_number_assign(s: &mut OverlayState, new_number: usize) {
    let Some(target_pid) = s.interaction.context_menu_target_pid.take() else {
        return;
    };
    let _ = std::mem::take(&mut s.interaction.context_menu_candidates);

    let old_number = s
        .clients
        .windows
        .iter()
        .find(|w| w.pid == target_pid)
        .map(|w| w.number)
        .unwrap_or(0);
    // Swap numbers with any window that already has new_number.
    // If the target had no number (0), assign the displaced window the next available number.
    let replacement = if old_number > 0 {
        old_number
    } else {
        // Target was unassigned — give displaced window the next free number.
        let mut n = 1;
        while s.clients.windows.iter().any(|w| w.number == n) || n == new_number {
            n += 1;
        }
        n
    };
    if let Some(other) = s
        .clients
        .windows
        .iter_mut()
        .find(|w| w.number == new_number && w.pid != target_pid)
    {
        other.number = replacement;
    }
    if let Some(w) = s.clients.windows.iter_mut().find(|w| w.pid == target_pid) {
        w.number = new_number;
    }

    rebuild_thumbnails(s);
    publish(s);
}

unsafe fn handle_edge_assign(s: &mut OverlayState, cmd_id: u32) {
    let edge = match cmd_id - IDM_EDGE_BASE {
        0 => config::PipEdge::Right,
        1 => config::PipEdge::Left,
        2 => config::PipEdge::Top,
        3 => config::PipEdge::Bottom,
        _ => return,
    };
    // Switching edge resets to strip auto-layout: clear custom positions and
    // strip width (especially when changing orientation).
    s.layout.pip_edge = edge;
    s.layout.custom_strip_width = None;
    s.layout.has_custom_positions = false;
    s.interaction.edit_mode = false;
    let mut cfg = config::Config::load();
    cfg.pip_edge = edge;
    cfg.pip_strip_width = None;
    cfg.pip_positions.clear();
    let _ = cfg.save();
    rebuild_thumbnails(s);
    update_visibility(s);
}
