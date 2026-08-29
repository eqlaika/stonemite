import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";

import { Button, CheckboxOption } from "../../components/Controls";
import { newId } from "../../triggers/model";
import type { Profile, TriggerLibrary } from "../../triggers/types";

export function ProfilesPanel({
  library,
  onChange,
}: {
  library: TriggerLibrary;
  onChange: (library: TriggerLibrary) => void;
}) {
  const [openId, setOpenId] = useState<string | null>(null);

  const updateProfile = (profile: Profile) =>
    onChange({
      ...library,
      profiles: library.profiles.map((existing) =>
        existing.id === profile.id ? profile : existing,
      ),
    });

  return (
    <div className="tw-side-panel">
      <p className="tw-panel-hint">
        Profiles choose which triggers run and for whom. With no profiles, every
        enabled trigger runs on every character.
      </p>
      {library.profiles.map((profile) => (
        <details
          key={profile.id}
          open={openId === profile.id}
          onToggle={(event) =>
            setOpenId(event.currentTarget.open ? profile.id : null)
          }
        >
          <summary>
            {profile.name || "(unnamed profile)"}
            {profile.enabled ? "" : " (off)"}
          </summary>
          <label>
            <span>Name</span>
            <input
              value={profile.name}
              onChange={(event) =>
                updateProfile({ ...profile, name: event.target.value })
              }
            />
          </label>
          <CheckboxOption
            label="Profile enabled"
            checked={profile.enabled}
            onChange={(enabled) => updateProfile({ ...profile, enabled })}
          />
          <label>
            <span>Applies to</span>
            <select
              value={profile.assignment.kind}
              onChange={(event) =>
                updateProfile({
                  ...profile,
                  assignment:
                    event.target.value === "global"
                      ? { kind: "global" }
                      : { kind: "characters", characters: [] },
                })
              }
            >
              <option value="global">Every character</option>
              <option value="characters">Selected characters</option>
            </select>
          </label>
          {profile.assignment.kind === "characters" ? (
            <CharacterListEditor profile={profile} onChange={updateProfile} />
          ) : null}
          <fieldset className="tw-profile-folders">
            <legend>Enabled folders</legend>
            {library.folders.length === 0 ? (
              <p className="tw-panel-hint">No folders yet.</p>
            ) : (
              library.folders.map((folder) => (
                <CheckboxOption
                  key={folder.id}
                  label={folder.name}
                  checked={profile.folders.includes(folder.id)}
                  onChange={(checked) =>
                    updateProfile({
                      ...profile,
                      folders: checked
                        ? [...profile.folders, folder.id]
                        : profile.folders.filter((id) => id !== folder.id),
                    })
                  }
                />
              ))
            )}
          </fieldset>
          <p className="tw-panel-hint">
            {profile.triggers.length} individually selected trigger(s). Use the
            list's bulk bar to add the current selection.
          </p>
          <Button
            onClick={() =>
              onChange({
                ...library,
                profiles: library.profiles.map((existing) =>
                  existing.id === profile.id
                    ? {
                        ...existing,
                        triggers: library.triggers.map((trigger) => trigger.id),
                      }
                    : existing,
                ),
              })
            }
          >
            Include every trigger
          </Button>
          <Button
            variant="danger"
            onClick={() => {
              if (window.confirm(`Delete profile "${profile.name}"?`)) {
                onChange({
                  ...library,
                  profiles: library.profiles.filter(
                    (existing) => existing.id !== profile.id,
                  ),
                });
              }
            }}
          >
            <Trash2 size={13} aria-hidden="true" /> Delete profile
          </Button>
        </details>
      ))}
      <Button
        onClick={() =>
          onChange({
            ...library,
            profiles: [
              ...library.profiles,
              {
                id: newId(),
                name: `Profile ${library.profiles.length + 1}`,
                enabled: true,
                assignment: { kind: "global" },
                triggers: library.triggers.map((trigger) => trigger.id),
                folders: [],
                voice: null,
                voiceRate: 0,
                volume: 100,
              },
            ],
          })
        }
      >
        <Plus size={14} aria-hidden="true" /> New profile
      </Button>
    </div>
  );
}

function CharacterListEditor({
  profile,
  onChange,
}: {
  profile: Profile;
  onChange: (profile: Profile) => void;
}) {
  const [character, setCharacter] = useState("");
  const [server, setServer] = useState("");
  if (profile.assignment.kind !== "characters") return null;
  const characters = profile.assignment.characters;

  return (
    <div className="tw-character-list">
      <ul>
        {characters.map((selector, index) => (
          <li key={index}>
            {selector.character}
            {selector.server ? ` (${selector.server})` : ""}
            <button
              type="button"
              className="tw-icon-button"
              aria-label={`Remove ${selector.character}`}
              onClick={() =>
                onChange({
                  ...profile,
                  assignment: {
                    kind: "characters",
                    characters: characters.filter(
                      (_, other) => other !== index,
                    ),
                  },
                })
              }
            >
              <Trash2 size={12} aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>
      <div className="tw-character-add">
        <input
          placeholder="Character"
          aria-label="Character name"
          value={character}
          onChange={(event) => setCharacter(event.target.value)}
        />
        <input
          placeholder="Server (optional)"
          aria-label="Server"
          value={server}
          onChange={(event) => setServer(event.target.value)}
        />
        <Button
          disabled={!character.trim()}
          onClick={() => {
            onChange({
              ...profile,
              assignment: {
                kind: "characters",
                characters: [
                  ...characters,
                  { character: character.trim(), server: server.trim() },
                ],
              },
            });
            setCharacter("");
            setServer("");
          }}
        >
          Add
        </Button>
      </div>
    </div>
  );
}
