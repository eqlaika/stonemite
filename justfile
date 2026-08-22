# Stonemite build & release tasks

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

zip_name := "stonemite-x86_64-pc-windows-msvc.zip"

# Default: list available recipes
default:
    @just --list

# Install frontend dependencies when npm is available; native Windows SSH
# builds may instead consume settings-ui/dist built on the development host.
settings-ui-deps:
    @$npm = Get-Command npm -ErrorAction SilentlyContinue; if ($npm) { if (-not (Test-Path "settings-ui/node_modules")) { npm --prefix settings-ui ci } } elseif (-not (Test-Path "settings-ui/dist/index.html")) { throw "npm is unavailable and settings-ui/dist is not prebuilt" }

# Build the embedded frontend, or verify the locally prebuilt mirror on Windows.
settings-ui-build: settings-ui-deps
    @$npm = Get-Command npm -ErrorAction SilentlyContinue; if ($npm) { npm --prefix settings-ui run build } elseif (-not (Test-Path "settings-ui/dist/index.html")) { throw "settings-ui/dist is not prebuilt" } else { Write-Host "Using prebuilt settings-ui/dist (npm is unavailable)." }

settings-ui-test:
    @if (-not (Get-Command npm -ErrorAction SilentlyContinue)) { throw "npm is required for settings UI tests" }
    npm --prefix settings-ui run typecheck
    npm --prefix settings-ui test

# Verify canonical and internal version metadata agree
version-check:
    python scripts/version.py check

# Build debug
build: version-check settings-ui-build
    cargo build -p trusik
    cargo build -p stonemite

# Build release
build-release: version-check settings-ui-build
    cargo build --release -p trusik
    cargo build --release -p stonemite

# Get the canonical YYYY.MM.DD[.N] public version
version:
    @python scripts/version.py get

# Set the canonical version and synchronize Cargo/Tauri metadata
bump new_version:
    @python scripts/version.py set "{{new_version}}"

# Build release and create zip for distribution
package: build-release
    @New-Item -ItemType Directory -Force -Path dist | Out-Null
    @Copy-Item target/release/stonemite.exe dist/
    @Copy-Item THIRD_PARTY_NOTICES.md dist/
    @python -c "import zipfile; z=zipfile.ZipFile('dist/{{zip_name}}','w',zipfile.ZIP_STORED); z.write('dist/stonemite.exe','stonemite.exe'); z.write('dist/THIRD_PARTY_NOTICES.md','THIRD_PARTY_NOTICES.md'); z.close()"
    @Write-Host "`nPackage ready: dist/{{zip_name}}"

# Build Inno Setup installer (requires Inno Setup 6)
installer: build-release
    @$iscc = (Get-Command "ISCC.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source); if (-not $iscc) { $iscc = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" }; if (-not (Test-Path $iscc)) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }; $ver = python scripts/version.py get; & $iscc /DAppVersion="$ver" installer.iss

# Full release flow: bump version, build, package, installer (usage: just release 2026.08.22)
release new_version: (bump new_version) build-release
    @New-Item -ItemType Directory -Force -Path dist | Out-Null
    @Copy-Item target/release/stonemite.exe dist/
    @Copy-Item THIRD_PARTY_NOTICES.md dist/
    @python -c "import zipfile; z=zipfile.ZipFile('dist/{{zip_name}}','w',zipfile.ZIP_STORED); z.write('dist/stonemite.exe','stonemite.exe'); z.write('dist/THIRD_PARTY_NOTICES.md','THIRD_PARTY_NOTICES.md'); z.close()"
    @$iscc = (Get-Command "ISCC.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source); if (-not $iscc) { $iscc = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" }; if (-not (Test-Path $iscc)) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }; & $iscc /DAppVersion="{{new_version}}" installer.iss
    @$notes = @(); $capture = $false; foreach ($line in (Get-Content CHANGELOG.md)) { if ($line -match '^## v{{new_version}}') { $capture = $true; continue } elseif ($capture -and $line -match '^## ') { break } elseif ($capture) { $notes += $line } }; ($notes -join "`n").Trim() | Set-Content dist/release-notes.md -NoNewline
    @Write-Host "`nRelease v{{new_version}} packaged:"
    @Write-Host "  dist/{{zip_name}}"
    @Write-Host "  dist/stonemite-{{new_version}}-setup.exe"
    @Write-Host "  dist/release-notes.md"
    @Write-Host "Next steps:"
    @Write-Host "  1. git add -A && git commit -m 'Release v{{new_version}}'"
    @Write-Host "  2. git tag v{{new_version}}"
    @Write-Host "  3. git push && git push --tags"
    @Write-Host "  4. gh release create v{{new_version}} dist/{{zip_name}} dist/stonemite-{{new_version}}-setup.exe --title 'v{{new_version}}' --notes-file dist/release-notes.md"

# Quit a running instance through its message loop so LAN sockets close cleanly
quit:
    @$running = Get-Process -Name stonemite -ErrorAction SilentlyContinue | Where-Object { $_.Path } | Select-Object -First 1; if (-not $running) { exit 0 }; $helper = Start-Process -FilePath $running.Path -ArgumentList "--quit" -Wait -PassThru; if ($helper.ExitCode -ne 0) { throw "Stonemite did not exit cleanly; refusing to force-terminate it" }

# Build debug, quitting any running instance first
run: quit build
    @Start-Process target/debug/stonemite.exe

# Mirror, build, and run the current working tree on the configured Windows host
deploy-dev:
    #!/usr/bin/env bash
    # This recipe intentionally runs on the local Unix development host.
    set -euo pipefail
    exec ./scripts/deploy-dev

# Clean build artifacts and dist
clean:
    cargo clean
    @Remove-Item -Recurse -Force dist -ErrorAction SilentlyContinue
    @Remove-Item -Recurse -Force settings-ui/dist -ErrorAction SilentlyContinue
