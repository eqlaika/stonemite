/// Return the version label presented by this build.
///
/// Release builds use the Cargo package version. Development deployment can
/// override the label without changing release manifests or updater semantics.
pub const fn version() -> &'static str {
    match option_env!("STONEMITE_BUILD_LABEL") {
        Some(label) => label,
        None => env!("CARGO_PKG_VERSION"),
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
    fn version_label_is_not_empty() {
        assert!(!version().is_empty());
    }
}
