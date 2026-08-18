# Ikkinz

Ikkinz is the internal codename for the **Stonemite · EQ boxing** 5×3 Stream Deck control surface. It shows the current six-client roster, activates an exact loaded EQ client, swaps the window numbers of the active and selected clients, forms a group, directs ready background clients to follow or assist the active character, reflects targeted-input readiness, and explicitly enables or disables Stonemite broadcasting.

The Group key invites every other named, input-ready client from the active box, waits one second, then sends `Ctrl+I` to those invited clients. Boxes without a detected character name or ready targeted-input channel are skipped.

The Follow key identifies the active named leader and concurrently sends `/follow <leader>` to every other input-ready client.

The Assist key replaces the noninteractive STONE tile. It identifies the active named main box and concurrently sends `/assist <main>` to every other input-ready client.

The Swap key replaces the old MITE ambient key. Press **Swap**, then press a character. Stonemite exchanges that character's window number with the currently active character's number without activating a different window. Press Swap again or select the current character to cancel.

V0 is intentionally the core deck. Burn, Camp, heals, spells, live EQ targets, buffs, and cooldowns are not implemented until their commands and failure policy are defined.

## Requirements

- Stonemite v0.5.0 or later on Windows 10+
- Stream Deck Desktop 7.4 or later on macOS 12+ or Windows 10+
- Node.js 24+ for development
- Stream Deck Mobile Pro for the 5×3 layout
- A trusted private network between Stream Deck Desktop and the Stonemite PC

Stonemite's LAN API uses authenticated `ws://`. Pairing proves which client may control Stonemite, but traffic is not encrypted. Do not use it on an untrusted network.

## Set up Stonemite

On the Windows PC:

1. Open **Stonemite settings > General > Integrations**.
2. Enable integrations.
3. Choose **Devices on my local network**.
4. Save and restart Stonemite.
5. Allow Stonemite on **Private networks** if Windows Firewall asks.

## Develop

From `ikkinz/`:

```sh
npm install
npm run format:check
npm run lint
npm run typecheck
npm test
npm run build
npm run validate
```

The asset step generates data URLs from `../app/assets/app.png` and `../app/assets/class_icons/`; the generated TypeScript file is intentionally ignored. Action tiles use vendored Lucide Animated geometry and never fetch icons at runtime; licenses ship in `co.laikasoft.ikkinz.sdPlugin/THIRD_PARTY_NOTICES.md`. The Stream Deck bundle is written to `co.laikasoft.ikkinz.sdPlugin/bin/`.

To link a development build after validation:

```sh
npx streamdeck dev
npx streamdeck link co.laikasoft.ikkinz.sdPlugin
npm run watch
```

Linking restarts or changes the user's Stream Deck installation, so it is a deliberate manual step rather than part of tests. The tracked manifest keeps Node inspection disabled so packaged credentials are not exposed to a debugger. For a local debugging session only, temporarily add `"Debug": "enabled"` under `Nodejs`, restart the linked plugin, and remove it before validation or packaging.

## Install the 5×3 dashboard

Ikkinz includes preset profiles for the 15-key Stream Deck and Stream Deck Mobile. Accept the profile installation when Stream Deck prompts after installing Ikkinz; the complete dashboard appears without placing each action manually. The installed profile remains editable.

Every placed action is the same. Ikkinz uses the key's zero-based row and column to render and handle that cell. Leave each action image at its default: a user-defined image takes precedence over Ikkinz's live rendering, and Ikkinz disables user titles for this action.

## Pair with Jaggedpine

1. In Stonemite, choose **Pair a device**. Stonemite shows an address such as `jaggedpine.local:19720` and a six-digit code for five minutes.
2. In Stream Deck Desktop, select any Ikkinz grid key to open its property inspector.
3. Enter the displayed address and six digits, then choose **Pair with Stonemite**.
4. Ikkinz exchanges the code once, stores only the address and long credential in Stream Deck's plugin-wide settings, and reconnects automatically afterward.

Use **Reconnect** after changing the address or recovering the network. Use **Forget this device** to remove the saved credential; pair again before control is restored.

If `jaggedpine.local` does not resolve from the Mac, use the private IPv4 address Stonemite is listening on, for example `192.168.1.50:19720`.

## Package

```sh
npm run pack
```

The command builds, validates with the current Elgato CLI schema, and writes an installable `.streamDeckPlugin` to `ikkinz/dist/`.

## What the deck can truthfully report

- **Activated** means Stonemite accepted an exact-client activation request and returned authoritative current state.
- **Window numbers swapped** means Stonemite exchanged the active and selected clients' stable numbers and returned authoritative current state; it does not change the active client.
- **Broadcast on/off** is Stonemite's pushed broadcast state, not a blind local toggle.
- **Input ready** means that client's compatible trusik input channel acknowledged readiness.
- Group, Follow, and Assist progress confirm exact-client input delivery only; they do not prove that EQ formed the group, moved a character, or changed a target.
- Ikkinz does not observe whether EQ accepted input or performed a resulting in-game action.

Pairing codes, bearer credentials, and raw command traffic are never written to Ikkinz logs.
