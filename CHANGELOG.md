# Changelog

## v0.5.0

- Added experimental remote integration to support Stream Deck actions.
- Added an Ikkinz Group key that invites ready boxes from the active client and sends `Ctrl+I` to accept after one second
- Added an Ikkinz Follow key that concurrently sends `/follow <leader>` to ready background clients
- Added an Ikkinz Assist key that concurrently sends `/assist <main>` to ready background clients
- Simplified Ikkinz tiles with 15px minimum text, no top rails, a stable Bcast label, distinct action colors, filled running/armed states, and four reserved blank cells
- Replaced the Ikkinz MITE key with a two-step Swap key for exchanging the active and selected characters' window numbers
- Allowed targeted input sequences to run concurrently across independent EQ clients
- Fixed targeted modifier chords reaching background clients as an unmodified key by sequencing modifier press and release around the primary key
- Removed anonymous usage telemetry
- Refresh character and server identity when the same EQ process changes characters or servers
- Fixed auto-login failing after a self-update by embedding the matching trusik input DLL in stonemite.exe
- Hardened release builds against missing or invalid embedded trusik DLLs and removed the redundant loose DLL from release packages
- Fixed rapid client switching corrupting active/PiP label state by guarding overlay swaps against reentrant Windows events and reconciling foreground state during polling
- Fixed swaps updating labels without reliably foregrounding and focusing the requested EQ window; state now commits only after Windows confirms the target owns foreground and keyboard focus
- Prevented child processes from inheriting LAN sockets, added graceful `--quit` shutdown, and updated `just quit` so port 19720 is released cleanly

## v0.4.1

- Require Windows 10 or later in installer
- Update README for v0.4.0 features

## v0.4.0

- Auto-login: automatically enter credentials on the EQ login screen with encrypted password storage and server selection
- Prevent overlay freeze when swapping to/from a zoning EQ window
- Fix DrawTextW crash with empty text

## v0.3.2

- Hide background EQ windows from Alt-Tab (active window stays visible, enabled by default)
- Add auto order windows setting to keep PiP thumbnails sorted by slot number
- Fix DPI scaling for DPI-unaware EQ windows
- Remove "(right-click)" hint from unknown character labels

## v0.3.1

- Automatic update check on launch with configurable interval
- Open settings window automatically on first launch
- Anchor active window label at top-right when PiP edge is left
- Fix overlay rendering at half size after monitor reconnect on high-DPI displays
- Fix number reassignment displacing a window to number 0
- Fix potential panic when system fails to allocate context menus
- Log hotkey registration failures for easier troubleshooting

## v0.3.0

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

## v0.2.0

- Free PiP placement: move and resize individual PiPs anywhere on screen
- Edit Layout mode with 8-directional resize and 16:9 aspect enforcement
- Snap-to-grid, snap-to-monitor-edges, and snap-to-other-PiPs (hold Shift to bypass)
- Per-PiP windows (each PiP is its own top-level window with a DWM thumbnail)
- Tray menu: Edit/Lock Layout toggle, Reset to auto layout
- Strip auto-layout preserved as default; free placement is opt-in via Edit Layout
- Custom positions and snap grid size saved to config

## v0.1.1

- Right-click context menu on active window label
- Per-monitor DPI scaling for multi-monitor setups
- Pastel color palette for character labels
- Anonymous opt-out usage telemetry (disable in config or during install)

## v0.1.0

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
