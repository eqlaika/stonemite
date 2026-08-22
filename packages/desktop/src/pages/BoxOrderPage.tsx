import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  GripVertical,
  Search,
  Trash2,
} from "lucide-react";
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
} from "react";

import { Button, FormSection, TextInput } from "../components/Controls";
import { SettingsPage } from "../components/SettingsPage";
import { loadRunningCharacters } from "../settings/api";
import { useSettings } from "../settings/SettingsContext";
import type { BoxIdentity, RunningCharacter } from "../settings/types";
import "./BoxOrderPage.css";

function identityKey(identity: BoxIdentity): string {
  return `${identity.server.trim().toLowerCase()}\u001f${identity.character
    .trim()
    .toLowerCase()}`;
}

function sameIdentity(left: BoxIdentity, right: BoxIdentity): boolean {
  return identityKey(left) === identityKey(right);
}

function identityLabel(identity: BoxIdentity): string {
  return `${identity.character} — ${identity.server}`;
}

export function moveBoxIdentity(
  identities: BoxIdentity[],
  fromIndex: number,
  toIndex: number,
): BoxIdentity[] {
  if (
    fromIndex === toIndex ||
    fromIndex < 0 ||
    toIndex < 0 ||
    fromIndex >= identities.length ||
    toIndex >= identities.length
  ) {
    return identities;
  }
  const next = [...identities];
  const [moved] = next.splice(fromIndex, 1);
  if (!moved) return identities;
  next.splice(toIndex, 0, moved);
  return next;
}

type CharacterSource = "running" | "known";

interface CharacterOption {
  identity: BoxIdentity;
  source: CharacterSource;
  windowNumber: number | null;
}

function characterOptions(
  value: BoxIdentity[],
  runningCharacters: RunningCharacter[],
  knownCharacters: BoxIdentity[],
): CharacterOption[] {
  const listed = new Set(value.map(identityKey));
  const seenRunning = new Set<string>();
  const running = runningCharacters
    .filter((identity) => {
      const key = identityKey(identity);
      if (listed.has(key) || seenRunning.has(key)) return false;
      seenRunning.add(key);
      return true;
    })
    .sort(
      (left, right) =>
        (left.windowNumber ?? Number.MAX_SAFE_INTEGER) -
          (right.windowNumber ?? Number.MAX_SAFE_INTEGER) ||
        left.character.localeCompare(right.character, undefined, {
          sensitivity: "base",
        }) ||
        left.server.localeCompare(right.server, undefined, {
          sensitivity: "base",
        }),
    )
    .map<CharacterOption>((identity) => ({
      identity,
      source: "running",
      windowNumber: identity.windowNumber,
    }));

  const runningKeys = new Set(
    running.map(({ identity }) => identityKey(identity)),
  );
  const seenKnown = new Set<string>();
  const known = knownCharacters
    .filter((identity) => {
      const key = identityKey(identity);
      if (listed.has(key) || runningKeys.has(key) || seenKnown.has(key)) {
        return false;
      }
      seenKnown.add(key);
      return true;
    })
    .sort(
      (left, right) =>
        left.server.localeCompare(right.server, undefined, {
          sensitivity: "base",
        }) ||
        left.character.localeCompare(right.character, undefined, {
          sensitivity: "base",
        }),
    )
    .map<CharacterOption>((identity) => ({
      identity,
      source: "known",
      windowNumber: null,
    }));

  return [...running, ...known];
}

interface CharacterComboboxProps {
  id: string;
  options: CharacterOption[];
  value: BoxIdentity | null;
  onChange: (identity: BoxIdentity | null) => void;
}

