import { useEffect, useRef, useState } from "react";

import {
  Button,
  CheckboxOption,
  Field,
  FormSection,
  RangeInput,
  SelectInput,
  Toggle,
} from "../components/Controls";
import { SettingsPage } from "../components/SettingsPage";
import { previewNotificationSound } from "../settings/api";
import { useSettings } from "../settings/SettingsContext";
import type { NotificationSettings, SettingsDraft } from "../settings/types";
import "./NotificationsPage.css";

type EventSetting =
  | "tells"
  | "groupInvites"
  | "raidInvites"
  | "tradeProposals"
  | "resurrections"
  | "deaths";

type PreviewStatus =
  | { state: "idle" }
  | { state: "playing"; message: string }
  | { state: "success"; message: string }
  | { state: "error"; message: string };

const NOTIFICATION_EVENTS: ReadonlyArray<{
  setting: EventSetting;
  label: string;
}> = [
  { setting: "tells", label: "Tells" },
  { setting: "groupInvites", label: "Group invites" },
  { setting: "raidInvites", label: "Raid invites" },
  { setting: "tradeProposals", label: "Trade requests" },
  { setting: "resurrections", label: "Resurrection offers" },
  { setting: "deaths", label: "Character deaths" },
];

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function CombatAwarenessControls({
  value,
  onChange,
}: {
  value: NotificationSettings;
  onChange: (update: Partial<NotificationSettings>) => void;
}) {
  return (
    <>
      <Toggle
        label="Show combat awareness"
        description="Use red and white hit frames, blood-red damage feedback, and persistent problem states on background PiPs."
        checked={value.combatAwarenessEnabled}
        onChange={(combatAwarenessEnabled) =>
          onChange({ combatAwarenessEnabled })
        }
      />
      <fieldset
        className="combat-awareness-controls"
        disabled={!value.combatAwarenessEnabled}
        aria-label="Combat awareness timing"
      >
        <Field
          label="Attack highlight duration"
          description="How long a successful melee or archery hit keeps the red combat frame visible."
          descriptionId="combat-hit-duration-help"
        >
          <RangeInput
            value={value.combatHitDurationSeconds}
            min={0.5}
            max={10}
            step={0.5}
            suffix=" s"
            ariaLabel="Attack highlight duration"
            ariaDescribedBy="combat-hit-duration-help"
            onChange={(combatHitDurationSeconds) =>
              onChange({ combatHitDurationSeconds })
            }
          />
        </Field>
      </fieldset>
      <p className="help-text">
        Range and line-of-sight warnings clear after a successful hit or a short
        quiet period. Melody and death clear when EverQuest logs recovery.
        Combat awareness never plays notification sounds.
      </p>
    </>
  );
}

export function NotificationsPage() {
  const { draft, setDraft, options } = useSettings();
  const mounted = useRef(true);
  const previewRequest = useRef(0);
  const [previewStatus, setPreviewStatus] = useState<PreviewStatus>({
    state: "idle",
  });

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      previewRequest.current += 1;
    };
  }, []);

  if (!draft || !options) return null;

  const updateNotifications = (
    update: (
      notifications: NotificationSettings,
    ) => SettingsDraft["notifications"],
  ) => {
    setDraft((current) =>
      current
        ? {
            ...current,
            notifications: update(current.notifications),
          }
        : current,
    );
  };

  const previewSound = async () => {
    const sound = draft.notifications.sound;
    const request = ++previewRequest.current;
    setPreviewStatus({
      state: "playing",
      message: "Playing notification sound preview…",
    });
    try {
      await previewNotificationSound(sound);
      if (mounted.current && previewRequest.current === request) {
        setPreviewStatus({
          state: "success",
          message: "Notification sound preview played.",
        });
      }
    } catch (error: unknown) {
      if (mounted.current && previewRequest.current === request) {
        setPreviewStatus({
          state: "error",
          message: `Notification sound preview failed: ${errorMessage(error)}`,
        });
      }
    }
  };

  const soundControlsDisabled = !draft.notifications.soundEnabled;

  return (
    <SettingsPage
      title="Notifications"
      description="Choose how background boxes show combat and notable events."
    >
      <FormSection
        title="Visual highlight"
        description="Draw attention to activity in a background PiP."
      >
        <Toggle
          label="Highlight the background PiP"
          description="Show the latest event briefly, then leave its border and one dot per unread event until that box is activated."
          checked={draft.notifications.visualEnabled}
          onChange={(visualEnabled) =>
            updateNotifications((notifications) => ({
              ...notifications,
              visualEnabled,
            }))
          }
        />
      </FormSection>

      <FormSection
        title="Combat awareness"
        description="Show immediate combat activity and problems without creating unread notifications."
      >
        <CombatAwarenessControls
          value={draft.notifications}
          onChange={(update) =>
            updateNotifications((notifications) => ({
              ...notifications,
              ...update,
            }))
          }
        />
      </FormSection>

      <FormSection
        title="Events"
        description="Choose which EverQuest events trigger enabled visual and sound notifications."
      >
        <fieldset className="notification-events">
          <legend>Notify for</legend>
          <div className="notification-event-list">
            {NOTIFICATION_EVENTS.map(({ setting, label }) => (
              <CheckboxOption
                key={setting}
                label={label}
                checked={draft.notifications[setting]}
                onChange={(checked) =>
                  updateNotifications((notifications) => ({
                    ...notifications,
                    [setting]: checked,
                  }))
                }
              />
            ))}
          </div>
        </fieldset>
        <p className="help-text">
          Chat events use the character&apos;s configured EverQuest color.
          Trade, resurrection, and death use Stonemite status colors.
        </p>
      </FormSection>

      <FormSection
        title="Sound"
        description="Play bundled EverQuest audio-trigger sounds for selected events."
      >
        <Toggle
          label="Play a sound"
          checked={draft.notifications.soundEnabled}
          onChange={(soundEnabled) => {
            previewRequest.current += 1;
            setPreviewStatus({ state: "idle" });
            updateNotifications((notifications) => ({
              ...notifications,
              soundEnabled,
            }));
          }}
        />
        <div
          className={`notification-sound-controls${soundControlsDisabled ? " is-disabled" : ""}`}
        >
          <Field label="Sound" htmlFor="notification-sound">
            <div className="notification-sound-picker">
              <SelectInput
                id="notification-sound"
                value={draft.notifications.sound}
                options={options.notificationSounds}
                disabled={soundControlsDisabled}
                onChange={(event) => {
                  previewRequest.current += 1;
                  setPreviewStatus({ state: "idle" });
                  updateNotifications((notifications) => ({
                    ...notifications,
                    sound: event.currentTarget.value,
                  }));
                }}
              />
              <Button
                disabled={
                  soundControlsDisabled || previewStatus.state === "playing"
                }
                onClick={() => void previewSound()}
              >
                {previewStatus.state === "playing" ? "Playing…" : "Preview"}
              </Button>
            </div>
          </Field>
          {soundControlsDisabled ? (
            <p className="notification-dependency-note">
              Turn on Play a sound to choose or preview a sound.
            </p>
          ) : null}
        </div>

        <div
          className={`notification-preview-status preview-${previewStatus.state}`}
          role={previewStatus.state === "error" ? "alert" : "status"}
          aria-live={previewStatus.state === "error" ? "assertive" : "polite"}
          aria-atomic="true"
        >
          {previewStatus.state === "idle" ? "" : previewStatus.message}
        </div>

        <p className="help-text">
          Sounds use EverQuest&apos;s bundled default audio-trigger files. The
          active box plays sound only.
        </p>
      </FormSection>
    </SettingsPage>
  );
}
