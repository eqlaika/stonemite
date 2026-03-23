# Changelog

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
