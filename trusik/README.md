# Trusik

A minimal `dinput8.dll` proxy for EverQuest character detection.

Named after the [Trusik](https://everquest.allakhazam.com/db/faction.html?faction=432), the followers of Trushar on Taelosia, exiled to the mountains by the Nihilites — a small, resilient group operating from within hostile territory.

## What it does

Trusik is a DLL proxy that sits in the EQ installation directory. When EQ starts, Windows loads our `dinput8.dll` instead of the system one. All DirectInput calls pass through to the real DLL untouched — no input interception, no process memory access, no changes to game behavior or rendering.

The one thing we add is a `CreateFileW` IAT hook to detect when EQ opens a log file (`eqlog_CharName_Server.txt`). We parse the character name and server from the filename and write them into a named shared memory region (`Local\Stonemite_{pid}`) that stonemite reads to map each EQ process to its character.

## How it works

1. **DllMain** — loads the real `dinput8.dll` from `C:\Windows\System32`, creates shared memory, installs IAT hook
2. **DirectInput8Create** — pure passthrough to the real function
3. **CreateFileW hook** — checks if the filename matches `eqlog_*_*.txt`, parses character/server, writes to shared memory
4. **Shared memory** — `Local\Stonemite_{pid}` with a simple `CharacterInfo` struct (magic, pid, character, server)

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

