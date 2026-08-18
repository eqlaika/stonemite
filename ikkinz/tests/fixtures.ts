import type { TrusharState } from "../src/types/trushar";

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
      },
    ],
    active_client_id: "client-1",
    broadcast: { available: true, enabled: false },
    capabilities: {
      activate: true,
      swap_window_numbers: true,
      set_broadcast: true,
      send_text: true,
      send_keys: true,
    },
    ...overrides,
  };
}

export function stateMessage(state = stateFixture()): string {
  return JSON.stringify({ type: "state", version: 1, state });
}
