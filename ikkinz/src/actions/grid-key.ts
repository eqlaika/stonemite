import {
  action,
  type KeyDownEvent,
  type SendToPluginEvent,
  SingletonAction,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";
import type { JsonObject, JsonValue } from "@elgato/utils";
import { DashboardStore } from "../state/store";
import { TrusharClient } from "../trushar/client";
import { GridKeyController } from "./grid-key-controller";

export type { PluginSettings } from "./grid-key-controller";

@action({ UUID: "co.laikasoft.ikkinz.grid-key" })
export class GridKeyAction extends SingletonAction {
  readonly #controller: GridKeyController;

  constructor(store: DashboardStore, client: TrusharClient) {
    super();
    this.#controller = new GridKeyController(store, client);
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

  override onPropertyInspectorDidAppear(): Promise<void> {
    return this.#controller.onPropertyInspectorDidAppear();
  }

  override onSendToPlugin(
    event: SendToPluginEvent<JsonValue, JsonObject>,
  ): Promise<void> {
    return this.#controller.onSendToPlugin(event);
  }
}