function CharacterCombobox({
  id,
  options,
  value,
  onChange,
}: CharacterComboboxProps) {
  const listboxId = `${id}-listbox`;
  const runningHeadingId = `${id}-running-heading`;
  const knownHeadingId = `${id}-known-heading`;
  const inputRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState(value ? identityLabel(value) : "");
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);

  useEffect(() => {
    if (!value) setQuery("");
  }, [value]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle || (value && query === identityLabel(value))) return options;
    return options.filter(({ identity, windowNumber }) =>
      `${identity.character} ${identity.server} ${
        windowNumber === null ? "" : `box ${windowNumber}`
      }`
        .toLowerCase()
        .includes(needle),
    );
  }, [options, query, value]);

  useEffect(() => {
    setActiveIndex((current) =>
      filtered.length === 0 ? 0 : Math.min(current, filtered.length - 1),
    );
  }, [filtered.length]);

  const running = filtered.filter((option) => option.source === "running");
  const known = filtered.filter((option) => option.source === "known");
  const selectedKey = value ? identityKey(value) : null;
  const activeOption = filtered[activeIndex];

  const choose = (option: CharacterOption) => {
    onChange(option.identity);
    setQuery(identityLabel(option.identity));
    setOpen(false);
    inputRef.current?.focus();
  };

  const openList = () => {
    if (options.length === 0) return;
    if (value) setQuery("");
    setOpen(true);
    setActiveIndex(0);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (!open) openList();
      else if (filtered.length > 0)
        setActiveIndex((current) => (current + 1) % filtered.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) openList();
      else if (filtered.length > 0)
        setActiveIndex(
          (current) => (current - 1 + filtered.length) % filtered.length,
        );
    } else if (event.key === "Home" && open && filtered.length > 0) {
      event.preventDefault();
      setActiveIndex(0);
    } else if (event.key === "End" && open && filtered.length > 0) {
      event.preventDefault();
      setActiveIndex(filtered.length - 1);
    } else if (event.key === "Enter") {
      if (!open) {
        event.preventDefault();
        openList();
      } else if (activeOption) {
        event.preventDefault();
        choose(activeOption);
      }
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      setOpen(false);
      setQuery(value ? identityLabel(value) : "");
    }
  };

  const renderOption = (option: CharacterOption, index: number) => {
    const key = identityKey(option.identity);
    const optionId = `${listboxId}-option-${index}`;
    return (
      <button
        id={optionId}
        key={`${option.source}-${key}`}
        type="button"
        role="option"
        className={`character-combobox-option${
          activeIndex === index ? " active" : ""
        }`}
        aria-selected={selectedKey === key}
        onMouseDown={(event) => event.preventDefault()}
        onMouseEnter={() => setActiveIndex(index)}
        onClick={() => choose(option)}
      >
        <span className="character-combobox-copy">
          <strong>{option.identity.character}</strong>
          <span>{option.identity.server}</span>
        </span>
        {option.source === "running" ? (
          <span className="character-combobox-status">
            {option.windowNumber === null
              ? "Running"
              : `Box ${option.windowNumber}`}
          </span>
        ) : null}
      </button>
    );
  };

  return (
    <div
      className="character-combobox"
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          setOpen(false);
          setQuery(value ? identityLabel(value) : "");
        }
      }}
    >
      <div className="character-combobox-input-wrap">
        <Search size={15} aria-hidden="true" />
        <input
          ref={inputRef}
          id={id}
          className="text-input character-combobox-input"
          type="text"
          role="combobox"
          value={query}
          placeholder={
            options.length === 0
              ? "No unlisted characters"
              : "Search characters or servers…"
          }
          autoComplete="off"
          disabled={options.length === 0}
          aria-autocomplete="list"
          aria-expanded={open}
          aria-controls={listboxId}
          aria-activedescendant={
            open && activeOption
              ? `${listboxId}-option-${activeIndex}`
              : undefined
          }
          onFocus={openList}
          onChange={(event) => {
            setQuery(event.currentTarget.value);
            onChange(null);
            setOpen(true);
            setActiveIndex(0);
          }}
          onKeyDown={handleKeyDown}
        />
        <button
          type="button"
          className="character-combobox-toggle"
          aria-label={
            open ? "Close character choices" : "Open character choices"
          }
          aria-expanded={open}
          disabled={options.length === 0}
          tabIndex={-1}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            if (open) setOpen(false);
            else {
              openList();
              inputRef.current?.focus();
            }
          }}
        >
          <ChevronDown size={15} aria-hidden="true" />
        </button>
      </div>

      {open ? (
        <div
          id={listboxId}
          className="character-combobox-listbox"
          role="listbox"
          aria-label="Detected characters"
        >
          {filtered.length === 0 ? (
            <p className="character-combobox-empty">No matching characters</p>
          ) : null}
          {running.length > 0 ? (
            <div role="group" aria-labelledby={runningHeadingId}>
              <div id={runningHeadingId} className="character-combobox-heading">
                Running now
              </div>
              {running.map((option, index) => renderOption(option, index))}
            </div>
          ) : null}
          {known.length > 0 ? (
            <div role="group" aria-labelledby={knownHeadingId}>
              <div id={knownHeadingId} className="character-combobox-heading">
                Known characters
              </div>
              {known.map((option, index) =>
                renderOption(option, running.length + index),
              )}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

interface BoxOrderEditorProps {
  value: BoxIdentity[];
  knownCharacters: BoxIdentity[];
  runningCharacters?: RunningCharacter[];
  onChange: (value: BoxIdentity[]) => void;
}

export function BoxOrderEditor({
  value,
  knownCharacters,
  runningCharacters = [],
  onChange,
}: BoxOrderEditorProps) {
  const characterId = useId();
  const serverId = useId();
  const detectedId = useId();
  const [selectedCharacter, setSelectedCharacter] =
    useState<BoxIdentity | null>(null);
  const [manualCharacter, setManualCharacter] = useState("");
  const [manualServer, setManualServer] = useState("");
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);
  const [announcement, setAnnouncement] = useState("");

  const availableOptions = useMemo(
    () => characterOptions(value, runningCharacters, knownCharacters),
    [knownCharacters, runningCharacters, value],
  );

  const manualCandidate = {
    character: manualCharacter.trim(),
    server: manualServer.trim(),
  };
  const manualComplete = Boolean(
    manualCandidate.character && manualCandidate.server,
  );
  const manualDuplicate =
    manualComplete &&
    value.some((identity) => sameIdentity(identity, manualCandidate));

  const commit = (next: BoxIdentity[], message: string) => {
    onChange(next);
    setAnnouncement(message);
  };

  const move = (fromIndex: number, toIndex: number) => {
    const next = moveBoxIdentity(value, fromIndex, toIndex);
    if (next === value) return;
    const identity = value[fromIndex];
    if (!identity) return;
    commit(next, `Moved ${identity.character} to box ${toIndex + 1}.`);
  };

  const remove = (index: number) => {
    const identity = value[index];
    if (!identity) return;
    commit(
      value.filter((_candidate, candidateIndex) => candidateIndex !== index),
      `Removed ${identity.character} from the preferred box order.`,
    );
  };

  const addSelected = () => {
    if (!selectedCharacter) return;
    commit(
      [...value, selectedCharacter],
      `Added ${selectedCharacter.character} as box ${value.length + 1}.`,
    );
    setSelectedCharacter(null);
  };

  const addManual = (event: FormEvent) => {
    event.preventDefault();
    if (!manualComplete || manualDuplicate) return;
    commit(
      [...value, manualCandidate],
      `Added ${manualCandidate.character} as box ${value.length + 1}.`,
    );
    setManualCharacter("");
    setManualServer("");
  };

  const startDrag = (event: DragEvent<HTMLLIElement>, index: number) => {
    setDraggedIndex(index);
    setDropIndex(index);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", String(index));
  };

  const drop = (event: DragEvent<HTMLLIElement>, index: number) => {
    event.preventDefault();
    if (draggedIndex !== null) move(draggedIndex, index);
    setDraggedIndex(null);
    setDropIndex(null);
  };

  return (
    <>
      <FormSection
        title="Preferred order"
        description="Configured characters receive the lowest available box numbers. Missing characters are skipped; unlisted characters follow."
      >
        {value.length === 0 ? (
          <div className="box-order-empty">
            <strong>No preferred order</strong>
            <p>Windows keep the order in which Stonemite discovers them.</p>
          </div>
        ) : (
          <ol className="box-order-list" aria-label="Preferred box order">
            {value.map((identity, index) => (
              <li
                className={`box-order-row${
                  draggedIndex === index ? " dragging" : ""
                }${dropIndex === index ? " drop-target" : ""}`}
                key={`${identityKey(identity)}-${index}`}
                draggable
                onDragStart={(event) => startDrag(event, index)}
                onDragOver={(event) => {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  setDropIndex(index);
                }}
                onDrop={(event) => drop(event, index)}
                onDragEnd={() => {
                  setDraggedIndex(null);
                  setDropIndex(null);
                }}
              >
                <span className="box-order-grip" title="Drag to reorder">
                  <GripVertical size={16} aria-hidden="true" />
                </span>
                <span
                  className="box-order-number"
                  aria-label={`Box ${index + 1}`}
                >
                  {index + 1}
                </span>
                <span className="box-order-identity">
                  <strong>{identity.character}</strong>
                  <span>{identity.server}</span>
                </span>
                <span className="box-order-actions">
                  <Button
                    type="button"
                    variant="quiet"
                    className="box-order-action"
                    aria-label={`Move ${identity.character} up`}
                    disabled={index === 0}
                    onClick={() => move(index, index - 1)}
                  >
                    <ArrowUp size={15} aria-hidden="true" />
                  </Button>
                  <Button
                    type="button"
                    variant="quiet"
                    className="box-order-action"
                    aria-label={`Move ${identity.character} down`}
                    disabled={index === value.length - 1}
                    onClick={() => move(index, index + 1)}
                  >
                    <ArrowDown size={15} aria-hidden="true" />
                  </Button>
                  <Button
                    type="button"
                    variant="quiet"
                    className="box-order-action box-order-remove"
                    aria-label={`Remove ${identity.character}`}
                    onClick={() => remove(index)}
                  >
                    <Trash2 size={15} aria-hidden="true" />
                  </Button>
                </span>
              </li>
            ))}
          </ol>
        )}
        <p className="box-order-announcement" aria-live="polite">
          {announcement}
        </p>
      </FormSection>

      <FormSection
        title="Add character"
        description="Search characters running now or identities Stonemite remembers from previous logins."
      >
        <div className="box-order-known-control">
          <label htmlFor={detectedId}>Detected character</label>
          <div className="box-order-add-row">
            <CharacterCombobox
              id={detectedId}
              options={availableOptions}
              value={selectedCharacter}
              onChange={setSelectedCharacter}
            />
            <Button
              type="button"
              aria-label="Add detected character"
              disabled={!selectedCharacter}
              onClick={addSelected}
            >
              Add
            </Button>
          </div>
        </div>

        <div className="box-order-divider" aria-hidden="true">
          <span>or enter manually</span>
        </div>

        <form className="box-order-manual" onSubmit={addManual}>
          <div>
            <label htmlFor={characterId}>Character</label>
            <TextInput
              id={characterId}
              value={manualCharacter}
              maxLength={128}
              autoCapitalize="none"
              spellCheck={false}
              onChange={(event) =>
                setManualCharacter(event.currentTarget.value)
              }
            />
          </div>
          <div>
            <label htmlFor={serverId}>Server</label>
            <TextInput
              id={serverId}
              value={manualServer}
              maxLength={128}
              autoCapitalize="none"
              spellCheck={false}
              onChange={(event) => setManualServer(event.currentTarget.value)}
            />
          </div>
          <Button
            type="submit"
            aria-label="Add manually entered character"
            disabled={!manualComplete || manualDuplicate}
          >
            Add
          </Button>
        </form>
        {manualDuplicate ? (
          <p className="box-order-error" role="alert">
            That character is already in the preferred order.
          </p>
        ) : null}
      </FormSection>
    </>
  );
}

export function BoxOrderPage() {
  const { draft, setDraft, options } = useSettings();
  const [runningCharacters, setRunningCharacters] = useState<
    RunningCharacter[]
  >([]);

  useEffect(() => {
    let active = true;
    const refresh = () => {
      void loadRunningCharacters()
        .then((characters) => {
          if (active) setRunningCharacters(characters);
        })
        .catch(() => {
          // Keep cached choices usable if the tray is restarting.
        });
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => {
      active = false;
      window.removeEventListener("focus", refresh);
    };
  }, []);

  if (!draft || !options) return null;

  return (
    <SettingsPage
      title="Box order"
      description="Keep detected characters in consistent numbered boxes, independently of login accounts or launch order."
    >
      <BoxOrderEditor
        value={draft.boxOrder}
        knownCharacters={options.knownCharacters}
        runningCharacters={runningCharacters}
        onChange={(boxOrder) =>
          setDraft((current) => (current ? { ...current, boxOrder } : current))
        }
      />
    </SettingsPage>
  );
}
