import { describe, expect, it } from "vitest";
import {
  MAX_MESSAGE_BYTES,
  ProtocolError,
  parseServerMessage,
  parseState,
} from "../src/types/trushar";
import { emptyXTargetState, stateFixture, stateMessage } from "./fixtures";

describe("trushar parser", () => {
  it("accepts complete state and ignores additive fields", () => {
    const raw = JSON.parse(stateMessage()) as Record<string, unknown>;
    raw.future = { additive: true };
    const state = raw.state as Record<string, unknown>;
    state.future_state = "ignored";
    const clients = state.clients as Array<Record<string, unknown>>;
    clients[0]!.future_client = 42;

    const message = parseServerMessage(JSON.stringify(raw));
    expect(message.type).toBe("state");
    if (message.type === "state") {
      expect(message.state.clients[0]?.character).toBe("Laika");
      expect(message.state.clients[0]?.xtarget).toMatchObject({
        selected_slot: 1,
        consider_bound: true,
        slots: [
          { slot: 1, label: "Auto hater", bound: true },
          { slot: 2, label: "Laika", bound: true },
        ],
      });
      expect(message.state.revision).toBe(4);
    }
  });

  it("parses immediate no-target Consider feedback", () => {
    const raw = JSON.parse(JSON.stringify(stateFixture())) as {
      clients: Array<{ xtarget: Record<string, unknown> }>;
    };
    raw.clients[0]!.xtarget.consider = { no_target: true };

    expect(parseState(raw).clients[0]?.xtarget.consider).toEqual({
      type: "no_target",
    });
  });

  it("defaults older clients to input not ready and sorts by window number", () => {
    const state = stateFixture({
      clients: [
        {
          id: "client-2",
          window_number: 2,
          active: false,
          activatable: true,
          input_ready: true,
          xtarget: emptyXTargetState(),
        },
        {
          id: "client-1",
          window_number: 1,
          active: true,
          activatable: true,
          input_ready: false,
          xtarget: emptyXTargetState(),
        },
      ],
    });
    const raw = JSON.parse(JSON.stringify(state)) as {
      clients: Array<Record<string, unknown>>;
      capabilities: Record<string, unknown>;
    };
    delete raw.clients[1]!.input_ready;
    delete raw.clients[1]!.xtarget;
    const capabilities = raw.capabilities as Record<string, unknown>;
    delete capabilities.swap_window_numbers;
    delete capabilities.set_mouse_clutch;
    delete capabilities.eq_actions;
    delete (raw as Record<string, unknown>).mouse_clutch;
    const parsed = parseState(raw);
    expect(parsed.capabilities.swap_window_numbers).toBe(false);
    expect(parsed.capabilities.set_mouse_clutch).toBe(false);
    expect(parsed.mouse_clutch).toEqual({
      phase: "inactive",
      availability: "unsupported",
    });
    expect(parsed.capabilities.eq_actions).toEqual({
      use_center_screen: false,
      invite_follow: false,
      hotbars: 0,
      hotbar_buttons: 0,
      spell_gems: 0,
      keymap_actions: false,
    });
    expect(parsed.clients.map((client) => client.id)).toEqual([
      "client-1",
      "client-2",
    ]);
    expect(parsed.clients[0]?.input_ready).toBe(false);
    expect(parsed.clients[0]?.xtarget).toEqual({
      supported: false,
      slots: [],
      consider_bound: false,
      consider_pending: false,
    });
  });

  it("parses result, error, and pairing messages", () => {
    const state = stateFixture();
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "result",
          version: 1,
          request_id: "x",
          result: {
            type: "activated",
            status: "already_active",
            foreground_confirmed: true,
            future: true,
          },
          state,
        }),
      ).type,
    ).toBe("result");
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "result",
          version: 1,
          request_id: "swap",
          result: {
            type: "window_numbers_swapped",
            active_previous_number: 1,
            selected_previous_number: 3,
          },
          state,
        }),
      ),
    ).toMatchObject({
      type: "result",
      result: { type: "window_numbers_swapped" },
    });
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "result",
          version: 1,
          request_id: "clutch",
          result: {
            type: "mouse_clutch_hold_updated",
            held: true,
          },
          state,
        }),
      ),
    ).toMatchObject({
      type: "result",
      result: { type: "mouse_clutch_hold_updated", held: true },
    });
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "result",
          version: 1,
          request_id: "action",
          result: {
            type: "eq_action_delivered",
            action: { type: "hotbar", bar: 11, button: 12 },
          },
          state,
        }),
      ),
    ).toMatchObject({
      type: "result",
      result: {
        type: "eq_action_delivered",
        action: { type: "hotbar", bar: 11, button: 12 },
      },
    });
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "result",
          version: 1,
          request_id: "mapped",
          result: {
            type: "eq_keymap_actions_listed",
            mappings: ["DUCK", "SIT_STAND"],
            window_numbers: [1, 2],
            next_after: "SIT_STAND",
          },
          state,
        }),
      ),
    ).toMatchObject({
      type: "result",
      result: {
        type: "eq_keymap_actions_listed",
        mappings: ["DUCK", "SIT_STAND"],
      },
    });
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "result",
          version: 1,
          request_id: "batch",
          result: {
            type: "eq_action_batch_delivered",
            action: { type: "keymap", mapping: "DUCK" },
            window_numbers: [1, 2],
          },
          state,
        }),
      ),
    ).toMatchObject({
      type: "result",
      result: {
        type: "eq_action_batch_delivered",
        action: { type: "keymap", mapping: "DUCK" },
      },
    });
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "result",
          version: 1,
          request_id: "all-seven",
          result: {
            type: "eq_action_batch_delivered",
            action: { type: "keymap", mapping: "DUCK" },
            window_numbers: [1, 2, 3, 4, 5, 6, 7],
          },
          state,
        }),
      ),
    ).toMatchObject({
      type: "result",
      result: { window_numbers: [1, 2, 3, 4, 5, 6, 7] },
    });
    expect(
      parseServerMessage(
        JSON.stringify({
          type: "error",
          version: 1,
          request_id: "x",
          error: { code: "client_not_found", message: "gone" },
        }),
      ),
    ).toMatchObject({
      type: "error",
      request_id: "x",
      error: { code: "client_not_found" },
    });
    expect(
      parseServerMessage(
        JSON.stringify({ type: "paired", version: 1, auth_token: "secret" }),
      ),
    ).toMatchObject({
      type: "paired",
      auth_token: "secret",
    });
  });

  it.each([
    ["invalid JSON", "{"],
    ["unknown type", JSON.stringify({ type: "future", version: 1 })],
    [
      "wrong version",
      JSON.stringify({ type: "state", version: 2, state: stateFixture() }),
    ],
    [
      "missing required state",
      JSON.stringify({ type: "state", version: 1, state: {} }),
    ],
    [
      "malformed activation result",
      JSON.stringify({
        type: "result",
        version: 1,
        request_id: "x",
        result: { type: "activated", status: "surprised" },
        state: stateFixture(),
      }),
    ],
    [
      "malformed window-number swap result",
      JSON.stringify({
        type: "result",
        version: 1,
        request_id: "x",
        result: {
          type: "window_numbers_swapped",
          active_previous_number: 0,
          selected_previous_number: 2,
        },
        state: stateFixture(),
      }),
    ],
    [
      "malformed broadcast result",
      JSON.stringify({
        type: "result",
        version: 1,
        request_id: "x",
        result: { type: "broadcast_set", enabled: "yes" },
        state: stateFixture(),
      }),
    ],
    [
      "malformed Mouse Clutch result",
      JSON.stringify({
        type: "result",
        version: 1,
        request_id: "x",
        result: { type: "mouse_clutch_hold_updated", held: "yes" },
        state: stateFixture(),
      }),
    ],
    [
      "malformed Mouse Clutch state",
      JSON.stringify({
        type: "state",
        version: 1,
        state: stateFixture({
          mouse_clutch: {
            phase: "stuck" as "inactive",
            availability: "ready",
          },
        }),
      }),
    ],
    [
      "malformed EQ action result",
      JSON.stringify({
        type: "result",
        version: 1,
        request_id: "x",
        result: {
          type: "eq_action_delivered",
          action: { type: "spell_gem", gem: 15 },
        },
        state: stateFixture(),
      }),
    ],
    [
      "malformed mapped-action list",
      JSON.stringify({
        type: "result",
        version: 1,
        request_id: "x",
        result: {
          type: "eq_keymap_actions_listed",
          mappings: ["../DUCK"],
          window_numbers: [1],
        },
        state: stateFixture(),
      }),
    ],
    [
      "malformed EQ action capabilities",
      JSON.stringify({
        type: "state",
        version: 1,
        state: stateFixture({
          capabilities: {
            ...stateFixture().capabilities,
            eq_actions: {
              use_center_screen: true,
              invite_follow: true,
              hotbars: -1,
              hotbar_buttons: 12,
              spell_gems: 14,
              keymap_actions: true,
            },
          },
        }),
      }),
    ],
    [
      "invalid optional identity",
      JSON.stringify({
        type: "state",
        version: 1,
        state: stateFixture({
          clients: [
            {
              id: "x",
              window_number: 1,
              active: true,
              activatable: true,
              input_ready: true,
              xtarget: emptyXTargetState(),
              character: 7 as unknown as string,
            },
          ],
        }),
      }),
    ],
  ])("rejects %s", (_name, input) => {
    expect(() => parseServerMessage(input)).toThrow(ProtocolError);
  });

  it("rejects oversized frames before parsing", () => {
    expect(() => parseServerMessage("x".repeat(MAX_MESSAGE_BYTES + 1))).toThrow(
      "16384-byte",
    );
  });
});
