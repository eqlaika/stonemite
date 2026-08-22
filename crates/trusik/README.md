# Trusik

A minimal `dinput8.dll` proxy for EverQuest character detection and key broadcasting.

Named after the Trusik, the followers of Trushar on Taelosia, exiled to the mountains by the Nihilites — a small, resilient group operating from within hostile territory.

## What it does

Trusik is a DLL proxy that sits in the EQ installation directory. When EQ starts, Windows loads our `dinput8.dll` instead of the system one. Calls are forwarded to the real system DLL, with narrow proxies around `IDirectInput8` and keyboard/mouse devices. Trusik does not inspect EQ process memory or alter rendering.

It provides two features:

- **Character detection** — a `CreateFileW` IAT hook detects when EQ opens a log file (`eqlog_CharName_Server.txt`). The character name and server are parsed from the filename and written into a named shared memory region (`Local\Stonemite_{pid}`) that stonemite reads to map each EQ process to its character. A changed log identity is republished when the same EQ process camps or changes servers.
- **Key broadcasting** — reads a per-process shared memory region (`Local\DI8_{pid}`) written by stonemite's low-level keyboard hook. When keys are flagged in the region, trusik injects them as synthetic DirectInput key state into the EQ process, allowing background windows to receive keystrokes without focus.

## How it works

1. **DllMain** — remains an empty loader-lock-safe stub
2. **DirectInput8Create** — lazily loads the real DLL from System32, initializes shared state/hooks outside the loader lock, and wraps only the exact DirectInput A/W interfaces implemented by the proxy
3. **CreateFileW hook** — checks if the filename matches `eqlog_*_*.txt`, parses character/server, writes to shared memory
4. **Shared memory (character)** — `Local\Stonemite_{pid}` with a `CharacterInfo` struct (magic, pid, character, server, generation); the generation prevents mixed-field reads while identity changes
5. **Shared memory (keys)** — versioned `Local\DI8_{pid}` state with separate controller and auto-type key buffers and heartbeats; trusik combines fresh owners in `GetDeviceState` and buffered `GetDeviceData`

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
