import streamDeck, {
  type DidReceiveSettingsEvent,
  type KeyAction,
  type KeyDownEvent,
  type PropertyInspectorDidAppearEvent,
  type PropertyInspectorDidDisappearEvent,
  type SendToPluginEvent,
  SingletonAction,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import type { JsonObject, JsonValue } from "@elgato/utils";
import {
  isLucideAnimatedIcon,
  type LucideAnimatedIcon,
} from "../render/action-icons";
import { renderCell, renderHotkeyTile } from "../render/key-svg";
import { DashboardStore } from "../state/store";
import { CommandError, TrusharClient } from "../trushar/client";
import type { EqActionTargets, TrusharState } from "../types/trushar";
import { HOTKEY_ACTION_DEFINITION } from "./key-definitions";

export const HOTKEY_MANIFEST_ID = HOTKEY_ACTION_DEFINITION.uuid;
const FEEDBACK_TIMEOUT_MS = 3_000;
const SUCCESS_HOLD_MS = 700;
const MOTION_FRAME_MS = 125;
const MAX_TILE_LABEL_LENGTH = 14;
export const DEFAULT_HOTKEY_COLOR = "#59d8d0";

export interface HotkeySettings extends JsonObject {
  targetMode?: string;
  windowNumbers?: number[];
  mapping?: string;
  label?: string;
  icon?: string;
  color?: string;
}

export interface NormalizedHotkeySettings {
  targetMode: "all" | "selected";
  windowNumbers: number[];
  mapping?: string;
  label: string;
  icon: LucideAnimatedIcon;
  color: string;
}

interface VisibleHotkey {
  action: KeyAction<HotkeySettings>;
  settings: NormalizedHotkeySettings;
  lastImage?: string;
  error?: { message: string; until: number };
  errorTimer?: ReturnType<typeof setTimeout>;
  successHeld?: boolean;
  successTimer?: ReturnType<typeof setTimeout>;
}

export class HotkeyAction extends SingletonAction<HotkeySettings> {
  override readonly manifestId = HOTKEY_MANIFEST_ID;
  readonly #store: DashboardStore;
  readonly #client: TrusharClient;
  readonly #keys = new Map<string, VisibleHotkey>();
  readonly #inFlight = new Set<string>();
  #motionFrame = 0;
  #motionTimer: ReturnType<typeof setInterval> | null = null;
  #renderQueued = false;
  #rendering = false;
  #inspectorAction: KeyAction<HotkeySettings> | null = null;

  constructor(store: DashboardStore, client: TrusharClient) {
    super();
    this.#store = store;
    this.#client = client;
    this.#store.subscribe(() => {
      this.#queueRender();
      if (this.#inspectorAction)
        void this.#sendInspectorState(this.#inspectorAction);
    });
  }

  override onWillAppear(event: WillAppearEvent<HotkeySettings>): void {
    if (!event.action.isKey() || event.action.isInMultiAction()) return;
    this.#keys.set(event.action.id, {
      action: event.action,
      settings: normalizeHotkeySettings(event.payload.settings),
    });
    this.#queueRender();
  }

  override onWillDisappear(event: WillDisappearEvent<HotkeySettings>): void {
    const key = this.#keys.get(event.action.id);
    if (key?.errorTimer) clearTimeout(key.errorTimer);
    if (key?.successTimer) clearTimeout(key.successTimer);
    this.#keys.delete(event.action.id);
    this.#inFlight.delete(event.action.id);
    if (this.#inspectorAction?.id === event.action.id)
      this.#inspectorAction = null;
    this.#syncMotion();
  }

  override onDidReceiveSettings(
    event: DidReceiveSettingsEvent<HotkeySettings>,
  ): void {
    if (!event.action.isKey()) return;
    const key = this.#keys.get(event.action.id);
    if (key) {
      this.#clearSuccess(key);
      key.settings = normalizeHotkeySettings(event.payload.settings);
      delete key.lastImage;
    }
    this.#queueRender();
    if (this.#inspectorAction?.id === event.action.id)
      void this.#sendInspectorState(event.action);
  }

  override async onKeyDown(event: KeyDownEvent<HotkeySettings>): Promise<void> {
    if (!event.action.isKey() || this.#inFlight.has(event.action.id)) return;
    const key = this.#keys.get(event.action.id);
    if (!key) return;
    const settings = key.settings;
    if (!settings.mapping) {
      await event.action.showAlert();
      return;
    }
    const unavailable = hotkeyUnavailableReason(
      this.#store.view.snapshot,
      settings,
    );
    if (this.#store.view.connection.state !== "connected" || unavailable) {
      this.#setError(key, unavailable ?? "Stonemite offline");
      await event.action.showAlert();
      return;
    }

    this.#clearSuccess(key);
    this.#inFlight.add(event.action.id);
    this.#syncMotion();
    this.#queueRender();
    let delivered = false;
    try {
      const result = await this.#client.sendEqActionBatch(
        targetsForSettings(settings),
        {
          type: "keymap",
          mapping: settings.mapping,
        },
      );
      if (result.result.type !== "eq_action_batch_delivered") {
        throw new CommandError(
          "protocol_error",
          "Stonemite returned the wrong mapped-hotkey result.",
        );
      }
      delivered = true;
    } catch (error) {
      this.#setError(key, friendlyHotkeyError(error));
      await event.action.showAlert();
    } finally {
      this.#inFlight.delete(event.action.id);
      if (delivered) this.#holdSuccess(key);
      this.#syncMotion();
      this.#queueRender();
    }
  }

  override async onPropertyInspectorDidAppear(
    event: PropertyInspectorDidAppearEvent<HotkeySettings>,
  ): Promise<void> {
    if (!event.action.isKey()) return;
    this.#inspectorAction = event.action;
    await this.#sendInspectorState(event.action);
  }

  override onPropertyInspectorDidDisappear(
    event: PropertyInspectorDidDisappearEvent<HotkeySettings>,
  ): void {
    if (this.#inspectorAction?.id === event.action.id)
      this.#inspectorAction = null;
  }

  override async onSendToPlugin(
    event: SendToPluginEvent<JsonValue, HotkeySettings>,
  ): Promise<void> {
    if (!event.action.isKey() || !isRecord(event.payload)) return;
    const type = event.payload.type;
    if (type !== "get-hotkey-state" && type !== "list-hotkey-mappings") return;
    const requestId =
      typeof event.payload.requestId === "string"
        ? event.payload.requestId
        : undefined;
    const targets = parseTargets(event.payload.targets);
    await this.#sendInspectorState(event.action, targets, requestId);
  }

  async #sendInspectorState(
    action: KeyAction<HotkeySettings>,
    draftTargets?: EqActionTargets,
    requestId?: string,
  ): Promise<void> {
    const visible = this.#keys.get(action.id);
    const saved = visible
      ? visible.settings
      : normalizeHotkeySettings(await action.getSettings());
    const targets = draftTargets ?? targetsForSettings(saved);
    const snapshot = this.#store.view.snapshot;
    let mappings: string[] = [];
    let mappedWindowNumbers: number[] = [];
    let mappingError: string | undefined;
    if (
      requestId &&
      this.#store.view.connection.state === "connected" &&
      snapshot?.capabilities.eq_actions.keymap_actions
    ) {
      try {
        const result = await this.#client.listEqKeymapActions(targets);
        mappings = result.mappings;
        mappedWindowNumbers = result.window_numbers;
      } catch (error) {
        mappingError =
          error instanceof Error
            ? error.message
            : "Stonemite could not read the selected EQ key mappings.";
      }
    }
    await streamDeck.ui.sendToPropertyInspector({
      type: "hotkey-state",
      ...(requestId ? { requestId } : {}),
      settings: serializeHotkeySettings(saved),
      connection: {
        state: this.#store.view.connection.state,
        title: this.#store.view.connection.title,
        detail: this.#store.view.connection.detail,
      },
      capabilityAvailable: Boolean(
        snapshot?.capabilities.eq_actions.keymap_actions,
      ),
      boxes: boxSummaries(snapshot),
      mappings: mappings.map((mapping) => ({
        value: mapping,
        label: formatEqMappingLabel(mapping),
      })),
      mappedWindowNumbers,
      ...(mappingError ? { mappingError } : {}),
    });
  }

  #holdSuccess(key: VisibleHotkey): void {
    this.#clearSuccess(key);
    key.successHeld = true;
    key.successTimer = setTimeout(() => {
      delete key.successHeld;
      delete key.successTimer;
      this.#queueRender();
    }, SUCCESS_HOLD_MS);
    key.successTimer.unref?.();
  }

  #clearSuccess(key: VisibleHotkey): void {
    if (key.successTimer) clearTimeout(key.successTimer);
    delete key.successHeld;
    delete key.successTimer;
  }

  #setError(key: VisibleHotkey, message: string): void {
    this.#clearSuccess(key);
    if (key.errorTimer) clearTimeout(key.errorTimer);
    key.error = { message, until: Date.now() + FEEDBACK_TIMEOUT_MS };
    key.errorTimer = setTimeout(() => {
      delete key.error;
      delete key.errorTimer;
      this.#queueRender();
    }, FEEDBACK_TIMEOUT_MS);
    key.errorTimer.unref?.();
    this.#queueRender();
  }

  #syncMotion(): void {
    if (this.#inFlight.size > 0 && !this.#motionTimer) {
      this.#motionFrame = 0;
      this.#motionTimer = setInterval(() => {
        this.#motionFrame = (this.#motionFrame + 1) % 8;
        this.#queueRender();
      }, MOTION_FRAME_MS);
      this.#motionTimer.unref?.();
      return;
    }
    if (this.#inFlight.size === 0 && this.#motionTimer) {
      clearInterval(this.#motionTimer);
      this.#motionTimer = null;
      this.#motionFrame = 0;
    }
  }

  #queueRender(): void {
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
        const updates: Promise<void>[] = [];
        for (const key of this.#keys.values()) {
          const image = this.#renderKey(key);
          if (image === key.lastImage) continue;
          updates.push(
            key.action.setImage(image).then(() => {
              key.lastImage = image;
            }),
          );
        }
        await Promise.allSettled(updates);
      }
    } finally {
      this.#rendering = false;
    }
  }

  #renderKey(key: VisibleHotkey): string {
    if (key.error && key.error.until > Date.now()) {
      return renderCell({
        type: "feedback",
        feedback: {
          kind: "error",
          message: key.error.message,
          until: key.error.until,
        },
      });
    }
    const configured = Boolean(key.settings.mapping);
    const unavailable = hotkeyUnavailableReason(
      this.#store.view.snapshot,
      key.settings,
    );
    const available =
      this.#store.view.connection.state === "connected" &&
      configured &&
      !unavailable;
    return renderHotkeyTile(
      {
        configured,
        label: key.settings.label,
        icon: key.settings.icon,
        color: key.settings.color,
        targets: targetSummary(key.settings),
        ...(unavailable ? { status: unavailable.toUpperCase() } : {}),
        available,
        active: this.#inFlight.has(key.action.id) || Boolean(key.successHeld),
        animating: this.#inFlight.has(key.action.id),
      },
      this.#motionFrame,
    );
  }
}

