# Changelog

## Unreleased

### Added

- Added a default-on setting that turns off key broadcasting after the last EverQuest client exits
- Added a default-on in-game Stonemite button anchored beside the PiP strip, with left-click Settings access and the full tray menu on right-click
- Added named box cycles with configurable character rings, wrapping next and previous hotkeys, unavailable-client skipping, and no-repeat keyboard-pedal support
- Added configurable trade-request notifications that clear when EverQuest logs a cancelled trade and remain non-actionable because EQ exposes no trade-accept key binding
- Added configurable character/server box ordering that restores stable window numbers across random launch order, process restarts, and mixed-server rosters
- Added a dedicated hold-to-broadcast Stream Deck Mouse Clutch action with authoritative ready, active, releasing, compatibility, and failure states
- Added connection-scoped, renewable Trushar Mouse Clutch holds with independent physical/deck ownership, bounded lease expiry, and automatic cleanup on disconnect
- Added log-only PiP spell casting bars with persona-specific recent timing estimates and confirmed completion, fizzle, resist, and interruption feedback

### Changed

- Extracted passive EverQuest spell-data parsing into the reusable platform-neutral `eqspell` crate and keyed learned cast timing by stable spell ID
- Changed PiP casting bars to drain smoothly from full to empty at roughly 60 FPS as the cast completes
- Enlarged the in-game Stonemite control into a transparent draggable logo whose monitor-relative position persists across restarts, DPI changes, and display moves
- Kept PiP hosts persistent across client swaps, with a sliding thumbnail handoff and animated reordering instead of rebuilding all five windows through a blank frame
- Pushed Mouse Clutch phase and readiness changes through Trushar so every deck tile reflects physical-key, duplicate-tile, focus-cancellation, and release-drain transitions
- Removed the bard melody-stopped PiP warning

### Fixed

- Resolved sparse character and persona keymap overrides over legacy and shared EverQuest bindings so mapped Stream Deck actions are not falsely reported as unbound
- Prevented PiP mouse input from reaching the foreground EQ client and kept window swapping responsive while PiP context menus are open

## v2026.08.22

### Added

- Added built-in box notifications for incoming tells, group and raid invitations, resurrection offers, and character deaths, with per-box PiP border animation, brief previews, persistent per-event unread dots, per-event toggles, and selectable bundled EQ audio-trigger sounds
- Added user-initiated **Accept** and **Dismiss** controls to eligible group-invite previews; Accept sends the receiving box's effective Invite/Follow binding without activating it, joined/declined log events clear pending invitations, and unavailable input or an unbound action falls back to the ordinary notification
- Added generic parsing and profile-aware resolution for arbitrary EQ `[TextColors]` `User_N` RGB values, used by Tell, group, and raid notifications
- Added a configurable Mouse Clutch, bound to F13 by default, that hold-broadcasts physical mouse movement, buttons, and wheel input from the active EQ client to ready background clients with matching window geometry and DPI
- Added F13–F24 and keyboard-emulating foot-pedal support, live Mouse Clutch status in the overlay and tray, and fail-safe release when focus, clients, settings, or the controlling Stonemite process change
- Added the **Stonemite · EQ boxing** Stream Deck plugin for Stream Deck Desktop 7.4+ on macOS 12+ or Windows 10+, with a live six-client roster, character and class identity, input-readiness indicators, exact-client activation, authoritative broadcast state, automatic reconnection, and visible command errors
- Added a Stream Deck Broadcast key that explicitly turns Stonemite key broadcasting on or off instead of issuing a blind toggle
- Added a two-step Stream Deck Swap key for exchanging the active and selected characters' stable window numbers without changing the active client
- Added generic keymap-aware Trushar actions for all 11×12 EQ hotbar buttons, all 14 spell gems, and arbitrary mapped EQ actions
- Added authoritative active-box and background-box targets alongside all-loaded and stable box-number targeting
- Added a configurable Stream Deck Hotkey tile with a custom name, per-tile color, contrast-aware running content, the complete pinned 466-icon Lucide Animated catalog, and all-loaded, active, background, or stable box-number targets
- Added shared keymap discovery and target-set preflight so a configured hotkey starts on no boxes unless every resolved box is loaded, input-ready, idle, and mapped

### Changed

- Enabled loopback integrations by default and made the Stream Deck plugin connect automatically without authentication when Stream Deck Desktop runs on the Stonemite PC
- Limited address/code authentication to cross-PC LAN access and moved pairing, reconnection, and saved-credential management into the Stonemite setup key
- Resolved semantic actions per character and persona from `Character_Server_Class.ini`, with legacy character INI and shared `eqclient.ini` fallbacks
- Made Stream Deck character slots and controls independently placeable, supported duplicate actions, and removed preset profiles for now
- Kept Stream Deck gameplay controls one-to-one with mapped EQ actions; grouping, following, assisting, and other multi-step behavior now belongs in user-authored in-game socials
- Improved Stream Deck feedback with prominent active-client styling, immediate authoritative state after successful actions, 15px minimum text, a stable Bcast label, distinct action colors, filled running and armed states, and bounded icon animations
- Allowed targeted input sequences to run concurrently across independent EQ clients while still rejecting overlapping sequences for the same client
- Rebuilt EQ log ingestion around filesystem notifications and a background worker, with 500 ms fallback reconciliation and reliable handling of partial writes, truncation, recreation, removed sources, malformed records, and notification loss
- Updated the Stonemite/trusik shared-memory input ABI for independent keyboard and mouse activation; Mouse Clutch requires the matching proxy and an EQ restart after upgrading
- Clarified and documented the project's policy against unattended gameplay automation while permitting only passive log-driven UI telemetry, notifications, and display-only timers

