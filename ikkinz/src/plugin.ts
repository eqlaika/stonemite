import streamDeck from "@elgato/streamdeck";
import {
  createDashboardKeyActions,
  type PluginSettings,
} from "./actions/dashboard-key";
import { connectionForSettings } from "./actions/dashboard-controller";
import { DashboardStore, type ConnectionStatus } from "./state/store";
import { TrusharClient } from "./trushar/client";

streamDeck.logger.setLevel("info");
streamDeck.settings.useExperimentalMessageIdentifiers = true;

const store = new DashboardStore();
let streamDeckConnected = false;

const client = new TrusharClient({
  onState: (state) => store.setSnapshot(state),
  onStatus: (status) => {
    store.setConnection(status);
    if (streamDeckConnected) void sendStatus(status);
  },
  log: (message) => streamDeck.logger.info(message),
});

for (const action of createDashboardKeyActions(store, client)) {
  streamDeck.actions.registerAction(action);
}
streamDeck.system.onSystemDidWakeUp(() => client.reconnect());
streamDeck.settings.onDidReceiveGlobalSettings<PluginSettings>((event) => {
  applySettings(event.settings);
});

await streamDeck.connect();
streamDeckConnected = true;
applySettings(await streamDeck.settings.getGlobalSettings<PluginSettings>());

function applySettings(settings: PluginSettings): void {
  try {
    client.configure(connectionForSettings(settings));
  } catch (error) {
    client.disconnect();
    store.setConnection({
      state: "error",
      title: "Saved connection not valid",
      detail:
        error instanceof Error
          ? error.message
          : "Use this PC or pair over LAN again.",
    });
  }
}

async function sendStatus(status: ConnectionStatus): Promise<void> {
  try {
    await streamDeck.ui.sendToPropertyInspector({
      type: "connection-status",
      status: {
        state: status.state,
        title: status.title,
        detail: status.detail,
      },
    });
  } catch {
    // No property inspector is open. The selected key will request fresh status when it appears.
  }
}
