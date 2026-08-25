import {
  Bell,
  CircleHelp,
  Gamepad2,
  Info,
  Keyboard,
  ListOrdered,
  MonitorUp,
  Radio,
  RefreshCw,
  Save,
  UserRound,
  X,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

import { Button, InlineStatus } from "./components/Controls";
import { AboutPage } from "./pages/AboutPage";
import { AccountsPage } from "./pages/AccountsPage";
import { BoxOrderPage } from "./pages/BoxOrderPage";
import { BroadcastingPage } from "./pages/BroadcastingPage";
import { GeneralPage } from "./pages/GeneralPage";
import { HotkeysPage } from "./pages/HotkeysPage";
import { NotificationsPage } from "./pages/NotificationsPage";
import { PipPage } from "./pages/PipPage";
import { closeSettingsWindow, requestRestart } from "./settings/api";
import { useSettings } from "./settings/SettingsContext";

type PageId =
  | "general"
  | "accounts"
  | "boxOrder"
  | "pip"
  | "notifications"
  | "hotkeys"
  | "broadcasting"
  | "about";

interface NavigationItem {
  id: PageId;
  label: string;
  icon: LucideIcon;
}

const navigation: NavigationItem[] = [
  { id: "general", label: "General", icon: Gamepad2 },
  { id: "accounts", label: "Accounts", icon: UserRound },
  { id: "boxOrder", label: "Box order", icon: ListOrdered },
  { id: "pip", label: "PiP overlay", icon: MonitorUp },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "hotkeys", label: "Hotkeys", icon: Keyboard },
  { id: "broadcasting", label: "Broadcasting", icon: Radio },
  { id: "about", label: "About", icon: Info },
];

function CurrentPage({ page }: { page: PageId }) {
  switch (page) {
    case "general":
      return <GeneralPage />;
    case "accounts":
      return <AccountsPage />;
    case "boxOrder":
      return <BoxOrderPage />;
    case "pip":
      return <PipPage />;
    case "notifications":
      return <NotificationsPage />;
    case "hotkeys":
      return <HotkeysPage />;
    case "broadcasting":
      return <BroadcastingPage />;
    case "about":
      return <AboutPage />;
  }
}

export function App() {
  const {
    loadState,
    loadError,
    saveState,
    saveError,
    runtime,
    dirty,
    save,
    reset,
  } = useSettings();
  const [page, setPage] = useState<PageId>("general");
  const [restartRequired, setRestartRequired] = useState(false);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key.toLowerCase() === "s") {
        event.preventDefault();
        if (dirty && saveState !== "saving") void handleSave();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  async function handleSave() {
    const outcome = await save();
    if (!outcome) return;
    if (outcome.restartRequired) {
      setRestartRequired(true);
    } else {
      await closeSettingsWindow();
    }
  }

  async function handleCancel() {
    if (dirty && !window.confirm("Discard your unsaved settings changes?"))
      return;
    reset();
    await closeSettingsWindow();
  }

  async function handleRestart() {
    await requestRestart();
    await closeSettingsWindow();
  }

  if (loadState === "loading") {
    return (
      <main className="launch-state" aria-busy="true">
        <RefreshCw className="spin" aria-hidden="true" />
        <h1>Opening settings</h1>
        <p>Reading your Stonemite configuration…</p>
      </main>
    );
  }

  if (loadState === "error") {
    return (
      <main className="launch-state">
        <CircleHelp aria-hidden="true" />
        <h1>Settings could not open</h1>
        <p>{loadError ?? "Stonemite could not read its configuration."}</p>
        <Button variant="primary" onClick={() => window.location.reload()}>
          Try again
        </Button>
      </main>
    );
  }

  return (
    <div className="app-shell">
      <aside className="sidebar" aria-label="Settings sections">
        <div className="product-lockup">
          <img src="/app.png" width="38" height="38" alt="" />
          <div>
            <strong>Stonemite</strong>
            <span>Settings</span>
          </div>
        </div>
        <nav>
          {navigation.map((item) => {
            const Icon = item.icon;
            return (
              <button
                key={item.id}
                type="button"
                className={page === item.id ? "nav-item active" : "nav-item"}
                aria-label={item.label}
                aria-current={page === item.id ? "page" : undefined}
                onClick={() => setPage(item.id)}
              >
                <Icon size={17} strokeWidth={1.8} aria-hidden="true" />
                <span>{item.label}</span>
              </button>
            );
          })}
        </nav>
        <div className="sidebar-meta">Version {runtime?.version}</div>
      </aside>

      <main className="content-pane">
        <div className="content-scroll" tabIndex={-1}>
          <CurrentPage page={page} />
          {saveError ? (
            <InlineStatus tone="error" title="Settings were not saved">
              {saveError}
            </InlineStatus>
          ) : null}
          {restartRequired ? (
            <InlineStatus tone="warning" title="Restart required">
              <p>Integration access changed. Restart Stonemite to apply it.</p>
              <div className="status-actions">
                <Button variant="primary" onClick={() => void handleRestart()}>
                  Restart now
                </Button>
                <Button onClick={() => void closeSettingsWindow()}>
                  Later
                </Button>
              </div>
            </InlineStatus>
          ) : null}
        </div>

        <footer className="action-bar">
          <span className="save-state" aria-live="polite">
            {saveState === "saving"
              ? "Saving settings…"
              : dirty
                ? "Unsaved changes"
                : "All changes saved"}
          </span>
          <div className="action-buttons">
            <Button
              onClick={() => void handleCancel()}
              disabled={saveState === "saving"}
            >
              <X size={15} aria-hidden="true" />
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={() => void handleSave()}
              disabled={!dirty || saveState === "saving"}
            >
              <Save size={15} aria-hidden="true" />
              Save
            </Button>
          </div>
        </footer>
      </main>
    </div>
  );
}