### Fixed

- Fixed arrow and other extended navigation keys being broadcast as their numpad counterparts by preserving DirectInput's extended scan-code bit
- Fixed targeted modifier chords reaching background clients as an unmodified key by sequencing modifier press and release around the primary key
- Fixed rapid client switching corrupting active/PiP label state by guarding overlay swaps against reentrant Windows events and reconciling foreground state during polling
- Fixed swaps updating labels without reliably foregrounding and focusing the requested EQ window; state now commits only after Windows confirms the target owns foreground and keyboard focus
- Fixed programmatic client swaps leaving EQ's DirectInput mouse unacquired until the user clicked by reasserting mouse activation after foreground and focus are stable
- Prevented child processes from inheriting LAN sockets, added graceful `--quit` shutdown, and updated `just quit` so port 19720 is released cleanly

### Developer

- Extracted the reusable, platform-neutral `eqlog` crate with canonical log records, typed `/who` identity and pet events, telemetry reduction, and explicit source-generation resets
- Reorganized the Rust workspace under `crates/` and moved the Trushar protocol documentation alongside its crate
- Added schema-validated generation of the separate `.streamDeckPlugin` artifact, which is not bundled with the desktop installer or portable ZIP, plus Marketplace and plugin assets, pinned third-party notices, and automated formatting, linting, type-checking, protocol, rendering, and package validation
- Added product and Stream Deck visual-system documentation plus GitHub Sponsors metadata

## v2026.08.17

- Added experimental remote integration to support Stream Deck actions.
- Removed anonymous usage telemetry
- Refresh character and server identity when the same EQ process changes characters or servers
- Fixed auto-login failing after a self-update by embedding the matching trusik input DLL in stonemite.exe
- Hardened release builds against missing or invalid embedded trusik DLLs and removed the redundant loose DLL from release packages

## v2026.03.27.1

- Require Windows 10 or later in installer
- Update README for v2026.03.27 features

## v2026.03.27

- Auto-login: automatically enter credentials on the EQ login screen with encrypted password storage and server selection
- Prevent overlay freeze when swapping to/from a zoning EQ window
- Fix DrawTextW crash with empty text

## v2026.03.26

- Hide background EQ windows from Alt-Tab (active window stays visible, enabled by default)
- Add auto order windows setting to keep PiP thumbnails sorted by slot number
- Fix DPI scaling for DPI-unaware EQ windows
- Remove "(right-click)" hint from unknown character labels

## v2026.03.25.1

- Automatic update check on launch with configurable interval
- Open settings window automatically on first launch
- Anchor active window label at top-right when PiP edge is left
- Fix overlay rendering at half size after monitor reconnect on high-DPI displays
- Fix number reassignment displacing a window to number 0
- Fix potential panic when system fails to allocate context menus
- Log hotkey registration failures for easier troubleshooting

## v2026.03.25

- Key broadcasting to background EQ clients
- Swap-to-window hotkeys (Ctrl+F1–F6) with configurable bindings
- Toast notifications for swaps, window closes, and broadcast toggle
- Character class detection with class icons in labels
- Character cache and pet claim detection
- Redesigned PiP labels with number badges, rounded corners, and configurable opacity
- Settings dialog rebuilt with egui
- Settings window remembers its position between opens
- Press-to-capture hotkey binding with modifier support
- Clear broadcast key states when EQ loses focus
- Squircle background on app icons for dark taskbar visibility
- Remove dinput8.dll from EQ directory on uninstall
- Fix crash when an EQ window exits while others remain
- Fix crash when monitor is unplugged
- Fix PiP label z-order after interactions
- Fix active label click-through and hover opacity
- Fix doubled keystrokes from re-injection
- Fix PiP windows hiding when context menu opens

## v2026.03.23

- Free PiP placement: move and resize individual PiPs anywhere on screen
- Edit Layout mode with 8-directional resize and 16:9 aspect enforcement
- Snap-to-grid, snap-to-monitor-edges, and snap-to-other-PiPs (hold Shift to bypass)
- Per-PiP windows (each PiP is its own top-level window with a DWM thumbnail)
- Tray menu: Edit/Lock Layout toggle, Reset to auto layout
- Strip auto-layout preserved as default; free placement is opt-in via Edit Layout
- Custom positions and snap grid size saved to config

## v2026.03.21.1

- Right-click context menu on active window label
- Per-monitor DPI scaling for multi-monitor setups
- Pastel color palette for character labels
- Anonymous opt-out usage telemetry (disable in config or during install)

## v2026.03.21

Initial release.

- PiP overlay with DWM thumbnails (up to 5 windows)
- Click-to-swap between EQ windows
- Drag-to-reorder PiP strips
- Hover highlighting
- Character name labels (auto-detected from EQ log files)
- Active window label
- Configurable PiP strip edge (left, right, top, bottom)
- Drag-to-resize PiP strip
- Settings dialog
- DPI-aware / HiDPI scaling
- System tray with show/hide toggle, hotkey support
- Auto-update from GitHub releases
- Inno Setup installer with optional Windows startup
