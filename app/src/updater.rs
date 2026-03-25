use self_update::cargo_crate_version;

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
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("eqlaika")
        .repo_name("stonemite")
        .build()
        .and_then(|list| list.fetch());

    match releases {
        Ok(rs) => {
            if let Some(latest) = rs.into_iter().next() {
                let ver = latest.version.trim_start_matches('v').to_string();
                if ver == cargo_crate_version!() {
                    CheckResult::UpToDate
                } else {
                    CheckResult::Available { version: ver }
                }
            } else {
                CheckResult::UpToDate
            }
        }
        Err(e) => CheckResult::Error(e.to_string()),
    }
}

/// Check for a new release on GitHub and apply it if available.
/// Returns the new version and release notes on success.
pub fn check_and_update() -> UpdateResult {
    // Fetch release list to get release notes before updating.
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("eqlaika")
        .repo_name("stonemite")
        .build()
        .and_then(|list| list.fetch());

    let notes = releases
        .ok()
        .and_then(|rs| rs.into_iter().next())
        .and_then(|r| r.body)
        .unwrap_or_default();

    let result = self_update::backends::github::Update::configure()
        .repo_owner("eqlaika")
        .repo_name("stonemite")
        .bin_name("stonemite.exe")
        .target("x86_64-pc-windows-msvc")
        .current_version(cargo_crate_version!())
        .no_confirm(true)
        .build()
        .and_then(|updater| updater.update());

    match result {
        Ok(status) => {
            let latest = status.version();
            if latest == cargo_crate_version!() {
                UpdateResult::UpToDate
            } else {
                UpdateResult::Updated {
                    version: latest.to_string(),
                    notes,
                }
            }
        }
        Err(e) => UpdateResult::Error(e.to_string()),
    }
}

/// Return the current compiled version.
pub fn current_version() -> &'static str {
    cargo_crate_version!()
}

/// Restart the application by spawning the current exe and exiting.
pub fn restart() -> ! {
    let exe = std::env::current_exe().expect("Failed to get current exe path");
    let _ = std::process::Command::new(exe).spawn();
    std::process::exit(0);
}
