# Stonemite build & release tasks

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

zip_name := "stonemite-x86_64-pc-windows-msvc.zip"

# Default: list available recipes
default:
    @just --list

# Build debug
build:
    cargo build -p stonemite

# Build release
build-release:
    cargo build --release -p stonemite

# Get current version from Cargo.toml
version:
    @(Get-Content app/Cargo.toml | Select-String '^version = "(.+)"' | ForEach-Object { $_.Matches.Groups[1].Value } | Select-Object -First 1)

# Bump version in Cargo.toml (usage: just bump 0.2.0)
bump new_version:
    @$content = Get-Content app/Cargo.toml -Raw; $content = $content -replace '(?m)(?<=^\[package\]\r?\nname = "stonemite"\r?\n)version = ".*"', 'version = "{{new_version}}"'; Set-Content app/Cargo.toml $content -NoNewline
    @Write-Host "Version bumped to {{new_version}}"

# Build release and create zip for distribution
package: build-release
    @New-Item -ItemType Directory -Force -Path dist | Out-Null
    @Copy-Item target/release/stonemite.exe dist/
    @python -c "import zipfile; z=zipfile.ZipFile('dist/{{zip_name}}','w',zipfile.ZIP_STORED); z.write('dist/stonemite.exe','stonemite.exe'); z.close()"
    @Write-Host "`nPackage ready: dist/{{zip_name}}"

# Build Inno Setup installer (requires Inno Setup 6)
installer: build-release
    @$iscc = (Get-Command "ISCC.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source) ; if (-not $iscc) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }; $ver = (Get-Content app/Cargo.toml | Select-String '^version = "(.+)"').Matches.Groups[1].Value; & $iscc /DAppVersion="$ver" installer.iss

# Full release flow: bump version, build, package, installer (usage: just release 0.2.0)
release new_version: (bump new_version) build-release
    @New-Item -ItemType Directory -Force -Path dist | Out-Null
    @Copy-Item target/release/stonemite.exe dist/
    @python -c "import zipfile; z=zipfile.ZipFile('dist/{{zip_name}}','w',zipfile.ZIP_STORED); z.write('dist/stonemite.exe','stonemite.exe'); z.close()"
    @$iscc = (Get-Command "ISCC.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source) ; if (-not $iscc) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }; & $iscc /DAppVersion="{{new_version}}" installer.iss
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

# Quit a running instance
quit:
    @try { Stop-Process -Name stonemite -ErrorAction Stop } catch { }; exit 0

# Build debug, quitting any running instance first
run: quit build
    @Start-Process target/debug/stonemite.exe

# Clean build artifacts and dist
clean:
    cargo clean
    @Remove-Item -Recurse -Force dist -ErrorAction SilentlyContinue
