import { describe, expect, it } from "vitest";
import {
  MAX_MESSAGE_BYTES,
  ProtocolError,
  parseServerMessage,
  parseState,
} from "../src/types/trushar";
import { stateFixture, stateMessage } from "./fixtures";

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
      expect(message.state.revision).toBe(4);
    }
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
        },
        {
          id: "client-1",
          window_number: 1,
          active: true,
          activatable: true,
          input_ready: false,
        },
      ],
    });
    const raw = JSON.parse(JSON.stringify(state)) as {
      clients: Array<Record<string, unknown>>;
      capabilities: Record<string, unknown>;
    };
    delete raw.clients[1]!.input_ready;
    const capabilities = raw.capabilities as Record<string, unknown>;
    delete capabilities.swap_window_numbers;
    delete capabilities.eq_actions;
    const parsed = parseState(raw);
    expect(parsed.capabilities.swap_window_numbers).toBe(false);
    expect(parsed.capabilities.eq_actions).toEqual({
      use_center_screen: false,
      invite_follow: false,
      hotbars: 0,
      hotbar_buttons: 0,
      spell_gems: 0,
    });
    expect(parsed.clients.map((client) => client.id)).toEqual([
      "client-1",
      "client-2",
    ]);
    expect(parsed.clients[0]?.input_ready).toBe(false);
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
