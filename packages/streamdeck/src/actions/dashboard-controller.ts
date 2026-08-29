import { randomUUID } from "node:crypto";
import streamDeck, {
  type DidReceiveSettingsEvent,
  type KeyAction,
  type KeyDownEvent,
  type KeyUpEvent,
  type SendToPluginEvent,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import type { JsonObject, JsonValue } from "@elgato/utils";
import { renderCell } from "../render/key-svg";
import { buildKey, buildSwapPlan, type DashboardKey } from "../state/layout";
import { DashboardStore } from "../state/store";
import {
  CommandError,
  LOCAL_CONNECTION,
  normalizeAddress,
  TrusharClient,
  type ConnectionConfig,
} from "../trushar/client";
import {
  characterKeyForWindow,
  normalizeWindowNumber,
  type BoxSettings,
} from "./box-settings";
import {
  CHARACTER_ACTION_DEFINITION,
  keyForManifestId,
} from "./key-definitions";

const SWAP_TILE_FRAME_MS = 125;
const SWAP_TILE_FRAME_COUNT = 8;
const MOUSE_CLUTCH_RENEW_MS = 500;

export interface PluginSettings extends JsonObject {
  address?: string;
  authToken?: string;
}

export interface VisibleKey {
  action: KeyAction;
  key: DashboardKey;
  lastImage?: string;
}

interface ClutchHold {
  holdId: string;
  accepted: boolean;
  renewTimer: ReturnType<typeof setTimeout> | null;
}

export class DashboardController {
  readonly #store: DashboardStore;
  readonly #client: TrusharClient;
  readonly #keys = new Map<string, VisibleKey>();
  #renderQueued = false;
  #rendering = false;
  #motionFrame = 0;
  #motionTimer: ReturnType<typeof setInterval> | null = null;
  #bootStarted = false;
  #credentialEpoch = 0;
  readonly #activationsInFlight = new Set<string>();
  readonly #clutchHolds = new Map<string, ClutchHold>();
  #broadcastInFlight = false;
  #swapArmed = false;
  #swapInFlight = false;

  constructor(store: DashboardStore, client: TrusharClient) {
    this.#store = store;
    this.#client = client;
    this.#store.subscribe((view) => {
      if (!buildSwapPlan(view).available) this.#swapArmed = false;
      if (view.connection.state !== "connected") this.#forgetClutchHolds();
      this.#queueRender();
    });
  }

  async onWillAppear(event: WillAppearEvent<BoxSettings>): Promise<void> {
    if (!event.action.isKey() || event.action.isInMultiAction()) return;
    const key =
      event.action.manifestId === CHARACTER_ACTION_DEFINITION.uuid
        ? characterKeyForWindow(
            normalizeWindowNumber(event.payload?.settings?.windowNumber),
          )
        : keyForManifestId(event.action.manifestId);
    if (!key) return;
    this.#keys.set(event.action.id, { action: event.action, key });
    this.#startBoot();
    this.#queueRender();
  }

  onDidReceiveSettings(event: DidReceiveSettingsEvent<BoxSettings>): void {
    if (
      !event.action.isKey() ||
      event.action.manifestId !== CHARACTER_ACTION_DEFINITION.uuid
    ) {
      return;
    }
    const visible = this.#keys.get(event.action.id);
    if (!visible) return;
    visible.key = characterKeyForWindow(
      normalizeWindowNumber(event.payload.settings.windowNumber),
    );
    delete visible.lastImage;
    this.#queueRender();
  }

  onWillDisappear(event: WillDisappearEvent): void {
    const key = this.#keys.get(event.action.id);
    if (key?.key === "mouse-clutch") {
      void this.#releaseClutch(key, false);
    }
    this.#keys.delete(event.action.id);
    this.#syncMotion();
  }

  async onKeyDown(event: KeyDownEvent): Promise<void> {
    if (!event.action.isKey()) return;
    const key = this.#keys.get(event.action.id);
    if (!key) return;
    const cell = buildKey(
      this.#store.view,
      key.key,
      key.action.id,
      this.#swapArmed,
    );

    try {
      if (cell.type === "mouse-clutch") {
        if (!cell.available || this.#clutchHolds.has(key.action.id)) return;
        void this.#beginClutch(key).catch(async (error: unknown) => {
          this.#store.setFeedback(key.action.id, {
            kind: "error",
            message: friendlyError(error),
          });
          await key.action.showAlert();
        });
        return;
      }
      if (this.#swapInFlight) return;
      if (cell.type === "swap" && cell.available) {
        this.#swapArmed = !this.#swapArmed;
        this.#queueRender();
        return;
      }
      if (this.#swapArmed) {
        if (cell.type !== "character" || !cell.enabled) return;
        this.#swapArmed = false;
        this.#queueRender();
        if (cell.client.active) return;

        const feedbackKey = key.action.id;
        this.#swapInFlight = true;
        this.#store.setFeedback(
          feedbackKey,
          { kind: "pending", message: "Swapping" },
          10_000,
        );
        try {
          const result = await this.#client.swapWindowNumbers(cell.client.id);
          if (result.result.type !== "window_numbers_swapped")
            throw new CommandError(
              "protocol_error",
              "Stonemite returned the wrong window-number swap result.",
            );
          this.#store.clearFeedback(feedbackKey);
        } finally {
          this.#swapInFlight = false;
          this.#queueRender();
        }
        return;
      }
      if (
        cell.type === "character" &&
        cell.enabled &&
        !this.#activationsInFlight.has(cell.client.id)
      ) {
        const feedbackKey = key.action.id;
        this.#activationsInFlight.add(cell.client.id);
        this.#store.setFeedback(
          feedbackKey,
          { kind: "pending", message: "Activating" },
          10_000,
        );
        try {
          const result = await this.#client.activate(cell.client.id);
          if (result.result.type !== "activated")
            throw new CommandError(
              "protocol_error",
              "Stonemite returned the wrong activation result.",
            );
          this.#store.clearFeedback(feedbackKey);
        } finally {
          this.#activationsInFlight.delete(cell.client.id);
        }
        return;
      }
      if (
        cell.type === "broadcast" &&
        cell.available &&
        !this.#broadcastInFlight
      ) {
        const feedbackKey = key.action.id;
        this.#broadcastInFlight = true;
        this.#store.setFeedback(
          feedbackKey,
          { kind: "pending", message: "Updating" },
          10_000,
        );
        try {
          await this.#client.setBroadcast(!cell.enabled);
          this.#store.clearFeedback(feedbackKey);
        } finally {
          this.#broadcastInFlight = false;
        }
      }
    } catch (error) {
      this.#store.setFeedback(key.action.id, {
        kind: "error",
        message: friendlyError(error),
      });
      await event.action.showAlert();
    }
  }

  async onKeyUp(event: KeyUpEvent): Promise<void> {
    if (!event.action.isKey()) return;
    const key = this.#keys.get(event.action.id);
    if (!key || key.key !== "mouse-clutch") return;
    await this.#releaseClutch(key, true);
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
      await streamDeck.settings.setGlobalSettings<PluginSettings>({});
      if (epoch !== this.#credentialEpoch) return;
      this.#client.configure(LOCAL_CONNECTION);
      this.#store.clear();
      await this.#sendStatus();
      return;
    }

    if (payload.type === "reconnect") {
      const epoch = ++this.#credentialEpoch;
      this.#client.reconnect();
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

  async #beginClutch(key: VisibleKey): Promise<void> {
    const hold: ClutchHold = {
      holdId: randomUUID(),
      accepted: false,
      renewTimer: null,
    };
    this.#clutchHolds.set(key.action.id, hold);
    this.#store.setFeedback(
      key.action.id,
      { kind: "pending", message: "Engaging" },
      10_000,
    );
    try {
      const result = await this.#client.beginMouseClutch(hold.holdId);
      if (this.#clutchHolds.get(key.action.id) !== hold) return;
      if (
        result.result.type !== "mouse_clutch_hold_updated" ||
        !result.result.held
      ) {
        throw new CommandError(
          "protocol_error",
          "Stonemite did not accept the Mouse Clutch hold.",
        );
      }
      hold.accepted = true;
      this.#store.clearFeedback(key.action.id);
      this.#scheduleClutchRenewal(key, hold);
    } catch (error) {
      if (this.#clutchHolds.get(key.action.id) !== hold) return;
      this.#clutchHolds.delete(key.action.id);
      throw error;
    }
  }

  async #releaseClutch(key: VisibleKey, showFailure: boolean): Promise<void> {
    const hold = this.#clutchHolds.get(key.action.id);
    if (!hold) return;
    this.#clutchHolds.delete(key.action.id);
    if (hold.renewTimer) clearTimeout(hold.renewTimer);
    this.#store.clearFeedback(key.action.id);
    try {
      const result = await this.#client.endMouseClutch(hold.holdId);
      if (
        result.result.type !== "mouse_clutch_hold_updated" ||
        result.result.held
      ) {
        throw new CommandError(
          "protocol_error",
          "Stonemite did not release the Mouse Clutch hold.",
        );
      }
    } catch (error) {
      if (
        error instanceof CommandError &&
        error.code === "mouse_clutch_hold_expired"
      ) {
        return;
      }
      if (showFailure && this.#store.view.connection.state === "connected") {
        this.#store.setFeedback(key.action.id, {
          kind: "error",
          message: friendlyError(error),
        });
        await key.action.showAlert();
      }
    }
  }

  #scheduleClutchRenewal(key: VisibleKey, hold: ClutchHold): void {
    if (
      !hold.accepted ||
      this.#clutchHolds.get(key.action.id) !== hold ||
      this.#store.view.connection.state !== "connected"
    ) {
      return;
    }
    hold.renewTimer = setTimeout(
      () => void this.#renewClutch(key, hold),
      MOUSE_CLUTCH_RENEW_MS,
    );
    hold.renewTimer.unref?.();
  }

  async #renewClutch(key: VisibleKey, hold: ClutchHold): Promise<void> {
    hold.renewTimer = null;
    if (this.#clutchHolds.get(key.action.id) !== hold) return;
    try {
      const result = await this.#client.renewMouseClutch(hold.holdId);
      if (
        result.result.type !== "mouse_clutch_hold_updated" ||
        !result.result.held
      ) {
        throw new CommandError(
          "mouse_clutch_hold_expired",
          "Mouse Clutch was canceled. Release and press the key again.",
        );
      }
    } catch (error) {
      if (this.#clutchHolds.get(key.action.id) !== hold) return;
      this.#clutchHolds.delete(key.action.id);
      this.#store.setFeedback(key.action.id, {
        kind: "error",
        message: friendlyError(error),
      });
      await key.action.showAlert();
      return;
    }
    this.#scheduleClutchRenewal(key, hold);
  }

  #forgetClutchHolds(): void {
    for (const hold of this.#clutchHolds.values()) {
      if (hold.renewTimer) clearTimeout(hold.renewTimer);
    }
    this.#clutchHolds.clear();
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
    this.#syncMotion();
    if (this.#renderQueued) return;
    this.#renderQueued = true;
    queueMicrotask(() => void this.#drainRenderQueue());
  }

  async #drainRenderQueue(): Promise<void> {
    if (this.#rendering) return;
    this.#rendering = true;
    try {
      while (this.#renderQueued) {
        this.#renderQueued = false;
        await this.#renderAll();
      }
    } finally {
      this.#rendering = false;
    }
  }

  #syncMotion(): void {
    const active = [...this.#keys.values()].some((key) => {
      const cell = buildKey(
        this.#store.view,
        key.key,
        key.action.id,
        this.#swapArmed,
      );
      return cell.type === "swap" && cell.armed;
    });

    if (active && !this.#motionTimer) {
      this.#motionFrame = 0;
      this.#motionTimer = setInterval(() => {
        this.#motionFrame = (this.#motionFrame + 1) % SWAP_TILE_FRAME_COUNT;
        this.#queueRender();
      }, SWAP_TILE_FRAME_MS);
      this.#motionTimer.unref?.();
      return;
    }

    if (!active && this.#motionTimer) {
      clearInterval(this.#motionTimer);
      this.#motionTimer = null;
      this.#motionFrame = 0;
    }
  }

  async #renderAll(): Promise<void> {
    const updates: Promise<void>[] = [];
    for (const key of this.#keys.values()) {
      const image = renderCell(
        buildKey(this.#store.view, key.key, key.action.id, this.#swapArmed),
        this.#motionFrame,
      );
      if (image === key.lastImage) continue;
      updates.push(updateVisibleKeyImage(key, image));
    }
    await Promise.allSettled(updates);
  }
}

export function connectionForSettings(saved: PluginSettings): ConnectionConfig {
  const address = saved.address?.trim() ?? "";
  const authToken = saved.authToken?.trim() ?? "";
  if (!address && !authToken) return { ...LOCAL_CONNECTION };
  if (!address || !authToken)
    throw new CommandError(
      "invalid_saved_connection",
      "The saved LAN connection is incomplete. Use this PC or pair over LAN again.",
    );
  return { address: normalizeAddress(address), authToken };
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
      case "window_number_swap_failed":
        return "Swap unavailable";
      case "broadcast_unavailable":
        return "Broadcast unavailable";
      case "mouse_clutch_unavailable":
        return "Clutch unavailable";
      case "mouse_clutch_not_ready":
        return "No compatible boxes";
      case "mouse_clutch_hold_expired":
        return "Press again";
      case "mouse_clutch_operation_failed":
        return "Clutch failed";
      case "command_timeout":
        return "Timed out";
      case "input_unavailable":
        return "Input not ready";
      default:
        return error.message;
    }
  }
  return error instanceof Error ? error.message : "Command failed";
}

function isRecord(value: JsonValue): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
