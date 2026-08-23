use std::collections::HashMap;

use windows::Win32::Foundation::HWND;

use crate::{config, eq_windows::EqWindow};

pub(super) const MAX_PIPS: usize = 5;

/// Authoritative client identity and active/PiP partition.
pub(super) struct ClientRegistry {
    pub(super) windows: Vec<EqWindow>,
    pip_order: Vec<u32>,
    pub(super) preferred_order: Vec<config::BoxIdentity>,
    active_pid: Option<u32>,
    pub(super) observed_identities: HashMap<u32, (String, String)>,
}

pub(super) struct ClientPartition {
    active_pid: Option<u32>,
    pip_order: Vec<u32>,
}

impl ClientRegistry {
    pub(super) fn new(preferred_order: Vec<config::BoxIdentity>) -> Self {
        Self {
            windows: Vec::new(),
            pip_order: Vec::new(),
            preferred_order,
            active_pid: None,
            observed_identities: HashMap::new(),
        }
    }

    pub(super) fn active_pid(&self) -> Option<u32> {
        self.active_pid
    }

    pub(super) fn pips(&self) -> &[u32] {
        &self.pip_order
    }

    pub(super) fn pip_at(&self, index: usize) -> Option<u32> {
        self.pip_order.get(index).copied()
    }

    pub(super) fn snapshot_partition(&self) -> ClientPartition {
        ClientPartition {
            active_pid: self.active_pid,
            pip_order: self.pip_order.clone(),
        }
    }

    pub(super) fn restore_partition(&mut self, partition: ClientPartition) {
        self.active_pid = partition.active_pid;
        self.pip_order = partition.pip_order;
    }

    pub(super) fn remove(&mut self, pid: u32) {
        self.windows.retain(|window| window.pid != pid);
        self.observed_identities.remove(&pid);
        self.pip_order.retain(|pip| *pip != pid);
        if self.active_pid == Some(pid) {
            self.active_pid = self.pip_order.first().copied();
            if let Some(promoted) = self.active_pid {
                self.pip_order.retain(|pip| *pip != promoted);
            }
        }
    }

    pub(super) fn add(&mut self, window: EqWindow, prefer_active: bool) {
        let pid = window.pid;
        self.windows.push(window);
        if self.active_pid.is_none() && prefer_active {
            self.active_pid = Some(pid);
        } else {
            self.pip_order.push(pid);
        }
    }

    pub(super) fn ensure_active(&mut self) {
        if self.active_pid.is_none() {
            if let Some(first) = self.pip_order.first().copied() {
                self.active_pid = Some(first);
                self.pip_order.retain(|pip| *pip != first);
            }
        }
    }

    pub(super) fn truncate_pips(&mut self, maximum: usize) {
        self.pip_order.truncate(maximum);
    }

    pub(super) fn swap_pips(&mut self, first: usize, second: usize) -> bool {
        if first >= self.pip_order.len() || second >= self.pip_order.len() {
            return false;
        }
        self.pip_order.swap(first, second);
        true
    }

    /// Promote a known client into the active partition. A client outside the
    /// visible PiP set replaces the last PiP with the previous active client.
    pub(super) fn promote(&mut self, target_pid: u32, max_pips: usize) -> bool {
        if exchange_active_with_pip(&mut self.active_pid, &mut self.pip_order, target_pid) {
            return true;
        }
        if self.active_pid == Some(target_pid)
            || !self.windows.iter().any(|window| window.pid == target_pid)
        {
            return false;
        }

        let previous_active = self.active_pid.replace(target_pid);
        if let Some(previous_active) = previous_active {
            if self.pip_order.len() < max_pips {
                self.pip_order.push(previous_active);
            } else if max_pips > 0 {
                self.pip_order[max_pips - 1] = previous_active;
                self.pip_order.truncate(max_pips);
            }
        }
        true
    }

    pub(super) fn apply_auto_order(&mut self) {
        self.pip_order.sort_by_key(|pid| {
            self.windows
                .iter()
                .find(|window| window.pid == *pid)
                .map_or(usize::MAX, |window| window.number)
        });
    }

    #[cfg(debug_assertions)]
    pub(super) fn debug_assert_partition(&self) {
        use std::collections::HashSet;

        let known: HashSet<u32> = self.windows.iter().map(|window| window.pid).collect();
        let unique_pips: HashSet<u32> = self.pip_order.iter().copied().collect();
        debug_assert_eq!(unique_pips.len(), self.pip_order.len());
        debug_assert!(self.active_pid.is_none_or(|pid| known.contains(&pid)));
        debug_assert!(self
            .active_pid
            .is_none_or(|pid| !unique_pips.contains(&pid)));
        debug_assert!(self.pip_order.iter().all(|pid| known.contains(pid)));
    }

    #[cfg(not(debug_assertions))]
    pub(super) fn debug_assert_partition(&self) {}
}

