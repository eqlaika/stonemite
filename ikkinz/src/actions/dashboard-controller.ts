import streamDeck, {
  type KeyAction,
  type KeyDownEvent,
  type SendToPluginEvent,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import type { JsonObject, JsonValue } from "@elgato/utils";
import {
  buildAssistPlan,
  buildFollowPlan,
  buildGroupPlan,
  buildKey,
  buildSwapPlan,
  type DashboardKey,
} from "../state/layout";
import { DashboardStore } from "../state/store";
import { renderCell } from "../render/key-svg";
import {
  CommandError,
  normalizeAddress,
  TrusharClient,
  type Credentials,
} from "../trushar/client";
import { keyForManifestId } from "./key-definitions";

const GROUP_ACCEPT_DELAY_MS = 1_000;
const ACTION_FEEDBACK_TIMEOUT_MS = 60_000;
const ACTIVE_TILE_FRAME_MS = 125;
const ACTIVE_TILE_FRAME_COUNT = 8;

export interface PluginSettings extends JsonObject {
  address?: string;
  authToken?: string;
}

export interface VisibleKey {
  action: KeyAction;
  key: DashboardKey;
  lastImage?: string;
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
  #broadcastInFlight = false;
  #groupInFlight = false;
  #followInFlight = false;
  #assistInFlight = false;
  #swapArmed = false;
  #swapInFlight = false;

  constructor(store: DashboardStore, client: TrusharClient) {
    this.#store = store;
    this.#client = client;
    this.#store.subscribe((view) => {
      if (!buildSwapPlan(view).available) this.#swapArmed = false;
      this.#queueRender();
    });
  }

  async onWillAppear(event: WillAppearEvent): Promise<void> {
    if (!event.action.isKey() || event.action.isInMultiAction()) return;
    const key = keyForManifestId(event.action.manifestId);
    if (!key) return;
    this.#keys.set(event.action.id, { action: event.action, key });
    this.#startBoot();
    this.#queueRender();
  }

  onWillDisappear(event: WillDisappearEvent): void {
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
      if (cell.type === "group" && cell.available && !this.#groupInFlight) {
        const feedbackKey = key.action.id;
        this.#groupInFlight = true;
        this.#setActionFeedback(feedbackKey, "Inviting", "group");
        try {
          await this.#formGroup(feedbackKey);
          this.#store.clearFeedback(feedbackKey);
        } finally {
          this.#groupInFlight = false;
        }
        return;
      }
      if (cell.type === "follow" && cell.available && !this.#followInFlight) {
        const feedbackKey = key.action.id;
        this.#followInFlight = true;
        this.#setActionFeedback(feedbackKey, "Following", "follow");
        try {
          await this.#startFollow();
          this.#store.clearFeedback(feedbackKey);
        } finally {
          this.#followInFlight = false;
        }
        return;
      }
      if (cell.type === "assist" && cell.available && !this.#assistInFlight) {
        const feedbackKey = key.action.id;
        this.#assistInFlight = true;
        this.#setActionFeedback(feedbackKey, "Sending", "assist");
        try {
          await this.#startAssist();
          this.#store.clearFeedback(feedbackKey);
        } finally {
          this.#assistInFlight = false;
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

  async #startFollow(): Promise<void> {
    const plan = buildFollowPlan(this.#store.view);
    if (!plan.available || !plan.leader) {
      throw new CommandError(
        "follow_unavailable",
        "No named active leader and ready followers are available.",
      );
    }

    const command = `/follow ${plan.leader.character.trim()}`;
    const results = await Promise.allSettled(
      plan.followers.map((follower) =>
        this.#client.sendText(follower.id, command, true),
      ),
    );
    const failures = results.filter((result) => result.status === "rejected");
    if (failures.length > 0) {
      throw new CommandError(
        failures.length === results.length ? "follow_failed" : "follow_partial",
        failures.length === results.length
          ? "No ready box received the follow command."
          : "Some ready boxes missed the follow command.",
      );
    }
  }

  async #startAssist(): Promise<void> {
    const plan = buildAssistPlan(this.#store.view);
    if (!plan.available || !plan.main) {
      throw new CommandError(
        "assist_unavailable",
        "No named active main box and ready assistants are available.",
      );
    }

    const command = `/assist ${plan.main.character.trim()}`;
    const results = await Promise.allSettled(
      plan.assistants.map((assistant) =>
        this.#client.sendText(assistant.id, command, true),
      ),
    );
    const failures = results.filter((result) => result.status === "rejected");
    if (failures.length > 0) {
      throw new CommandError(
        failures.length === results.length ? "assist_failed" : "assist_partial",
        failures.length === results.length
          ? "No ready box received the assist command."
          : "Some ready boxes missed the assist command.",
      );
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

    this.#setActionFeedback(feedbackKey, "Waiting 1 sec", "group");
    await wait(GROUP_ACCEPT_DELAY_MS);
    this.#setActionFeedback(feedbackKey, "Accepting", "group");

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

  #setActionFeedback(
    key: string,
    message: string,
    motion: "group" | "follow" | "assist",
  ): void {
    this.#store.setFeedback(
      key,
      { kind: "pending", message, motion },
      ACTION_FEEDBACK_TIMEOUT_MS,
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
      return (
        (cell.type === "swap" && cell.armed) ||
        (cell.type === "feedback" &&
          cell.feedback.kind === "pending" &&
          Boolean(cell.feedback.motion))
      );
    });

    if (active && !this.#motionTimer) {
      this.#motionFrame = 0;
      this.#motionTimer = setInterval(() => {
        this.#motionFrame = (this.#motionFrame + 1) % ACTIVE_TILE_FRAME_COUNT;
        this.#queueRender();
      }, ACTIVE_TILE_FRAME_MS);
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
      case "window_number_swap_failed":
        return "Swap unavailable";
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
      case "follow_unavailable":
        return "No ready boxes";
      case "follow_failed":
        return "Follow failed";
      case "follow_partial":
        return "Partial follow";
      case "assist_unavailable":
        return "No ready boxes";
      case "assist_failed":
        return "Assist failed";
      case "assist_partial":
        return "Partial assist";
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
