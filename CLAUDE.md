# Stonemite

EverQuest multiboxing PiP overlay tool for Windows.

- **app/** — system tray application with PiP overlay, window management, click-to-swap

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
just release 0.2.0   # bump version, build release, zip + installer to dist/
just bump 0.2.0      # bump version only
just package         # build release + zip (without version bump)
just installer       # build release + Inno Setup installer only
```

`just release` produces both `dist/stonemite-x86_64-pc-windows-msvc.zip` and `dist/stonemite-0.2.0-setup.exe`. Requires [Inno Setup 6](https://jrsoftware.org/isdl.php). The app uses `self_update` crate to check for updates from `eqlaika/stonemite` GitHub releases.

- `installer.iss` — Inno Setup script (installs to Program Files, Start Menu shortcut, optional autostart)

Update `CHANGELOG.md` before each release. `just release` extracts only the current version's section into `dist/release-notes.md` for the GitHub release. The changelog is also shown to users in the update dialog.

## Architecture

- Cargo workspace with one active member: `app/`

### App structure

- `config.rs` — TOML config at `%APPDATA%\Stonemite\config.toml`
- `tray.rs` — `Shell_NotifyIconW` system tray, hidden message window, context menu, WM_TIMER polling
- `eq_windows.rs` — EQ window enumeration via `EnumWindows`, slot assignment, z-order stacking
- `eq_characters.rs` — character name detection from EQ log files
- `overlay.rs` — PiP overlay window with DWM thumbnails (up to 5), hover highlighting, click-to-swap, drag-to-reorder, character labels
- `build.rs` — embeds app icon as Windows resource

## Style

- UI text uses sentence case: capitalize only the first word and proper nouns (e.g. "Edit layout", "Hide overlay hotkey", "PiP edge"). Applies to menu items, dialog labels, buttons, and descriptions.

## Key docs

- `config/example.toml` — example configuration
- `CHANGELOG.md` — release changelog
