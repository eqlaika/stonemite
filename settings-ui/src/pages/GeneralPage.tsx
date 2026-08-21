import { useEffect, useId, useRef, useState } from "react";

import {
  Button,
  Field,
  FormSection,
  InlineStatus,
  RangeInput,
  TextInput,
  Toggle,
} from "../components/Controls";
import { SettingsPage } from "../components/SettingsPage";
import {
  beginPairing,
  cancelPairing,
  chooseEqDirectory,
  pairingIsOpen,
} from "../settings/api";
import { useSettings } from "../settings/SettingsContext";
import type {
  IntegrationSettings,
  PairingSession,
  SettingsDraft,
} from "../settings/types";
import "./GeneralPage.css";

interface ActivePairing extends PairingSession {
  expiresAt: number;
}

interface PairingNotice {
  tone: "info" | "success" | "warning" | "error";
  title: string;
  message: string;
}

// The settings window stays in one JavaScript process until Stonemite restarts.
// Retaining the loaded values across sidebar navigation keeps pairing disabled
// after integration changes are saved but before that required restart.
let loadedIntegrations: IntegrationSettings | null = null;

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function sameIntegrations(
  left: IntegrationSettings,
  right: IntegrationSettings,
): boolean {
  return left.enabled === right.enabled && left.lanEnabled === right.lanEnabled;
}

