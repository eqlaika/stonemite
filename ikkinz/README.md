# Ikkinz

Ikkinz is the internal codename for the customizable **Stonemite · EQ boxing** Stream Deck controls. It shows the current six-client roster, activates an exact loaded EQ client, swaps the window numbers of the active and selected clients, forms a group, directs ready background clients to follow or assist the active character, sends configurable mapped hotkeys, reflects targeted-input readiness, and explicitly enables or disables Stonemite broadcasting. An editable 5×3 profile ships as the default layout.

The Group key invites every other named, input-ready client from the active box, waits one second, then invokes each invited client's configured **Invite/Follow** EQ action. Boxes without a detected character name or ready targeted-input channel are skipped.

The Follow key identifies the active named leader and concurrently sends `/follow <leader>` to every other input-ready client.

The Assist key replaces the noninteractive STONE tile. It identifies the active named main box and concurrently sends `/assist <main>` to every other input-ready client.

The Swap key replaces the old MITE ambient key. Press **Swap**, then press a character. Stonemite exchanges that character's window number with the currently active character's number without activating a different window. Press Swap again or select the current character to cancel.

The Bcast key explicitly toggles Stonemite broadcasting. Its label stays **Bcast** in both states; a solid red surface means enabled. Group is blue, Follow green, Assist gold, Use orchid, and Swap cyan. Each uses a dark surface while idle and fills with its action color only while running or armed.

The Use key invokes each input-ready client's configured **Use Center Screen** EQ action, including the active client. The Stonemite setup key shows connection recovery state and owns the plugin-wide connection inspector; pressing it does not send a command. The default profile places it at bottom left and leaves two cells blank.

V0 remains an honest control surface rather than a combat-state monitor. Configurable Hotkey tiles can invoke any EQ action whose effective key mapping Stonemite can resolve, including all 11×12 hotbar actions and all 14 spell gems, but Ikkinz does not invent fixed Burn, Camp, heal, target, buff, cooldown, or outcome policies.

## Requirements

- Stonemite v0.5.0 or later on Windows 10+
- Stream Deck Desktop 7.4 or later on macOS 12+ or Windows 10+
- Node.js 24+ for development
- Stream Deck Mobile Pro for the included 5×3 mobile profile
- A trusted private network only when Stream Deck Desktop and Stonemite run on different PCs

Stonemite's LAN API uses authenticated `ws://`. Pairing proves which client may control Stonemite, but traffic is not encrypted. Do not use LAN access on an untrusted network.

## Connect on this PC

When Stream Deck Desktop and Stonemite run on the same Windows PC, there is nothing to configure. Stonemite enables loopback integrations by default, and Ikkinz automatically connects to `127.0.0.1:19720` without authentication.

If the Stonemite setup key keeps showing **Connecting**, open **Stonemite settings > General > Integrations**, enable integrations, choose **This PC only**, then save and restart Stonemite.

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

The asset step generates data URLs from `../crates/stonemite/assets/app.png` and `../crates/stonemite/assets/class_icons/`; the generated TypeScript file is intentionally ignored. Action tiles use vendored Lucide Animated geometry and never fetch icons at runtime; licenses ship in `co.laikasoft.ikkinz.sdPlugin/THIRD_PARTY_NOTICES.md`. The complete configurable-icon catalog is pinned to the commit recorded in `src/render/lucide-animated.generated.ts`. Maintainers update it deliberately with `npx playwright install chromium` followed by `npm run icons`; ordinary builds consume the checked-in frames. The Stream Deck bundle is written to `co.laikasoft.ikkinz.sdPlugin/bin/`.

To link a development build after validation:

```sh
npx streamdeck dev
npx streamdeck link co.laikasoft.ikkinz.sdPlugin
npm run watch
```

Linking restarts or changes the user's Stream Deck installation, so it is a deliberate manual step rather than part of tests. The tracked manifest keeps Node inspection disabled so packaged credentials are not exposed to a debugger. For a local debugging session only, temporarily add `"Debug": "enabled"` under `Nodejs`, restart the linked plugin, and remove it before validation or packaging.

## Install the default dashboard

Ikkinz includes preset 5×3 profiles for the 15-key Stream Deck and Stream Deck Mobile. Accept the profile installation when Stream Deck prompts after installing Ikkinz; the complete dashboard appears without placing each action manually. The installed profile remains editable.

Character slots 1–6, Group, Broadcast, Follow, Use, Assist, Swap, Hotkey, and Stonemite setup are separate actions in the Stream Deck action list. Drag any of them to any key position to create your own layout; behavior follows the action rather than its row and column. Duplicate actions are supported. Leave each action image at its default because a user-defined image takes precedence over Ikkinz's live rendering. Ikkinz disables user titles for these actions.

## Configure a mapped hotkey

Drag **Hotkey** from the Stonemite action list onto any key, then use its property inspector:

1. Choose **All loaded boxes** or **Selected boxes**. Selected targets are stable Stonemite window numbers 1–6, so they keep following the same box when its character or persona changes.
2. Choose an EQ action from the shared mapping list. Only actions mapped on every selected loaded box appear. If the action you want is missing, assign its key mapping in EverQuest for each intended box and choose **Refresh mappings**.
3. Enter an optional tile name of up to 14 characters.
4. Choose a Stonemite palette color or use the custom color picker. The idle icon and running background use this color; running text automatically switches between white and dark for contrast.
5. Search the complete pinned Lucide Animated catalog and select an icon.
6. Choose **Save tile**.

All-loaded targeting resolves the current loaded roster when the tile is pressed. Before exposing any key, Stonemite requires every requested box to be present, input-ready, free of another targeted sequence, and mapped for the selected action. A predictable failure therefore sends nothing. A process that disappears after delivery starts can still produce an honest failure because input already observed by another EQ client cannot be rolled back.

## Connect over LAN

LAN pairing is needed only when Stream Deck Desktop and Stonemite run on different PCs.

1. On the Stonemite PC, open **Stonemite settings > General > Integrations**, choose **Devices on my local network**, then save and restart.
2. Allow Stonemite on **Private networks** if Windows Firewall asks.
3. In Stonemite, choose **Pair a device**. Stonemite shows an address such as `stonemite-pc.local:19720` and a six-digit code for five minutes.
4. In Stream Deck Desktop, select the **Stonemite setup** key to open its property inspector, then expand **Connect to another PC**.
5. Enter the displayed address and six digits, then choose **Pair over LAN**.

Ikkinz exchanges the code once, stores only the address and long credential in Stream Deck's plugin-wide settings, and reconnects automatically afterward. The setup inspector collapses the form to a compact LAN status after pairing. Use **Reconnect** to recover the link, **Pair another PC** to replace it, or **Use this PC** to remove the saved LAN credential and resume automatic loopback access.

If the `.local` address does not resolve from the Stream Deck computer, use the private IPv4 address Stonemite is listening on, for example `192.168.1.50:19720`.

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
- Group, Follow, Assist, Use, and Hotkey progress confirm exact-client input delivery only; they do not prove that EQ formed the group, moved a character, changed a target, used the centered object, or performed the mapped action.
- Ikkinz does not observe whether EQ accepted input or performed a resulting in-game action.

Pairing codes, bearer credentials, and raw command traffic are never written to Ikkinz logs.
