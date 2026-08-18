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
        const status =
          result.result.status === "already_active"
            ? "Already active"
            : "Activated";
        this.#store.setFeedback(feedbackKey, {
          kind: "success",
          message: status,
        });
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
        this.#store.setFeedback(feedbackKey, {
          kind: "success",
          message: cell.enabled ? "Broadcast off" : "Broadcast on",
        });
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
      default:
        return error.message;
    }
  }
  return error instanceof Error ? error.message : "Command failed";
}

function isRecord(value: JsonValue): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
