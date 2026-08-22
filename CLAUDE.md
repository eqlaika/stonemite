# Stonemite

EverQuest multiboxing tool for Windows.

- **crates/stonemite/** — system tray application with PiP overlay, swap hotkeys, key broadcasting, character autodetect

## Build

Requires [just](https://github.com/casey/just) task runner.

```
just build           # debug build
just build-release   # release build
just run             # quit running instance, build, and launch
just quit            # quit a running instance
just version         # print current version
just clean           # remove build artifacts and dist/
```

You can also use cargo directly:

```
cargo build -p stonemite
cargo build --release -p stonemite
```

Target: `x86_64-pc-windows-msvc`

## Release

```
just release 2026.08.22   # set version, build release, zip + installer to dist/
just bump 2026.08.22      # set and synchronize version metadata only
just package              # build release + zip (without version bump)
just installer            # build release + Inno Setup installer only
```

Public versions use the Pacific release date as `YYYY.MM.DD`; append `.1`, `.2`, and so on for additional releases that day. `VERSION` is canonical, while `scripts/version.py` maintains the order-preserving Cargo/Tauri encoding. `just release` produces both `dist/stonemite-x86_64-pc-windows-msvc.zip` and `dist/stonemite-2026.08.22-setup.exe`. Requires [Inno Setup 6](https://jrsoftware.org/isdl.php). The app uses `self_update` for GitHub release discovery, download, and executable replacement.

- `installer.iss` — Inno Setup script (installs to Program Files, Start Menu shortcut, optional autostart)

Update `CHANGELOG.md` before each release. `just release` extracts only the current version's section into `dist/release-notes.md` for the GitHub release. The changelog is also shown to users in the update dialog.

## Architecture

- Cargo workspace members under `crates/`:
  - `crates/stonemite/` — Windows tray application
  - `crates/eqlog/` — reusable, platform-neutral EQ log parser/event/telemetry package
  - `crates/trushar/` — integration protocol/server
  - `crates/trusik/` — injected DirectInput proxy

### App structure

- `config.rs` — TOML config at `%APPDATA%\Stonemite\config.toml`
- `tray.rs` — `Shell_NotifyIconW` system tray, hidden message window, context menu, WM_TIMER polling
- `eq_windows.rs` — EQ window enumeration via `EnumWindows`, slot assignment, z-order stacking
- `eq_characters.rs` — character name detection from EQ log files
- `log_watcher/` — Windows filesystem watcher, authoritative byte-offset tailer, bounded worker runtime, and app adapters built on `eqlog`
- `overlay.rs` — PiP overlay window with DWM thumbnails (up to 5), hover highlighting, click-to-swap, drag-to-reorder, character labels
- `build.rs` — embeds app icon as Windows resource

## Style

- UI text uses sentence case: capitalize only the first word and proper nouns (e.g. "Edit layout", "Hide overlay hotkey", "PiP edge"). Applies to menu items, dialog labels, buttons, and descriptions.

## Key docs

- `config/example.toml` — example configuration
- `CHANGELOG.md` — release changelog
