pub(super) fn pid_for_log_source(
    windows: &[crate::eq_windows::EqWindow],
    source: &crate::log_watcher::LogSource,
) -> Option<u32> {
    if let Some(pid) = source.id.as_str().strip_prefix("pid:") {
        return pid
            .parse()
            .ok()
            .filter(|pid| windows.iter().any(|window| window.pid == *pid));
    }

    let mut matches = windows.iter().filter(|window| {
        window
            .character
            .as_deref()
            .is_some_and(|character| character.eq_ignore_ascii_case(&source.character))
            && window
                .server
                .as_deref()
                .is_some_and(|server| server.eq_ignore_ascii_case(&source.server))
    });
    let pid = matches.next()?.pid;
    matches.next().is_none().then_some(pid)
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::HWND;

    use super::*;

    fn window(pid: u32) -> crate::eq_windows::EqWindow {
        crate::eq_windows::EqWindow {
            hwnd: HWND::default(),
            pid,
            number: pid as usize,
            character: Some("Bilka".to_owned()),
            server: Some("Xegony".to_owned()),
            class: None,
        }
    }

    #[test]
    fn exact_pid_sources_fail_closed_when_stale_and_legacy_identity_is_unambiguous() {
        let windows = [window(7)];
        let stale_exact = crate::log_watcher::LogSource::new("pid:8", "Bilka", "Xegony");
        assert_eq!(pid_for_log_source(&windows, &stale_exact), None);

        let legacy = crate::log_watcher::LogSource::new("legacy", "bilka", "xegony");
        assert_eq!(pid_for_log_source(&windows, &legacy), Some(7));
        assert_eq!(pid_for_log_source(&[window(7), window(8)], &legacy), None);
    }
}
