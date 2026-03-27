# Stonemite

<p align="center">
  <img src="app/assets/app.png" width="128" alt="Stonemite">
</p>

EverQuest multiboxing tool for Windows.

> **Note:** Requires Windows 10 or later. Stonemite works best with EQ's borderless fullscreen mode, added in the March 2026 patch.

Stonemite makes multiboxing EQ easy — PiP overlays with click-to-swap, swap hotkeys, key broadcasting, auto-login, automatic character detection, and a drag-and-drop layout editor. No subscription, no setup wizard.

## Install

Download the latest release from [GitHub Releases](https://github.com/eqlaika/stonemite/releases):

- **Installer** (`stonemite-x.y.z-setup.exe`) — installs to Program Files, creates Start Menu shortcut, optional Windows startup
- **Portable** (`stonemite-x86_64-pc-windows-msvc.zip`) — extract and run anywhere

A system tray icon appears with access to all settings. Check for updates from the tray menu.

## Build from source

Requires Rust (MSVC toolchain) and [just](https://github.com/casey/just).

```
just build           # debug build
just build-release   # release build
```

Target: `x86_64-pc-windows-msvc`

## Release

```
just release 0.4.0
```

This bumps the version in `Cargo.toml`, builds a release binary, and packages both `dist/stonemite-x86_64-pc-windows-msvc.zip` and `dist/stonemite-0.4.0-setup.exe`. Requires [Inno Setup 6](https://jrsoftware.org/isdl.php). Then:

1. Commit and tag: `git add -A && git commit -m "Release v0.4.0" && git tag v0.4.0`
2. Push: `git push && git push --tags`
3. Create a GitHub release: `gh release create v0.4.0 dist/stonemite-x86_64-pc-windows-msvc.zip dist/stonemite-0.4.0-setup.exe --title 'v0.4.0' --notes-file dist/release-notes.md`

The app checks for updates against [eqlaika/stonemite](https://github.com/eqlaika/stonemite) GitHub releases via the `self_update` crate.

## Stonemite vs ISBoxer

[ISBoxer](https://isboxer.com/) has been the go-to multiboxing tool for years. It's powerful, but it comes with a subscription, a lengthy setup process, and a lot of complexity most players don't need. Stonemite is a focused alternative:

| | Stonemite | ISBoxer |
|---|---|---|
| **Price** | Free, open source | ~$50/year subscription |
| **Setup** | Run the installer, done | Inner Space install, wizard pages, character slots, window layout configs |
| **PiP overlays** | Native DWM thumbnails, click-to-swap, drag-to-reorder | Video FX regions routed through Inner Space |
| **Character labels** | Auto-detected | Manual per-character setup |
| **Auto-login** | Encrypted credentials, automatic server select, one-click launch | Not available |
| **Input broadcasting** | Key broadcasting with whitelist/blacklist filtering | Full key/mouse broadcasting and round-robin |
| **Window management** | Auto-detects EQ windows, z-order stacking | Window layouts with snapping and resizing |
| **Resource usage** | ~5 MB single exe | Inner Space + ISBoxer addon |
| **Updates** | One-click from system tray | Manual download through Inner Space |

**When to use Stonemite:** You want multiboxing that just works — PiP overlays, swap hotkeys, key broadcasting, auto-login, and character detection with zero setup. Launch it and go.

**When to use ISBoxer:** You need mouse broadcasting, round-robin input, or you're already comfortable with the Inner Space ecosystem.

## Auto-login

Stonemite can launch your EQ accounts and log them in automatically — no patcher, no typing passwords, no clicking through server select. Add your accounts in Settings > Accounts, and Stonemite handles the rest.

### Password security

Your passwords are encrypted using [Windows DPAPI](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/) and stored in your local config file. They are:

- **Encrypted by Windows itself** — Stonemite does not implement its own encryption or manage any keys. Only your Windows user account on your machine can decrypt them.
- **Never transmitted** — passwords are never sent over the network. Stonemite's telemetry only sends an anonymous ID and app version. You can verify this in the [source code](https://github.com/eqlaika/stonemite).
- **Used only to launch EQ** — passwords are decrypted in memory only when launching the game client.

Stonemite is open source. The encryption code is in [`app/src/crypt.rs`](app/src/crypt.rs) and the telemetry code is in [`app/src/telemetry.rs`](app/src/telemetry.rs) — you can audit exactly what the app does with your data.

## Configuration

Config lives at `%APPDATA%\Stonemite\config.toml`. See [config/example.toml](config/example.toml) for options.

## Telemetry

Stonemite sends a single anonymous ping on each launch to help me understand if anyone is using the app. The payload contains only:

- A random anonymous ID (UUID, not tied to your identity)
- App version
- Windows version

No personal information, EQ character names, or config details are collected. Telemetry can be disabled by setting `telemetry = false` in `%APPDATA%\Stonemite\config.toml`, or by checking "Disable anonymous usage telemetry" during installation.

## Disclaimer

Stonemite uses standard Windows DWM thumbnail APIs to display copies of your game windows — the same mechanism Windows uses for taskbar previews and Alt-Tab. The DLL proxy handles character detection (reading log file names), key broadcasting (forwarding keystrokes to background windows), and auto-login (typing credentials into the game client).

Stonemite is substantially less invasive than [ISBoxer](https://isboxer.com/multiboxing/is-isboxer-allowed), which injects into the rendering pipeline and intercepts DirectX calls, and has been widely used for years without bans. Stonemite's DLL proxy only intercepts DirectInput to read log paths and forward keystrokes. It does not touch rendering, read or modify game memory, or sniff network traffic.

Use Stonemite at your own risk. The author is not responsible for any account actions including suspensions, bans, or other consequences resulting from its use.

## License

[GPL v3](LICENSE) — free to use, modify, and distribute. Modified versions must remain open source. Copyright (c) 2026 Laikasoft.
