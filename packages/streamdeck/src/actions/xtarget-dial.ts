import {
  type DialAction,
  type DialDownEvent,
  type DialRotateEvent,
  type DidReceiveSettingsEvent,
  SingletonAction,
  type TouchTapEvent,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import { CLASS_IMAGES } from "../render/assets.generated";
import { BADGE_COLORS } from "../state/layout";
import { DashboardStore } from "../state/store";
import { CommandError, TrusharClient } from "../trushar/client";
import type {
  ConsiderDifficulty,
  TrusharClient as TrusharClientState,
} from "../types/trushar";
import { normalizeWindowNumber, type BoxSettings } from "./box-settings";
import { XTARGET_ACTION_DEFINITION } from "./key-definitions";

export const XTARGET_MANIFEST_ID = XTARGET_ACTION_DEFINITION.uuid;
const XTARGET_LAYOUT = "layouts/xtarget-v4.json";
const BACKGROUND_IDLE =
  "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyMDAiIGhlaWdodD0iMTAwIiB2aWV3Qm94PSIwIDAgMjAwIDEwMCI+PGRlZnM+PGxpbmVhckdyYWRpZW50IGlkPSJnIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMSI+PHN0b3Agc3RvcC1jb2xvcj0iIzIwMjQyYiIvPjxzdG9wIG9mZnNldD0iMSIgc3RvcC1jb2xvcj0iIzE3MWExZiIvPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxyZWN0IHdpZHRoPSIyMDAiIGhlaWdodD0iMTAwIiByeD0iOCIgZmlsbD0idXJsKCNnKSIvPjwvc3ZnPg==";
const BACKGROUND_ACTIVE =
  "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyMDAiIGhlaWdodD0iMTAwIiB2aWV3Qm94PSIwIDAgMjAwIDEwMCI+PGRlZnM+PGxpbmVhckdyYWRpZW50IGlkPSJnIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMSI+PHN0b3Agc3RvcC1jb2xvcj0iIzIzNDIzNyIvPjxzdG9wIG9mZnNldD0iMSIgc3RvcC1jb2xvcj0iIzE0MjMxZiIvPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxyZWN0IHdpZHRoPSIyMDAiIGhlaWdodD0iMTAwIiByeD0iOCIgZmlsbD0idXJsKCNnKSIvPjxyZWN0IHg9IjEuNSIgeT0iMS41IiB3aWR0aD0iMTk3IiBoZWlnaHQ9Ijk3IiByeD0iNyIgZmlsbD0ibm9uZSIgc3Ryb2tlPSIjODBkZjg5IiBzdHJva2Utd2lkdGg9IjMiLz48L3N2Zz4=";
const ERROR_HOLD_MS = 2_500;
const TARGET_SETTLE_MS = 300;

interface VisibleDial {
  action: DialAction<BoxSettings>;
  windowNumber: number;
  selectedSlot?: number;
  queuedSlot?: number;
  selectionTimer?: ReturnType<typeof setTimeout>;
  selectionEpoch: number;
  selecting: boolean;
  considering: boolean;
  pendingConsider: boolean;
  activating: boolean;
  error?: string;
  errorSlot?: number;
  errorTimer?: ReturnType<typeof setTimeout>;
  lastFeedback?: string;
}

type DialFeedback = Record<
  string,
  {
    value?: string;
    color?: string;
    background?: string;
    enabled?: boolean;
  }
>;

interface DialView {
  active: boolean;
  activity: string | undefined;
  badge: string;
  character: string;
  classIcon: string | undefined;
  detail: string | undefined;
  role: string;
  roleColor: string;
  slot: string;
}

export class XTargetDialAction extends SingletonAction<BoxSettings> {
  override readonly manifestId = XTARGET_MANIFEST_ID;
  readonly #store: DashboardStore;
  readonly #client: TrusharClient;
  readonly #dials = new Map<string, VisibleDial>();
  #renderQueued = false;
  #rendering = false;

  constructor(store: DashboardStore, client: TrusharClient) {
    super();
    this.#store = store;
    this.#client = client;
    this.#store.subscribe(() => this.#queueRender());
  }

  override async onWillAppear(
    event: WillAppearEvent<BoxSettings>,
  ): Promise<void> {
    if (!event.action.isDial()) return;
    const visible: VisibleDial = {
      action: event.action,
      windowNumber: normalizeWindowNumber(event.payload.settings.windowNumber),
      selectionEpoch: 0,
      selecting: false,
      considering: false,
      pendingConsider: false,
      activating: false,
    };
    this.#dials.set(event.action.id, visible);
    await Promise.allSettled([
      event.action.setFeedbackLayout(XTARGET_LAYOUT),
      event.action.setTriggerDescription({
        rotate: "Select Extended Target",
        push: "Consider target",
        touch: "Activate box",
      }),
    ]);
    this.#queueRender();
  }

  override onWillDisappear(event: WillDisappearEvent<BoxSettings>): void {
    const visible = this.#dials.get(event.action.id);
    if (visible?.errorTimer) clearTimeout(visible.errorTimer);
    if (visible?.selectionTimer) clearTimeout(visible.selectionTimer);
    this.#dials.delete(event.action.id);
  }

  override onDidReceiveSettings(
    event: DidReceiveSettingsEvent<BoxSettings>,
  ): void {
    if (!event.action.isDial()) return;
    const visible = this.#dials.get(event.action.id);
    if (!visible) return;
    visible.windowNumber = normalizeWindowNumber(
      event.payload.settings.windowNumber,
    );
    delete visible.selectedSlot;
    delete visible.queuedSlot;
    if (visible.selectionTimer) clearTimeout(visible.selectionTimer);
    delete visible.selectionTimer;
    visible.selectionEpoch += 1;
    visible.pendingConsider = false;
    delete visible.lastFeedback;
    this.#clearError(visible);
    this.#queueRender();
  }

  override onDialRotate(event: DialRotateEvent<BoxSettings>): void {
    if (!event.action.isDial() || event.payload.ticks === 0) return;
    const visible = this.#dials.get(event.action.id);
    if (!visible) return;
    const client = this.#clientFor(visible);
    const slots = client?.xtarget.slots ?? [];
    if (!client || !client.xtarget.supported || slots.length === 0) {
      this.#setError(
        visible,
        client
          ? client.xtarget.supported
            ? "NO XTARGETS"
            : "UPDATE STONEMITE"
          : "BOX NOT LOADED",
      );
      void event.action.showAlert();
      return;
    }
    this.#reconcileSelection(visible, client);
    const index = Math.max(
      0,
      slots.findIndex((slot) => slot.slot === visible.selectedSlot),
    );
    // Apply every physical detent reported in a batched SDK event without
    // adding synthetic acceleration. Empty saved slots are absent from this list.
    const nextIndex = Math.max(
      0,
      Math.min(slots.length - 1, index + Math.trunc(event.payload.ticks)),
    );
    const next = slots[nextIndex];
    if (!next || next.slot === visible.selectedSlot) return;
    visible.selectedSlot = next.slot;
    this.#clearError(visible);
    this.#queueRender();

    if (!this.#canSendInput(client)) {
      this.#setError(visible, "INPUT UNAVAILABLE", next.slot);
    } else if (!next.bound) {
      this.#setError(visible, "UNBOUND", next.slot);
    }
    // Send the semantic direct-slot attempt even when it is known to be
    // unavailable. Stonemite clears stale Consider telemetry before reporting
    // the binding/readiness error, while still delivering no EQ input.
    visible.queuedSlot = next.slot;
    this.#scheduleSelection(visible);
  }

  override async onDialDown(event: DialDownEvent<BoxSettings>): Promise<void> {
    if (!event.action.isDial()) return;
    const visible = this.#dials.get(event.action.id);
    if (!visible || visible.considering) return;
    if (
      visible.selecting ||
      visible.queuedSlot !== undefined ||
      visible.selectionTimer !== undefined
    ) {
      visible.pendingConsider = true;
      return;
    }
    await this.#sendConsider(visible);
  }

  override async onTouchTap(event: TouchTapEvent<BoxSettings>): Promise<void> {
    if (!event.action.isDial() || event.payload.hold) return;
    const visible = this.#dials.get(event.action.id);
    if (!visible || visible.activating) return;
    const client = this.#clientFor(visible);
    if (!client?.activatable) {
      this.#setError(visible, client ? "CANNOT ACTIVATE" : "BOX NOT LOADED");
      await event.action.showAlert();
      return;
    }
    if (client.active) return;

    visible.activating = true;
    this.#clearError(visible);
    this.#queueRender();
    try {
      const response = await this.#client.activate(client.id);
      if (response.result.type !== "activated") {
        throw new CommandError(
          "protocol_error",
          "Stonemite returned the wrong activation result.",
        );
      }
    } catch (error) {
      this.#setError(visible, friendlyDialError(error));
      await event.action.showAlert();
    } finally {
      visible.activating = false;
      this.#queueRender();
    }
  }

  async #sendConsider(visible: VisibleDial): Promise<void> {
    if (visible.considering) return;
    if (visible.error) {
      await visible.action.showAlert();
      return;
    }
    const client = this.#clientFor(visible);
    if (!client) {
      this.#setError(visible, "BOX NOT LOADED");
      await visible.action.showAlert();
      return;
    }
    if (!client.xtarget.supported) {
      this.#setError(visible, "UPDATE STONEMITE");
      await visible.action.showAlert();
      return;
    }
    this.#reconcileSelection(visible, client);
    const selected = client.xtarget.slots.find(
      (slot) => slot.slot === visible.selectedSlot,
    );
    if (!this.#canSendInput(client)) {
      this.#setError(visible, "INPUT UNAVAILABLE");
      await visible.action.showAlert();
      return;
    }
    if (
      !selected?.bound ||
      !client.xtarget.consider_bound ||
      visible.selectedSlot !== client.xtarget.selected_slot
    ) {
      this.#setError(visible, "UNBOUND", visible.selectedSlot);
      await visible.action.showAlert();
      return;
    }

    visible.considering = true;
    this.#clearError(visible);
    this.#queueRender();
    try {
      const response = await this.#client.sendEqAction(client.id, {
        type: "keymap",
        mapping: "CONSIDER",
      });
      if (response.result.type !== "eq_action_delivered") {
        throw new CommandError(
          "protocol_error",
          "Stonemite returned the wrong Consider result.",
        );
      }
    } catch (error) {
      this.#setError(visible, friendlyDialError(error));
      await visible.action.showAlert();
    } finally {
      visible.considering = false;
      this.#queueRender();
    }
  }

  #scheduleSelection(visible: VisibleDial): void {
    if (visible.selectionTimer) clearTimeout(visible.selectionTimer);
    visible.selectionTimer = setTimeout(() => {
      delete visible.selectionTimer;
      void this.#drainSelection(visible);
    }, TARGET_SETTLE_MS);
    visible.selectionTimer.unref?.();
    this.#queueRender();
  }

  async #drainSelection(visible: VisibleDial): Promise<void> {
    if (visible.selecting || visible.queuedSlot === undefined) return;
    const slot = visible.queuedSlot;
    delete visible.queuedSlot;
    const epoch = visible.selectionEpoch;
    visible.selecting = true;
    this.#queueRender();
    const client = this.#clientFor(visible);
    if (!client) {
      visible.pendingConsider = false;
      this.#setError(visible, "BOX NOT LOADED", slot);
      await visible.action.showAlert();
    } else {
      try {
        const response = await this.#client.sendEqAction(client.id, {
          type: "keymap",
          mapping: `TARGET_XTARGET_${slot}`,
        });
        if (response.result.type !== "eq_action_delivered") {
          throw new CommandError(
            "protocol_error",
            "Stonemite returned the wrong XTarget result.",
          );
        }
        if (
          visible.selectionEpoch === epoch &&
          visible.selectedSlot === slot &&
          visible.queuedSlot === undefined
        ) {
          this.#clearError(visible);
        }
      } catch (error) {
        if (
          visible.selectionEpoch === epoch &&
          visible.selectedSlot === slot &&
          visible.queuedSlot === undefined
        ) {
          visible.pendingConsider = false;
          this.#setError(visible, friendlyDialError(error), slot);
          await visible.action.showAlert();
        }
      }
    }

    visible.selecting = false;
    this.#queueRender();
    if (
      visible.queuedSlot !== undefined &&
      visible.selectionTimer === undefined
    ) {
      void this.#drainSelection(visible);
      return;
    }
    if (
      visible.pendingConsider &&
      visible.queuedSlot === undefined &&
      visible.selectionTimer === undefined
    ) {
      const consider = !visible.error;
      visible.pendingConsider = false;
      if (consider) await this.#sendConsider(visible);
    }
  }

  #clientFor(visible: VisibleDial): TrusharClientState | undefined {
    return this.#store.view.snapshot?.clients.find(
      (client) => client.window_number === visible.windowNumber,
    );
  }

  #canSendInput(client: TrusharClientState): boolean {
    return (
      this.#store.view.connection.state === "connected" && client.input_ready
    );
  }

  #reconcileSelection(visible: VisibleDial, client: TrusharClientState): void {
    const localSelectionIsValid =
      visible.selectedSlot !== undefined &&
      client.xtarget.slots.some((slot) => slot.slot === visible.selectedSlot);
    if (localSelectionIsValid) return;
    const selected =
      client.xtarget.selected_slot ?? client.xtarget.slots[0]?.slot;
    if (selected === undefined) delete visible.selectedSlot;
    else visible.selectedSlot = selected;
  }

  #setError(
    visible: VisibleDial,
    message: string,
    attemptedSlot?: number,
  ): void {
    this.#clearError(visible);
    visible.error = message;
    if (attemptedSlot !== undefined) visible.errorSlot = attemptedSlot;
    visible.errorTimer = setTimeout(() => {
      delete visible.error;
      delete visible.errorSlot;
      delete visible.errorTimer;
      const client = this.#clientFor(visible);
      if (client) this.#reconcileSelection(visible, client);
      this.#queueRender();
    }, ERROR_HOLD_MS);
    visible.errorTimer.unref?.();
    this.#queueRender();
  }

  #clearError(visible: VisibleDial): void {
    if (visible.errorTimer) clearTimeout(visible.errorTimer);
    delete visible.errorTimer;
    delete visible.error;
    delete visible.errorSlot;
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
        for (const visible of this.#dials.values()) {
          const feedback = this.#feedbackFor(visible);
          const signature = JSON.stringify(feedback);
          if (signature === visible.lastFeedback) continue;
          updates.push(
            visible.action.setFeedback(feedback).then(() => {
              visible.lastFeedback = signature;
            }),
          );
        }
        await Promise.allSettled(updates);
      }
    } finally {
      this.#rendering = false;
    }
  }

  #feedbackFor(visible: VisibleDial): DialFeedback {
    const offline = this.#store.view.connection.state !== "connected";
    const client = this.#clientFor(visible);
    const badge =
      BADGE_COLORS[(visible.windowNumber - 1) % BADGE_COLORS.length] ??
      BADGE_COLORS[0];
    if (offline) {
      return feedback({
        active: false,
        activity: undefined,
        badge,
        character: `BOX ${visible.windowNumber}`,
        classIcon: undefined,
        detail: "NO STONEMITE CONNECTION",
        role: "OFFLINE",
        roleColor: "#ff826f",
        slot: "—",
      });
    }
    if (!client) {
      return feedback({
        active: false,
        activity: undefined,
        badge,
        character: `BOX ${visible.windowNumber}`,
        classIcon: undefined,
        detail: `NO CLIENT IN BOX ${visible.windowNumber}`,
        role: "NOT LOADED",
        roleColor: "#ffc75c",
        slot: "—",
      });
    }
    const character = client.character ?? `BOX ${visible.windowNumber}`;
    const classIcon = classIconFor(client.class_code);
    const activity = visible.activating ? "ACTIVATING" : undefined;
    if (!client.xtarget.supported) {
      return feedback({
        active: client.active,
        activity,
        badge,
        character,
        classIcon,
        detail: "XTARGET STATE UNSUPPORTED",
        role: "UPDATE STONEMITE",
        roleColor: "#ffc75c",
        slot: "—",
      });
    }
    this.#reconcileSelection(visible, client);
    const selected = client.xtarget.slots.find(
      (slot) => slot.slot === visible.selectedSlot,
    );

    if (visible.error) {
      const attempted =
        client.xtarget.slots.find((slot) => slot.slot === visible.errorSlot) ??
        selected;
      const detail =
        visible.error === "UNBOUND"
          ? attempted && !attempted.bound
            ? `TARGET_XTARGET_${attempted.slot} · UNBOUND`
            : !client.xtarget.consider_bound
              ? "CONSIDER · UNBOUND"
              : "EQ ACTION · UNBOUND"
          : visible.error === "INPUT UNAVAILABLE"
            ? "TARGET INPUT UNAVAILABLE"
            : visible.error === "BOX NOT LOADED"
              ? "TARGET CLIENT UNAVAILABLE"
              : "TARGET ACTION FAILED";
      return feedback({
        active: client.active,
        activity,
        badge,
        character,
        classIcon,
        detail,
        role: visible.error,
        roleColor: "#ff826f",
        slot: attempted ? String(attempted.slot) : "—",
      });
    }
    if (client.xtarget.slots.length === 0 || !selected) {
      return feedback({
        active: client.active,
        activity,
        badge,
        character,
        classIcon,
        detail: "NO SAVED XTARGET ROLES",
        role: "NO XTARGETS",
        roleColor: "#ffc75c",
        slot: "—",
      });
    }
    if (!selected.bound) {
      return feedback({
        active: client.active,
        activity,
        badge,
        character,
        classIcon,
        detail: `TARGET_XTARGET_${selected.slot} · UNBOUND`,
        role: "UNBOUND",
        roleColor: "#ff826f",
        slot: String(selected.slot),
      });
    }
    if (visible.considering || client.xtarget.consider_pending) {
      return feedback({
        active: client.active,
        activity: "CONSIDERING",
        badge,
        character,
        classIcon,
        detail: undefined,
        role: selected.label,
        roleColor: "#59d8d0",
        slot: String(selected.slot),
      });
    }
    const consider =
      visible.selectedSlot === client.xtarget.selected_slot
        ? client.xtarget.consider
        : undefined;
    if (consider?.type === "no_target") {
      return feedback({
        active: client.active,
        activity: "NO TARGET",
        badge,
        character,
        classIcon,
        detail: undefined,
        role: selected.label,
        roleColor: "#59d8d0",
        slot: String(selected.slot),
      });
    }
    if (consider) {
      const difficulty = consider.difficulty.replace("_", " ").toUpperCase();
      return feedback({
        active: client.active,
        activity,
        badge,
        character,
        classIcon,
        detail: `${consider.level ? `LVL ${consider.level} · ` : ""}${difficulty}`,
        role: consider.target,
        roleColor: difficultyColor(consider.difficulty),
        slot: String(selected.slot),
      });
    }
    return feedback({
      active: client.active,
      activity:
        activity ??
        (visible.selecting ? `TARGETING ${selected.slot}` : undefined),
      badge,
      character,
      classIcon,
      detail: client.xtarget.consider_bound ? undefined : "CONSIDER · UNBOUND",
      role: selected.label,
      roleColor: client.xtarget.consider_bound ? "#59d8d0" : "#ffc75c",
      slot: String(selected.slot),
    });
  }
}

