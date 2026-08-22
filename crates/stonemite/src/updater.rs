use self_update::update::Release;

use crate::calver::CalVer;

const REPOSITORY_OWNER: &str = "eqlaika";
const REPOSITORY_NAME: &str = "stonemite";
const UPDATE_TARGET: &str = "x86_64-pc-windows-msvc";

pub enum UpdateResult {
    UpToDate,
    Updated { version: String, notes: String },
    Error(String),
}

#[allow(dead_code)]
pub enum CheckResult {
    UpToDate,
    Available { version: String },
    Error(String),
}

/// Check whether a newer release exists on GitHub without downloading it.
pub fn check_for_update() -> CheckResult {
    match newest_available_release() {
        Ok(Some(release)) => CheckResult::Available {
            version: release.version,
        },
        Ok(None) => CheckResult::UpToDate,
        Err(error) => CheckResult::Error(error),
    }
}

/// Check for a new release on GitHub and apply it if available.
/// Returns the new version and release notes on success.
pub fn check_and_update() -> UpdateResult {
    let release = match newest_available_release() {
        Ok(Some(release)) => release,
        Ok(None) => return UpdateResult::UpToDate,
        Err(error) => return UpdateResult::Error(error),
    };

    let version = release.version.clone();
    let tag = release_tag(&release);
    let notes = release.body.unwrap_or_default();
    let result = self_update::backends::github::Update::configure()
        .repo_owner(REPOSITORY_OWNER)
        .repo_name(REPOSITORY_NAME)
        .bin_name("stonemite.exe")
        .target(UPDATE_TARGET)
        // A target tag bypasses self_update's SemVer discovery while retaining
        // its download, extraction, and safe executable replacement machinery.
        .current_version(crate::build_info::release_version())
        .target_version_tag(&tag)
        .no_confirm(true)
        .build()
        .and_then(|updater| updater.update());

    match result {
        Ok(status) if status.updated() && status.version() == version => {
            UpdateResult::Updated { version, notes }
        }
        Ok(status) => UpdateResult::Error(format!(
            "GitHub selected {tag}, but the updater returned {status}"
        )),
        Err(error) => UpdateResult::Error(error.to_string()),
    }
}

fn newest_available_release() -> Result<Option<Release>, String> {
    let current = CalVer::parse(crate::build_info::release_version())
        .map_err(|error| format!("invalid embedded Stonemite version: {error}"))?;
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPOSITORY_OWNER)
        .repo_name(REPOSITORY_NAME)
        // Historical source-only releases intentionally have no updater ZIP.
        .with_target(UPDATE_TARGET)
        .build()
        .and_then(|list| list.fetch())
        .map_err(|error| error.to_string())?;

    Ok(select_newest_release(releases, current))
}

fn select_newest_release(releases: Vec<Release>, current: CalVer) -> Option<Release> {
    releases
        .into_iter()
        .filter(|release| release.has_target_asset(UPDATE_TARGET))
        .filter_map(|release| {
            CalVer::parse(&release.version)
                .ok()
                .map(|version| (version, release))
        })
        .filter(|(version, _)| *version > current)
        .max_by_key(|(version, _)| *version)
        .map(|(_, release)| release)
}

fn release_tag(release: &Release) -> String {
    format!("v{}", release.version)
}

/// Return the current application-visible version.
pub fn current_version() -> &'static str {
    crate::build_info::version()
}

/// Restart the application by spawning the current exe and exiting.
pub fn restart() -> ! {
    let exe = std::env::current_exe().expect("Failed to get current exe path");
    let _ = std::process::Command::new(exe).spawn();
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use self_update::update::ReleaseAsset;

    use super::*;

    fn release(version: &str, has_update: bool, notes: &str) -> Release {
        Release {
            name: format!("v{version}"),
            version: version.to_owned(),
            date: String::new(),
            body: Some(notes.to_owned()),
            assets: has_update
                .then(|| ReleaseAsset {
                    download_url: String::new(),
                    name: format!("stonemite-{UPDATE_TARGET}.zip"),
                })
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn selects_newest_update_independent_of_api_order() {
        let current = CalVer::parse("2026.08.22").unwrap();
        let selected = select_newest_release(
            vec![
                release("2026.08.23", true, "newest notes"),
                release("0.5.0", true, "legacy"),
                release("2026.08.24", false, "source only"),
                release("2026.08.22.2", true, "older notes"),
                release("2026.08.22.1", true, "oldest notes"),
            ],
            current,
        )
        .unwrap();

        assert_eq!(selected.version, "2026.08.23");
        assert_eq!(selected.body.as_deref(), Some("newest notes"));
        assert_eq!(release_tag(&selected), "v2026.08.23");
    }

    #[test]
    fn ignores_equal_older_malformed_and_source_only_releases() {
        let current = CalVer::parse("2026.08.22.1").unwrap();
        let selected = select_newest_release(
            vec![
                release("2026.08.22.1", true, "equal"),
                release("2026.08.22", true, "older"),
                release("2026.08.22.2", false, "source only"),
                release("not-a-version", true, "malformed"),
            ],
            current,
        );

        assert!(selected.is_none());
    }
}