export function normalizeHotkeySettings(
  settings: HotkeySettings,
): NormalizedHotkeySettings {
  const targetMode = settings.targetMode === "selected" ? "selected" : "all";
  const windowNumbers = normalizeWindowNumbers(settings.windowNumbers);
  const mapping = normalizeMapping(settings.mapping);
  const defaultLabel = mapping ? shortEqMappingLabel(mapping) : "Configure";
  const label =
    typeof settings.label === "string" && settings.label.trim()
      ? settings.label.trim().slice(0, MAX_TILE_LABEL_LENGTH)
      : defaultLabel;
  return {
    targetMode,
    windowNumbers,
    ...(mapping ? { mapping } : {}),
    label,
    icon: isLucideAnimatedIcon(settings.icon) ? settings.icon : "keyboard",
    color: normalizeColor(settings.color),
  };
}

export function serializeHotkeySettings(
  settings: NormalizedHotkeySettings,
): HotkeySettings {
  return {
    targetMode: settings.targetMode,
    windowNumbers: settings.windowNumbers,
    ...(settings.mapping ? { mapping: settings.mapping } : {}),
    label: settings.label,
    icon: settings.icon,
    color: settings.color,
  };
}

export function targetsForSettings(
  settings: NormalizedHotkeySettings,
): EqActionTargets {
  return settings.targetMode === "all"
    ? { type: "all_loaded" }
    : {
        type: "window_numbers",
        window_numbers: settings.windowNumbers,
      };
}