function feedback(view: DialView): DialFeedback {
  return {
    background: {
      value: view.active ? BACKGROUND_ACTIVE : BACKGROUND_IDLE,
    },
    classIcon: view.classIcon
      ? { value: view.classIcon, enabled: true }
      : { enabled: false },
    character: {
      value: view.character,
      color: view.active ? "#80df89" : "#f5f7fa",
    },
    slot: { value: view.slot, background: view.badge, color: "#f5f7fa" },
    role: { value: view.role, color: view.roleColor },
    detail: view.detail
      ? { value: view.detail, color: "#a7b0bb", enabled: true }
      : { enabled: false },
    activity: view.activity
      ? {
          value: view.activity,
          background: view.activity === "NO TARGET" ? "#ff826f" : "#ffc75c",
          color: "#101615",
          enabled: true,
        }
      : { enabled: false },
  };
}

function classIconFor(classCode: string | undefined): string | undefined {
  return classCode ? CLASS_IMAGES[classCode.toUpperCase()] : undefined;
}

function difficultyColor(difficulty: ConsiderDifficulty): string {
  switch (difficulty) {
    case "green":
      return "#80df89";
    case "light_blue":
      return "#70d8f0";
    case "blue":
      return "#69a5ff";
    case "white":
      return "#f5f7fa";
    case "yellow":
      return "#ffc75c";
    case "red":
      return "#ff826f";
    case "unknown":
      return "#a7b0bb";
  }
}

function friendlyDialError(error: unknown): string {
  if (error instanceof CommandError) {
    if (error.code === "eq_action_unbound") return "UNBOUND";
    if (error.code === "input_unavailable") return "INPUT UNAVAILABLE";
    if (
      error.code === "client_not_found" ||
      error.code === "target_disappeared"
    )
      return "BOX NOT LOADED";
  }
  return "FAILED";
}
