import {
  type KeyDownEvent,
  type KeyUpEvent,
  type SendToPluginEvent,
  SingletonAction,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import type { JsonObject, JsonValue } from "@elgato/utils";
import { DashboardStore } from "../state/store";
import { TrusharClient } from "../trushar/client";
import { DashboardController } from "./dashboard-controller";
import {
  DASHBOARD_ACTION_DEFINITIONS,
  type DashboardActionDefinition,
} from "./key-definitions";

export type { PluginSettings } from "./dashboard-controller";

export function createDashboardKeyActions(
  store: DashboardStore,
  client: TrusharClient,
): DashboardKeyAction[] {
  const controller = new DashboardController(store, client);
  return DASHBOARD_ACTION_DEFINITIONS.map(
    (definition) => new DashboardKeyAction(controller, definition),
  );
}

export class DashboardKeyAction extends SingletonAction {
  override readonly manifestId: string;
  readonly #controller: DashboardController;

  constructor(
    controller: DashboardController,
    definition: DashboardActionDefinition,
  ) {
    super();
    this.#controller = controller;
    this.manifestId = definition.uuid;
  }

  override onWillAppear(event: WillAppearEvent): Promise<void> {
    return this.#controller.onWillAppear(event);
  }

  override onWillDisappear(event: WillDisappearEvent): void {
    this.#controller.onWillDisappear(event);
  }

  override onKeyDown(event: KeyDownEvent): Promise<void> {
    return this.#controller.onKeyDown(event);
  }

  override onKeyUp(event: KeyUpEvent): Promise<void> {
    return this.#controller.onKeyUp(event);
  }

  override onPropertyInspectorDidAppear(): Promise<void> {
    return this.#controller.onPropertyInspectorDidAppear();
  }

  override onSendToPlugin(
    event: SendToPluginEvent<JsonValue, JsonObject>,
  ): Promise<void> {
    return this.#controller.onSendToPlugin(event);
  }
}
