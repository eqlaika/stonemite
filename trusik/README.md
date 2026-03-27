# Trusik

A minimal `dinput8.dll` proxy for EverQuest character detection and key broadcasting.

Named after the Trusik, the followers of Trushar on Taelosia, exiled to the mountains by the Nihilites — a small, resilient group operating from within hostile territory.

## What it does

Trusik is a DLL proxy that sits in the EQ installation directory. When EQ starts, Windows loads our `dinput8.dll` instead of the system one. All DirectInput calls pass through to the real DLL untouched — no process memory access, no changes to game behavior or rendering.

It provides two features:

- **Character detection** — a `CreateFileW` IAT hook detects when EQ opens a log file (`eqlog_CharName_Server.txt`). The character name and server are parsed from the filename and written into a named shared memory region (`Local\Stonemite_{pid}`) that stonemite reads to map each EQ process to its character.
- **Key broadcasting** — reads a per-process shared memory region (`Local\DI8_{pid}`) written by stonemite's low-level keyboard hook. When keys are flagged in the region, trusik injects them as synthetic DirectInput key state into the EQ process, allowing background windows to receive keystrokes without focus.

## How it works

1. **DllMain** — loads the real `dinput8.dll` from `C:\Windows\System32`, creates shared memory regions, installs IAT hook
2. **DirectInput8Create** — pure passthrough to the real function
3. **CreateFileW hook** — checks if the filename matches `eqlog_*_*.txt`, parses character/server, writes to shared memory
4. **Shared memory (character)** — `Local\Stonemite_{pid}` with a simple `CharacterInfo` struct (magic, pid, character, server)
5. **Shared memory (keys)** — `Local\DI8_{pid}` read by trusik to inject synthetic key state into DirectInput's `GetDeviceState`

## Deployment

Stonemite manages the DLL lifecycle automatically:
- **Enable** "Character Detection" in settings → stonemite copies `dinput8.dll` to the EQ directory
- **Disable** → stonemite removes the DLL from the EQ directory
- Requires EQ restart to take effect

## Build

```
cargo build -p trusik          # debug
cargo build --release -p trusik # release → target/release/dinput8.dll
```