export function formatEqMappingLabel(mapping: string): string {
  const hotbar = mapping.match(/^HOT(\d+)_(\d+)$/u);
  if (hotbar) return `Hotbar ${hotbar[1]} button ${hotbar[2]}`;
  const spell = mapping.match(/^CAST(\d+)$/u);
  if (spell) return `Spell gem ${spell[1]}`;
  if (mapping === "USE") return "Use center screen";
  if (mapping === "INVITE_FOLLOW") return "Invite/follow";
  return mapping
    .replace(/^CMD_/u, "")
    .split("_")
    .filter(Boolean)
    .map((part) =>
      ["AA", "NPC", "PC", "PVP", "UI", "XTARGET"].includes(part)
        ? part
        : part.toLowerCase(),
    )
    .join(" ")
    .replace(/^./u, (character) => character.toUpperCase());
}

function shortEqMappingLabel(mapping: string): string {
  const hotbar = mapping.match(/^HOT(\d+)_(\d+)$/u);
  if (hotbar) return `Hot ${hotbar[1]}·${hotbar[2]}`;
  const spell = mapping.match(/^CAST(\d+)$/u);
  if (spell) return `Gem ${spell[1]}`;
  if (mapping === "USE") return "Use";
  if (mapping === "INVITE_FOLLOW") return "Invite";
  return formatEqMappingLabel(mapping).slice(0, MAX_TILE_LABEL_LENGTH);
}