/// Return the lowest positive integer not already used by any tracked window.
pub(super) fn next_available_number(eq_windows: &[EqWindow]) -> usize {
    let mut n = 1;
    while eq_windows.iter().any(|w| w.number == n) {
        n += 1;
    }
    n
}

/// Assign contiguous window numbers from the configured identity priority.
/// Unlisted and not-yet-identified clients follow in their current relative order.
pub(super) fn apply_preferred_box_order(
    eq_windows: &mut [EqWindow],
    preferred: &[config::BoxIdentity],
) -> bool {
    if preferred.is_empty() || eq_windows.is_empty() {
        return false;
    }

    let mut ordered: Vec<(u32, usize, Option<usize>)> = eq_windows
        .iter()
        .map(|window| {
            let rank = window
                .server
                .as_deref()
                .zip(window.character.as_deref())
                .and_then(|(server, character)| {
                    preferred
                        .iter()
                        .position(|identity| identity.matches(server, character))
                });
            (window.pid, window.number, rank)
        })
        .collect();
    ordered.sort_by_key(|(pid, number, rank)| {
        (rank.is_none(), rank.unwrap_or(*number), *number, *pid)
    });

    let mut changed = false;
    for (index, (pid, _, _)) in ordered.into_iter().enumerate() {
        let desired_number = index + 1;
        if let Some(window) = eq_windows.iter_mut().find(|window| window.pid == pid) {
            if window.number != desired_number {
                window.number = desired_number;
                changed = true;
            }
        }
    }
    changed
}

/// Exchange the stable window numbers of two loaded clients.
pub(super) fn exchange_window_numbers(
    eq_windows: &mut [EqWindow],
    first_pid: u32,
    second_pid: u32,
) -> Option<(usize, usize)> {
    let first_index = eq_windows
        .iter()
        .position(|window| window.pid == first_pid)?;
    let second_index = eq_windows
        .iter()
        .position(|window| window.pid == second_pid)?;
    let first_number = eq_windows[first_index].number;
    let second_number = eq_windows[second_index].number;
    if first_index != second_index {
        eq_windows[first_index].number = second_number;
        eq_windows[second_index].number = first_number;
    }
    Some((first_number, second_number))
}

pub(super) fn focused_foreground_pid(
    windows: &[EqWindow],
    foreground: HWND,
    mut has_keyboard_focus: impl FnMut(HWND) -> bool,
) -> Option<u32> {
    windows
        .iter()
        .find(|window| window.hwnd == foreground && has_keyboard_focus(window.hwnd))
        .map(|window| window.pid)
}

/// Exchange a PiP client with the active client while preserving the partition.
/// Returns false when the target is already active or is not currently a PiP.
pub(super) fn exchange_active_with_pip(
    active_pid: &mut Option<u32>,
    pip_order: &mut Vec<u32>,
    target_pid: u32,
) -> bool {
    if *active_pid == Some(target_pid) {
        return false;
    }
    let Some(position) = pip_order.iter().position(|pid| *pid == target_pid) else {
        return false;
    };
    if let Some(old_active) = *active_pid {
        pip_order[position] = old_active;
    } else {
        pip_order.remove(position);
    }
    *active_pid = Some(target_pid);
    true
}

