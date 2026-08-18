import streamDeck, {
  type KeyAction,
  type KeyDownEvent,
  type SendToPluginEvent,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import type { JsonObject, JsonValue } from "@elgato/utils";
import {
  buildCell,
  buildGroupPlan,
  cellKey,
  GRID_COLUMNS,
  GRID_ROWS,
  unsupportedCell,
} from "../state/layout";
import { DashboardStore } from "../state/store";
import { renderCell } from "../render/key-svg";
import {
  CommandError,
  normalizeAddress,
  TrusharClient,
  type Credentials,
} from "../trushar/client";

const GROUP_ACCEPT_DELAY_MS = 1_000;
const GROUP_FEEDBACK_TIMEOUT_MS = 60_000;

export interface PluginSettings extends JsonObject {
  address?: string;
  authToken?: string;
}

export interface VisibleKey {
  action: KeyAction;
  row: number;
  column: number;
  lastImage?: string;
}

export class GridKeyController {
  readonly #store: DashboardStore;
  readonly #client: TrusharClient;
  readonly #keys = new Map<string, VisibleKey>();
  #renderQueued = false;
  #bootStarted = false;
  #credentialEpoch = 0;
  #groupInFlight = false;

  constructor(store: DashboardStore, client: TrusharClient) {
    this.#store = store;
    this.#client = client;
    this.#store.subscribe(() => this.#queueRender());
  }

  async onWillAppear(event: WillAppearEvent): Promise<void> {
    if (!event.action.isKey() || event.action.isInMultiAction()) return;
    const coordinates = event.action.coordinates;
    if (
      !coordinates ||
      coordinates.row < 0 ||
      coordinates.column < 0 ||
      coordinates.row >= GRID_ROWS ||
      coordinates.column >= GRID_COLUMNS
    ) {
      await event.action.setImage(
        renderCell(
          unsupportedCell(coordinates?.row ?? -1, coordinates?.column ?? -1),
        ),
      );
      return;
    }
    this.#keys.set(event.action.id, {
      action: event.action,
      row: coordinates.row,
      column: coordinates.column,
    });
    this.#startBoot();
    this.#queueRender();
  }

  onWillDisappear(event: WillDisappearEvent): void {
    this.#keys.delete(event.action.id);
  }

  async onKeyDown(event: KeyDownEvent): Promise<void> {
    if (!event.action.isKey()) return;
    const key = this.#keys.get(event.action.id);
    if (!key) return;
    const cell = buildCell(this.#store.view, key.row, key.column);

    try {
      if (cell.type === "character" && cell.enabled) {
        const feedbackKey = cellKey(key.row, key.column);
        this.#store.setFeedback(
          feedbackKey,
          { kind: "pending", message: "Activating" },
          10_000,
        );
        const result = await this.#client.activate(cell.client.id);
        if (result.result.type !== "activated")
          throw new CommandError(
            "protocol_error",
            "Stonemite returned the wrong activation result.",
          );
        this.#store.clearFeedback(feedbackKey);
        return;
      }
      if (cell.type === "group" && cell.available && !this.#groupInFlight) {
        const feedbackKey = cellKey(key.row, key.column);
        this.#groupInFlight = true;
        this.#setGroupFeedback(feedbackKey, "Inviting");
        try {
          await this.#formGroup(feedbackKey);
          this.#store.clearFeedback(feedbackKey);
        } finally {
          this.#groupInFlight = false;
        }
        return;
      }
      if (cell.type === "broadcast" && cell.available) {
        const feedbackKey = cellKey(key.row, key.column);
        this.#store.setFeedback(
          feedbackKey,
          { kind: "pending", message: "Updating" },
          10_000,
        );
        await this.#client.setBroadcast(!cell.enabled);
        this.#store.clearFeedback(feedbackKey);
      }
    } catch (error) {
      this.#store.setFeedback(cellKey(key.row, key.column), {
        kind: "error",
        message: friendlyError(error),
      });
      await event.action.showAlert();
    }
  }

  async onPropertyInspectorDidAppear(): Promise<void> {
    await this.#sendStatus();
  }

  async onSendToPlugin(
    event: SendToPluginEvent<JsonValue, JsonObject>,
  ): Promise<void> {
    const payload = event.payload;
    if (!isRecord(payload) || typeof payload.type !== "string") return;

    if (payload.type === "get-status") {
      await this.#sendStatus();
      return;
    }

    if (payload.type === "forget") {
      const epoch = ++this.#credentialEpoch;
      this.#client.disconnect();
      this.#store.clear();
      await streamDeck.settings.setGlobalSettings<PluginSettings>({});
      if (epoch === this.#credentialEpoch) await this.#sendStatus();
      return;
    }

    if (payload.type === "reconnect") {
      const epoch = ++this.#credentialEpoch;
      const saved =
        await streamDeck.settings.getGlobalSettings<PluginSettings>();
      if (epoch !== this.#credentialEpoch) return;
      if (!saved.address || !saved.authToken) {
        this.#store.setConnection({
          state: "idle",
          title: "Not paired",
          detail: "Enter a six-digit pairing code first.",
        });
        await this.#sendStatus();
        return;
      }
      try {
        const credentials = credentialsForReconnect(saved, payload.address);
        if (epoch !== this.#credentialEpoch) return;
        this.#client.configure(credentials);
      } catch (error) {
        this.#store.setConnection({
          state: "error",
          title: "Pair again for this address",
          detail: friendlyError(error),
        });
      }
      if (epoch === this.#credentialEpoch) await this.#sendStatus();
      return;
    }

    if (payload.type === "pair") {
      if (
        typeof payload.address !== "string" ||
        typeof payload.code !== "string"
      )
        return;
      const epoch = ++this.#credentialEpoch;
      try {
        const credentials = await this.#client.pair(
          payload.address,
          payload.code,
        );
        if (epoch !== this.#credentialEpoch) return;
        await streamDeck.settings.setGlobalSettings<PluginSettings>({
          address: credentials.address,
          authToken: credentials.authToken,
        });
        if (epoch !== this.#credentialEpoch) return;
        this.#client.configure(credentials);
      } catch (error) {
        if (epoch !== this.#credentialEpoch) return;
        this.#store.setConnection({
          state: "error",
          title: "Pairing failed",
          detail: friendlyError(error),
        });
      }
      if (epoch === this.#credentialEpoch) await this.#sendStatus();
    }
  }

  async #formGroup(feedbackKey: string): Promise<void> {
    const plan = buildGroupPlan(this.#store.view);
    if (!plan.available || !plan.active) {
      throw new CommandError(
        "group_unavailable",
        "No active box and ready invitees are available.",
      );
    }

    const invited: Array<(typeof plan.invitees)[number]> = [];
    const failures: Error[] = [];
    for (const invitee of plan.invitees) {
      try {
        await this.#client.sendText(
          plan.active.id,
          `/invite ${invitee.character.trim()}`,
          true,
        );
        invited.push(invitee);
      } catch (error) {
        failures.push(asError(error));
      }
    }

    if (invited.length === 0) {
      throw (
        failures[0] ??
        new CommandError("group_unavailable", "No invites were delivered.")
      );
    }

    this.#setGroupFeedback(feedbackKey, "Waiting 1 sec");
    await wait(GROUP_ACCEPT_DELAY_MS);
    this.#setGroupFeedback(feedbackKey, "Accepting");

    for (const invitee of invited) {
      try {
        await this.#client.sendKeys(invitee.id, [
          {
            keys: ["left_control", "i"],
            hold_ms: 50,
            pause_ms: 40,
          },
        ]);
      } catch (error) {
        failures.push(asError(error));
      }
    }

    if (failures.length > 0) {
      throw new CommandError(
        "group_partial",
        "Some ready boxes missed the group sequence.",
      );
    }
  }

  #setGroupFeedback(key: string, message: string): void {
    this.#store.setFeedback(
      key,
      { kind: "pending", message },
      GROUP_FEEDBACK_TIMEOUT_MS,
    );
  }

  async #sendStatus(): Promise<void> {
    const status = this.#store.view.connection;
    await streamDeck.ui.sendToPropertyInspector({
      type: "connection-status",
      status: {
        state: status.state,
        title: status.title,
        detail: status.detail,
      },
    });
  }

  #startBoot(): void {
    if (this.#bootStarted) return;
    this.#bootStarted = true;
    this.#store.setBootStage(0);
    for (const [stage, delay] of [
      [1, 160],
      [2, 620],
      [3, 1_050],
    ] as const) {
      const timer = setTimeout(() => this.#store.setBootStage(stage), delay);
      timer.unref?.();
    }
  }

  #queueRender(): void {
    if (this.#renderQueued) return;
    this.#renderQueued = true;
    queueMicrotask(() => {
      this.#renderQueued = false;
      void this.#renderAll();
    });
  }

  async #renderAll(): Promise<void> {
    const updates: Promise<void>[] = [];
    for (const key of this.#keys.values()) {
      const image = renderCell(
        buildCell(this.#store.view, key.row, key.column),
      );
      if (image === key.lastImage) continue;
      updates.push(updateVisibleKeyImage(key, image));
    }
    await Promise.allSettled(updates);
  }
}

