/// Return the canonical public version embedded in this build.
///
/// This is always `YYYY.MM.DD[.N]`, including in development builds. Cargo and
/// Tauri use a separate, generated three-component encoding internally.
pub const fn release_version() -> &'static str {
    env!("STONEMITE_VERSION")
}

/// Return the version label presented by this build.
///
/// Development deployment can add commit and dirty-worktree information without
/// changing the canonical release identity used by the updater.
pub const fn version() -> &'static str {
    match option_env!("STONEMITE_BUILD_LABEL") {
        Some(label) => label,
        None => release_version(),
    }
}

/// Return whether this build uses the development profile family.
pub const fn is_development() -> bool {
    cfg!(stonemite_dev_build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_release_version_is_canonical_calver() {
        let parsed = crate::calver::CalVer::parse(release_version()).unwrap();
        assert_eq!(parsed.to_string(), release_version());
    }

    #[test]
    fn version_label_is_not_empty() {
        assert!(!version().is_empty());
    }
}