function formatCountdown(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

export function GeneralPage() {
  const { draft, setDraft, runtime, dirty } = useSettings();
  const directoryId = useId();
  const mounted = useRef(false);
  const [choosingDirectory, setChoosingDirectory] = useState(false);
  const [directoryError, setDirectoryError] = useState<string | null>(null);
  const [pairing, setPairing] = useState<ActivePairing | null>(null);
  const [pairingStarting, setPairingStarting] = useState(false);
  const [pairingNotice, setPairingNotice] = useState<PairingNotice | null>(
    null,
  );
  const [remainingSeconds, setRemainingSeconds] = useState(0);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!pairing) return;

    let active = true;
    let checking = false;

    const checkPairing = async () => {
      const remaining = Math.max(
        0,
        Math.ceil((pairing.expiresAt - Date.now()) / 1000),
      );
      if (active) setRemainingSeconds(remaining);

      if (remaining === 0) {
        if (active) {
          setPairing(null);
          setPairingNotice({
            tone: "warning",
            title: "Pairing code expired",
            message: "Select Pair a device to create a new six-digit code.",
          });
        }
        return;
      }

      if (checking) return;
      checking = true;
      try {
        const isOpen = await pairingIsOpen();
        if (active && !isOpen) {
          setPairing(null);
          setPairingNotice({
            tone: "success",
            title: "Pairing window closed",
            message:
              "The device accepted the code, or the pairing request was closed. If it did not connect, create a new code and try again.",
          });
        }
      } catch (error: unknown) {
        if (active) {
          setPairing(null);
          setPairingNotice({
            tone: "error",
            title: "Pairing status could not be checked",
            message: `${errorMessage(error)} The code was canceled; select Pair a device to try again.`,
          });
        }
      } finally {
        checking = false;
      }
    };

    void checkPairing();
    const timer = window.setInterval(() => void checkPairing(), 1_000);
    return () => {
      active = false;
      window.clearInterval(timer);
      void cancelPairing().catch(() => undefined);
    };
  }, [pairing]);

  if (!draft) return null;

  if (!loadedIntegrations) {
    loadedIntegrations = { ...draft.general.integrations };
  }

  const updateGeneral = (
    update: (general: SettingsDraft["general"]) => SettingsDraft["general"],
  ) => {
    setDraft((current) =>
      current ? { ...current, general: update(current.general) } : current,
    );
  };

  const updateIntegrations = (update: Partial<IntegrationSettings>) => {
    setPairing(null);
    setPairingNotice(
      pairing
        ? {
            tone: "warning",
            title: "Pairing canceled",
            message:
              "Integration access changed. Save and restart before pairing again.",
          }
        : null,
    );
    updateGeneral((general) => ({
      ...general,
      integrations: { ...general.integrations, ...update },
    }));
  };

  const chooseDirectory = async () => {
    setChoosingDirectory(true);
    setDirectoryError(null);
    try {
      const directory = await chooseEqDirectory(draft.general.eqDirectory);
      if (directory && mounted.current) {
        updateGeneral((general) => ({ ...general, eqDirectory: directory }));
      }
    } catch (error: unknown) {
      if (mounted.current) {
        setDirectoryError(
          `The folder picker could not open: ${errorMessage(error)} Enter the EverQuest folder manually or try Browse again.`,
        );
      }
    } finally {
      if (mounted.current) setChoosingDirectory(false);
    }
  };

  const startPairing = async () => {
    setPairingStarting(true);
    setPairingNotice(null);
    try {
      const session = await beginPairing();
      if (!mounted.current) {
        await cancelPairing();
        return;
      }
      setRemainingSeconds(session.expiresInSeconds);
      setPairing({
        ...session,
        expiresAt: Date.now() + session.expiresInSeconds * 1_000,
      });
    } catch (error: unknown) {
      if (mounted.current) {
        setPairingNotice({
          tone: "error",
          title: "Pairing could not start",
          message: `${errorMessage(error)} Confirm local-network access is saved, restart Stonemite, and try again.`,
        });
      }
    } finally {
      if (mounted.current) setPairingStarting(false);
    }
  };

  const stopPairing = async () => {
    setPairing(null);
    try {
      await cancelPairing();
      if (mounted.current) {
        setPairingNotice({
          tone: "info",
          title: "Pairing canceled",
          message: "The six-digit code can no longer be used.",
        });
      }
    } catch (error: unknown) {
      if (mounted.current) {
        setPairingNotice({
          tone: "error",
          title: "Pairing might still be open",
          message: `${errorMessage(error)} Close Settings or restart Stonemite to cancel the code.`,
        });
      }
    }
  };

  const integrationsChanged = !sameIntegrations(
    loadedIntegrations,
    draft.general.integrations,
  );
  const pairingBlocked = dirty || integrationsChanged;
  const canPair =
    draft.general.integrations.enabled &&
    draft.general.integrations.lanEnabled &&
    !pairingBlocked &&
    !pairingStarting &&
    !pairing;

  let pairingBlockedCopy: string | null = null;
  if (integrationsChanged && dirty) {
    pairingBlockedCopy =
      "Save these integration changes, then restart Stonemite before pairing a device.";
  } else if (integrationsChanged) {
    pairingBlockedCopy =
      "Integration changes are saved. Restart Stonemite before pairing a device.";
  } else if (dirty) {
    pairingBlockedCopy =
      "Save or discard your other settings changes before pairing a device.";
  }

  return (
    <SettingsPage
      title="General"
      description="Core EverQuest, integration, notification, and update behavior."
    >
      <FormSection
        title="EverQuest directory"
        description="The folder containing your EverQuest installation."
      >
        <Field label="Installation folder" htmlFor={directoryId}>
          <div className="directory-control">
            <TextInput
              id={directoryId}
              value={draft.general.eqDirectory}
              spellCheck={false}
              onChange={(event) =>
                updateGeneral((general) => ({
                  ...general,
                  eqDirectory: event.currentTarget.value,
                }))
              }
            />
            <Button
              onClick={() => void chooseDirectory()}
              disabled={choosingDirectory}
            >
              {choosingDirectory ? "Opening…" : "Browse…"}
            </Button>
          </div>
        </Field>
        {directoryError ? (
          <p className="general-error" role="alert">
            {directoryError}
          </p>
        ) : null}
      </FormSection>

      <FormSection title="EQ windows">
        <Toggle
          label="Hide from Alt-Tab"
          description="Keep EverQuest windows out of the Windows Alt-Tab switcher."
          checked={draft.general.hideFromAltTab}
          onChange={(hideFromAltTab) =>
            updateGeneral((general) => ({ ...general, hideFromAltTab }))
          }
        />
      </FormSection>

      <FormSection
        title="Integrations"
        description="Allow trusted apps such as Stream Deck plugins to control Stonemite."
      >
        <Toggle
          label="Enable integrations"
          checked={draft.general.integrations.enabled}
          onChange={(enabled) => updateIntegrations({ enabled })}
        />

        <fieldset
          className="integration-access"
          disabled={!draft.general.integrations.enabled}
        >
          <legend>Access</legend>
          <label>
            <input
              type="radio"
              name="integration-access"
              checked={!draft.general.integrations.lanEnabled}
              onChange={() => updateIntegrations({ lanEnabled: false })}
            />
            <span>
              <strong>This PC only</strong>
              <small>Only apps running on this computer can connect.</small>
            </span>
          </label>
          <label>
            <input
              type="radio"
              name="integration-access"
              checked={draft.general.integrations.lanEnabled}
              onChange={() => updateIntegrations({ lanEnabled: true })}
            />
            <span>
              <strong>Devices on my local network</strong>
              <small>Allow trusted devices on your private network.</small>
            </span>
          </label>
        </fieldset>

        {draft.general.integrations.enabled &&
        draft.general.integrations.lanEnabled ? (
          <div className="pairing-area">
            <p className="help-text">
              Only pair devices you trust on a private network. On first
              restart, allow <strong>Private networks</strong> if Windows asks.
            </p>

            <div className="pairing-action">
              {pairing ? (
                <Button onClick={() => void stopPairing()}>
                  Cancel pairing
                </Button>
              ) : (
                <Button
                  variant="primary"
                  disabled={!canPair}
                  onClick={() => void startPairing()}
                >
                  {pairingStarting ? "Creating code…" : "Pair a device"}
                </Button>
              )}
              {pairingBlockedCopy ? (
                <p className="pairing-blocked">{pairingBlockedCopy}</p>
              ) : null}
            </div>

            {pairing ? (
              <div
                className="pairing-session"
                role="status"
                aria-live="polite"
                aria-label={`Pairing code ${pairing.code}. Expires in ${formatCountdown(remainingSeconds)}.`}
              >
                <span>
                  In Stream Deck, connect to <strong>{pairing.address}</strong>{" "}
                  and enter:
                </span>
                <strong className="pairing-code" aria-hidden="true">
                  {pairing.code}
                </strong>
                <span className="pairing-countdown">
                  Expires in {formatCountdown(remainingSeconds)}
                </span>
              </div>
            ) : null}

            {pairingNotice ? (
              <div className="general-inline-status">
                <InlineStatus
                  tone={pairingNotice.tone}
                  title={pairingNotice.title}
                >
                  {pairingNotice.message}
                </InlineStatus>
              </div>
            ) : null}
          </div>
        ) : null}

        <p className="help-text">
          Changes to integration enablement or access require a restart.
          {runtime?.integrationAddress
            ? ` Current integration address: ${runtime.integrationAddress}.`
            : ""}
        </p>
      </FormSection>

      <FormSection
        title="Toast notifications"
        description="Control the compact notifications shown by Stonemite."
      >
        <Toggle
          label="Enabled"
          checked={draft.general.toast.enabled}
          onChange={(enabled) =>
            updateGeneral((general) => ({
              ...general,
              toast: { ...general.toast, enabled },
            }))
          }
        />
        <fieldset
          className="general-dependent-controls"
          disabled={!draft.general.toast.enabled}
          aria-label="Toast notification appearance"
        >
          <Field label="Height">
            <RangeInput
              value={draft.general.toast.height}
              min={24}
              max={128}
              suffix=" px"
              ariaLabel="Toast notification height"
              onChange={(height) =>
                updateGeneral((general) => ({
                  ...general,
                  toast: { ...general.toast, height },
                }))
              }
            />
          </Field>
          <Field label="Duration">
            <RangeInput
              value={draft.general.toast.durationSeconds}
              min={0.5}
              max={10}
              step={0.1}
              suffix=" s"
              ariaLabel="Toast notification duration"
              onChange={(durationSeconds) =>
                updateGeneral((general) => ({
                  ...general,
                  toast: { ...general.toast, durationSeconds },
                }))
              }
            />
          </Field>
        </fieldset>
      </FormSection>

      <FormSection
        title="Updates"
        description="Choose how often Stonemite checks for a new release."
      >
        <Toggle
          label="Check automatically on launch"
          checked={draft.general.updates.automatic}
          onChange={(automatic) =>
            updateGeneral((general) => ({
              ...general,
              updates: { ...general.updates, automatic },
            }))
          }
        />
        <fieldset
          className="general-dependent-controls"
          disabled={!draft.general.updates.automatic}
          aria-label="Automatic update check frequency"
        >
          <Field label="Check every">
            <RangeInput
              value={draft.general.updates.intervalDays}
              min={1}
              max={30}
              suffix={
                draft.general.updates.intervalDays === 1 ? " day" : " days"
              }
              ariaLabel="Automatic update check interval"
              onChange={(intervalDays) =>
                updateGeneral((general) => ({
                  ...general,
                  updates: { ...general.updates, intervalDays },
                }))
              }
            />
          </Field>
        </fieldset>
      </FormSection>
    </SettingsPage>
  );
}