/// Reconcile a newly observed automatic identity without overwriting a manual
/// assignment when the automatic source has not changed.
pub(super) fn reconcile_identity(
    observed: &mut HashMap<u32, (String, String)>,
    window: &mut EqWindow,
    character: String,
    server: String,
    class: Option<String>,
) -> bool {
    let unchanged = observed
        .get(&window.pid)
        .is_some_and(|(old_character, old_server)| {
            old_character.eq_ignore_ascii_case(&character)
                && old_server.eq_ignore_ascii_case(&server)
        });
    if unchanged {
        return false;
    }

    observed.insert(window.pid, (character.clone(), server.clone()));
    window.character = Some(character);
    window.server = Some(server);
    window.class = class;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(pid: u32) -> EqWindow {
        EqWindow {
            hwnd: HWND::default(),
            pid,
            number: pid as usize,
            character: None,
            server: None,
            class: None,
        }
    }

    fn identified_window(pid: u32, number: usize, server: &str, character: &str) -> EqWindow {
        let mut window = window(pid);
        window.number = number;
        window.server = Some(server.into());
        window.character = Some(character.into());
        window
    }

    #[test]
    fn promoting_a_known_client_outside_the_pips_preserves_a_bounded_partition() {
        let mut clients = ClientRegistry::new(Vec::new());
        clients.windows = (1..=7).map(window).collect();
        clients.active_pid = Some(1);
        clients.pip_order = vec![2, 3, 4, 5, 6];

        assert!(clients.promote(7, 5));
        assert_eq!(clients.active_pid, Some(7));
        assert_eq!(clients.pip_order, vec![2, 3, 4, 5, 1]);
        clients.debug_assert_partition();
    }

    #[test]
    fn preferred_order_is_global_case_insensitive_and_compacts_missing_entries() {
        let preferred = vec![
            config::BoxIdentity {
                server: "xegony".into(),
                character: "Laika".into(),
            },
            config::BoxIdentity {
                server: "bristlebane".into(),
                character: "Foo".into(),
            },
            config::BoxIdentity {
                server: "xegony".into(),
                character: "Kafka".into(),
            },
        ];
        let mut windows = vec![
            identified_window(10, 1, "XEGONY", "Kafka"),
            identified_window(20, 2, "Bristlebane", "foo"),
            identified_window(30, 3, "Xegony", "Unlisted"),
        ];

        assert!(apply_preferred_box_order(&mut windows, &preferred));
        assert_eq!(
            windows
                .iter()
                .map(|window| (window.pid, window.number))
                .collect::<Vec<_>>(),
            vec![(10, 2), (20, 1), (30, 3)]
        );
        assert!(!apply_preferred_box_order(&mut windows, &preferred));
    }

    #[test]
    fn unlisted_windows_retain_relative_order_after_ranked_windows() {
        let preferred = vec![config::BoxIdentity {
            server: "xegony".into(),
            character: "Laika".into(),
        }];
        let mut windows = vec![
            identified_window(10, 4, "Xegony", "UnknownOne"),
            identified_window(20, 2, "Xegony", "Laika"),
            identified_window(30, 3, "Teek", "UnknownTwo"),
        ];

        assert!(apply_preferred_box_order(&mut windows, &preferred));
        assert_eq!(
            windows
                .iter()
                .map(|window| (window.pid, window.number))
                .collect::<Vec<_>>(),
            vec![(10, 3), (20, 1), (30, 2)]
        );
    }

    #[test]
    fn empty_preference_preserves_existing_window_numbers() {
        let mut windows = vec![window(10), window(20)];
        windows[0].number = 4;
        windows[1].number = 2;

        assert!(!apply_preferred_box_order(&mut windows, &[]));
        assert_eq!(windows[0].number, 4);
        assert_eq!(windows[1].number, 2);
    }

    #[test]
    fn swapping_window_numbers_preserves_identity_and_supports_a_no_op() {
        let mut windows = vec![window(10), window(20), window(30)];
        windows[0].number = 1;
        windows[1].number = 2;
        windows[2].number = 3;

        assert_eq!(exchange_window_numbers(&mut windows, 10, 30), Some((1, 3)));
        assert_eq!(
            windows
                .iter()
                .map(|window| (window.pid, window.number))
                .collect::<Vec<_>>(),
            vec![(10, 3), (20, 2), (30, 1)]
        );
        assert_eq!(exchange_window_numbers(&mut windows, 20, 20), Some((2, 2)));
        assert!(exchange_window_numbers(&mut windows, 10, 99).is_none());
    }

    #[test]
    fn foreground_pid_requires_confirmed_keyboard_focus_during_client_changes() {
        let hwnd = HWND(42usize as *mut _);
        let mut candidate = window(42);
        candidate.hwnd = hwnd;
        let windows = vec![candidate];

        assert_eq!(focused_foreground_pid(&windows, hwnd, |_| false), None);
        assert_eq!(focused_foreground_pid(&windows, hwnd, |_| true), Some(42));
        assert_eq!(
            focused_foreground_pid(&windows, HWND(99usize as *mut _), |_| true),
            None
        );
    }

    #[test]
    fn rapid_foreground_changes_preserve_the_client_partition() {
        let mut active = Some(1);
        let mut pips = vec![2, 3, 4];

        for target in [2, 3, 2, 4, 3, 1, 4] {
            assert!(exchange_active_with_pip(&mut active, &mut pips, target));
            let mut partition = vec![active.expect("an active client")];
            partition.extend(pips.iter().copied());
            partition.sort_unstable();
            assert_eq!(partition, vec![1, 2, 3, 4]);
        }

        let before = (active, pips.clone());
        assert!(!exchange_active_with_pip(&mut active, &mut pips, 99));
        assert_eq!((active, pips), before);
    }

    #[test]
    fn new_identity_refreshes_but_unchanged_identity_preserves_manual_assignment() {
        let mut observed = HashMap::new();
        let mut window = window(42);

        assert!(reconcile_identity(
            &mut observed,
            &mut window,
            "Orlov".into(),
            "teek".into(),
            Some("SHK".into()),
        ));
        window.character = Some("Manual".into());
        window.server = Some("assignment".into());
        window.class = Some("CLR".into());

        assert!(!reconcile_identity(
            &mut observed,
            &mut window,
            "orlov".into(),
            "TEEK".into(),
            None,
        ));
        assert_eq!(window.character.as_deref(), Some("Manual"));

        assert!(reconcile_identity(
            &mut observed,
            &mut window,
            "Laika".into(),
            "xegony".into(),
            Some("SHM".into()),
        ));
        assert_eq!(window.character.as_deref(), Some("Laika"));
        assert_eq!(window.server.as_deref(), Some("xegony"));
        assert_eq!(window.class.as_deref(), Some("SHM"));
    }
}
