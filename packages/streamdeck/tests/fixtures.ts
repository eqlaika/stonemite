import type { TrusharState, XTargetState } from "../src/types/trushar";

export function emptyXTargetState(): XTargetState {
  return {
    supported: true,
    slots: [],
    consider_bound: false,
    consider_pending: false,
  };
}

export function stateFixture(
  overrides: Partial<TrusharState> = {},
): TrusharState {
  return {
    revision: 4,
    clients: [
      {
        id: "client-1",
        character: "Laika",
        server: "Xegony",
        class_code: "WAR",
        window_number: 1,
        active: true,
        activatable: true,
        input_ready: true,
        xtarget: {
          supported: true,
          slots: [
            { slot: 1, label: "Auto hater", bound: true },
            { slot: 2, label: "Laika", bound: true },
          ],
          selected_slot: 1,
          consider_bound: true,
          consider_pending: false,
        },
      },
    ],
    active_client_id: "client-1",
    broadcast: { available: true, enabled: false },
    mouse_clutch: { phase: "inactive", availability: "ready" },
    capabilities: {
      activate: true,
      swap_window_numbers: true,
      set_broadcast: true,
      set_mouse_clutch: true,
      send_text: true,
      send_keys: true,
      eq_actions: {
        use_center_screen: true,
        invite_follow: true,
        hotbars: 11,
        hotbar_buttons: 12,
        spell_gems: 14,
        keymap_actions: true,
      },
    },
    ...overrides,
  };
}

export function stateMessage(state = stateFixture()): string {
  return JSON.stringify({ type: "state", version: 1, state });
}