export function credentialsForReconnect(
  saved: PluginSettings,
  requestedAddress: JsonValue | undefined,
): Credentials {
  if (!saved.address || !saved.authToken)
    throw new CommandError("not_paired", "Pair this device first.");
  const savedAddress = normalizeAddress(saved.address);
  const requested =
    typeof requestedAddress === "string"
      ? normalizeAddress(requestedAddress)
      : savedAddress;
  if (requested !== savedAddress)
    throw new CommandError(
      "address_changed",
      "The address changed. Use a new six-digit code so the saved credential is never sent to another host.",
    );
  return { address: savedAddress, authToken: saved.authToken };
}

export async function updateVisibleKeyImage(
  key: VisibleKey,
  image: string,
): Promise<void> {
  await key.action.setImage(image);
  key.lastImage = image;
}

function friendlyError(error: unknown): string {
  if (error instanceof CommandError) {
    switch (error.code) {
      case "target_disappeared":
        return "Client left";
      case "activation_failed":
        return "Activation failed";
      case "broadcast_unavailable":
        return "Broadcast unavailable";
      case "command_timeout":
        return "Timed out";
      case "input_unavailable":
        return "Input not ready";
      case "group_unavailable":
        return "No ready boxes";
      case "group_partial":
        return "Partial send";
      default:
        return error.message;
    }
  }
  return error instanceof Error ? error.message : "Command failed";
}

function wait(delayMs: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, delayMs);
    timer.unref?.();
  });
}

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error("Command failed");
}

function isRecord(value: JsonValue): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