function normalizeColor(value: unknown): string {
  return typeof value === "string" && /^#[0-9a-f]{6}$/iu.test(value)
    ? value.toLowerCase()
    : DEFAULT_HOTKEY_COLOR;
}

function normalizeMapping(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const mapping = value.trim().toUpperCase();
  return mapping.length >= 1 &&
    mapping.length <= 128 &&
    /^[A-Z0-9_]+$/u.test(mapping)
    ? mapping
    : undefined;
}

function normalizeWindowNumbers(value: unknown): number[] {
  const numbers = Array.isArray(value)
    ? value.filter(
        (number): number is number =>
          Number.isSafeInteger(number) && number >= 1 && number <= 6,
      )
    : [];
  const unique = [...new Set(numbers)].sort((a, b) => a - b);
  return unique.length > 0 ? unique : [1];
}

function parseTargets(
  value: JsonValue | undefined,
): EqActionTargets | undefined {
  if (!isRecord(value) || typeof value.type !== "string") return undefined;
  if (value.type === "all_loaded") return { type: "all_loaded" };
  if (value.type !== "window_numbers") return undefined;
  const windowNumbers = normalizeWindowNumbers(value.window_numbers);
  return { type: "window_numbers", window_numbers: windowNumbers };
}

function hotkeyUnavailableReason(
  snapshot: TrusharState | null,
  settings: NormalizedHotkeySettings,
): string | undefined {
  if (!snapshot) return "No client state";
  if (!snapshot.capabilities.eq_actions.keymap_actions)
    return "Update Stonemite";
  if (settings.targetMode === "all") {
    if (snapshot.clients.length === 0) return "No loaded boxes";
    return snapshot.clients.every((client) => client.input_ready)
      ? undefined
      : "Input not ready";
  }
  for (const number of settings.windowNumbers) {
    const client = snapshot.clients.find(
      (candidate) => candidate.window_number === number,
    );
    if (!client) return `Box ${number} missing`;
    if (!client.input_ready) return `Box ${number} not ready`;
  }
  return undefined;
}

function targetSummary(settings: NormalizedHotkeySettings): string {
  if (settings.targetMode === "all") return "ALL";
  if (
    settings.windowNumbers.length === 6 &&
    settings.windowNumbers.every((number, index) => number === index + 1)
  )
    return "1–6";
  return settings.windowNumbers.join("·");
}

function boxSummaries(snapshot: TrusharState | null): Array<JsonObject> {
  return Array.from({ length: 6 }, (_, index) => {
    const windowNumber = index + 1;
    const client = snapshot?.clients.find(
      (candidate) => candidate.window_number === windowNumber,
    );
    return {
      windowNumber,
      loaded: Boolean(client),
      inputReady: Boolean(client?.input_ready),
      ...(client?.character ? { character: client.character } : {}),
      ...(client?.class_code ? { classCode: client.class_code } : {}),
    };
  });
}

function friendlyHotkeyError(error: unknown): string {
  if (error instanceof CommandError) {
    if (error.message.includes("may have received the action"))
      return "Partial send";
    switch (error.code) {
      case "client_not_found":
        return "Box missing";
      case "input_unavailable":
        return "Input not ready";
      case "eq_action_unbound":
        return "Action unbound";
      case "command_timeout":
        return "Timed out";
      case "input_operation_failed":
        return "Delivery failed";
      default:
        return error.message;
    }
  }
  return error instanceof Error ? error.message : "Hotkey failed";
}

function isRecord(
  value: JsonValue | undefined,
): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
