(() => {
  const client = SDPIComponents.streamDeckClient;
  const DEFAULT_HOTKEY_COLOR = "#59d8d0";
  const iconCatalog = window.IKKINZ_LUCIDE_ANIMATED_ICONS || {};
  const iconNames = Object.keys(iconCatalog).sort();
  const iconTimers = new WeakMap();
  const reducedMotion = window.matchMedia(
    "(prefers-reduced-motion: reduce)",
  ).matches;
  const form = document.querySelector("#hotkey-form");
  const targetModes = [
    ...document.querySelectorAll('input[name="target-mode"]'),
  ];
  const boxGrid = document.querySelector("#box-grid");
  const targetError = document.querySelector("#target-error");
  const mappingSearch = document.querySelector("#mapping-search");
  const mappingSelect = document.querySelector("#mapping");
  const mappingStatus = document.querySelector("#mapping-status");
  const mappingError = document.querySelector("#mapping-error");
  const refreshMappings = document.querySelector("#refresh-mappings");
  const tileLabel = document.querySelector("#tile-label");
  const colorSwatches = [
    ...document.querySelectorAll("#color-swatches [data-color]"),
  ];
  const tileColor = document.querySelector("#tile-color");
  const colorValue = document.querySelector("#color-value");
  const iconSearch = document.querySelector("#icon-search");
  const iconGrid = document.querySelector("#icon-grid");
  const iconEmpty = document.querySelector("#icon-empty");
  const selectedIcon = document.querySelector("#selected-icon");
  const selectedIconName = document.querySelector("#selected-icon-name");
  const moreIcons = document.querySelector("#more-icons");
  const formError = document.querySelector("#form-error");
  const save = document.querySelector("#save");
  const saveStatus = document.querySelector("#save-status");

  let draft = {
    targetMode: "all",
    windowNumbers: [1],
    mapping: "",
    label: "",
    icon: "keyboard",
    color: DEFAULT_HOTKEY_COLOR,
  };
  let boxes = [];
  let mappings = [];
  let settingsInitialization;
  let requestSequence = 0;
  let latestRequestId = "";
  let mappingBusy = false;
  let mappingValidated = false;
  let saving = false;
  let iconLimit = 72;

  function applySettings(settings) {
    draft = {
      targetMode: settings?.targetMode === "selected" ? "selected" : "all",
      windowNumbers: normalizeWindowNumbers(settings?.windowNumbers),
      mapping: typeof settings?.mapping === "string" ? settings.mapping : "",
      label: typeof settings?.label === "string" ? settings.label : "",
      icon:
        typeof settings?.icon === "string" && iconCatalog[settings.icon]
          ? settings.icon
          : "keyboard",
      color: normalizeColor(settings?.color),
    };
    for (const radio of targetModes) {
      radio.checked = radio.value === draft.targetMode;
    }
    tileLabel.value = draft.label;
    updateTargetVisibility();
    renderColor();
    renderSelectedIcon();
  }

  function normalizeColor(value) {
    return typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value)
      ? value.toLowerCase()
      : DEFAULT_HOTKEY_COLOR;
  }

  function normalizeWindowNumbers(value) {
    const numbers = Array.isArray(value)
      ? value.filter(
          (number) =>
            Number.isSafeInteger(number) && number >= 1 && number <= 6,
        )
      : [];
    const unique = [...new Set(numbers)].sort((a, b) => a - b);
    return unique.length ? unique : [1];
  }

  function updateTargetVisibility() {
    boxGrid.hidden = draft.targetMode !== "selected";
    targetError.hidden = true;
  }

  function renderBoxes() {
    boxGrid.replaceChildren();
    for (let number = 1; number <= 6; number += 1) {
      const box = boxes.find((candidate) => candidate.windowNumber === number);
      const label = document.createElement("label");
      label.className = "box-option";
      label.dataset.loaded = String(Boolean(box?.loaded));
      label.dataset.ready = String(Boolean(box?.inputReady));
      const input = document.createElement("input");
      input.type = "checkbox";
      input.value = String(number);
      input.checked = draft.windowNumbers.includes(number);
      input.addEventListener("change", () => {
        draft.windowNumbers = [
          ...boxGrid.querySelectorAll("input:checked"),
        ].map((candidate) => Number(candidate.value));
        targetError.hidden = draft.windowNumbers.length > 0;
        if (!targetError.hidden)
          targetError.textContent = "Select at least one Stonemite box.";
        void requestMappings();
      });
      const name = document.createElement("span");
      name.className = "box-name";
      name.textContent = box?.character
        ? `${number} · ${box.character}`
        : `${number} · ${box?.loaded ? "Unknown" : "Empty"}`;
      name.title = name.textContent;
      const dot = document.createElement("span");
      dot.className = "box-dot";
      dot.setAttribute("aria-hidden", "true");
      label.append(input, name, dot);
      boxGrid.append(label);
    }
  }

  function renderMappings() {
    const selected = draft.mapping;
    const query = mappingSearch.value.trim().toLowerCase();
    const filtered = mappings.filter(
      (mapping) =>
        !query ||
        mapping.label.toLowerCase().includes(query) ||
        mapping.value.toLowerCase().includes(query),
    );
    mappingSelect.replaceChildren();
    if (selected && !mappings.some((mapping) => mapping.value === selected)) {
      const stale = document.createElement("option");
      stale.value = selected;
      stale.textContent = `${formatMappingName(selected)} — not mapped`;
      mappingSelect.append(stale);
    }
    for (const mapping of filtered) {
      const option = document.createElement("option");
      option.value = mapping.value;
      option.textContent = mapping.label;
      option.title = mapping.value;
      mappingSelect.append(option);
    }
    mappingSelect.value = selected;
    mappingSelect.disabled = mappingBusy || mappingSelect.options.length === 0;
    mappingStatus.textContent = mappingBusy
      ? "Checking the selected boxes…"
      : `${mappings.length} shared mapped ${mappings.length === 1 ? "action" : "actions"}`;
    updateSaveAvailability();
  }

  function updateSaveAvailability() {
    const mappingKnownInvalid =
      mappingValidated &&
      Boolean(draft.mapping) &&
      !mappings.some((mapping) => mapping.value === draft.mapping);
    save.disabled =
      saving || mappingBusy || !draft.mapping || mappingKnownInvalid;
  }

  function renderColor() {
    tileColor.value = draft.color;
    colorValue.textContent = draft.color.toUpperCase();
    for (const swatch of colorSwatches) {
      swatch.setAttribute(
        "aria-selected",
        String(swatch.dataset.color === draft.color),
      );
    }
  }

  function setColor(value) {
    draft.color = normalizeColor(value);
    renderColor();
    renderSelectedIcon();
  }

  function renderSelectedIcon() {
    selectedIcon.replaceChildren();
    selectedIcon.style.color = draft.color;
    selectedIcon.innerHTML = iconSvg(draft.icon);
    selectedIconName.textContent = formatIconName(draft.icon);
    playIconAnimation(selectedIcon, draft.icon);
  }

  function renderIcons() {
    const query = iconSearch.value.trim().toLowerCase();
    const filtered = iconNames.filter(
      (name) => !query || formatIconName(name).toLowerCase().includes(query),
    );
    const visible = filtered.slice(0, iconLimit);
    iconGrid.replaceChildren();
    for (const name of visible) {
      const button = document.createElement("button");
      button.className = "icon-button";
      button.type = "button";
      button.dataset.icon = name;
      button.title = formatIconName(name);
      button.setAttribute("role", "option");
      button.setAttribute("aria-label", formatIconName(name));
      button.setAttribute("aria-selected", String(name === draft.icon));
      button.innerHTML = iconSvg(name);
      button.addEventListener("mouseenter", () =>
        playIconAnimation(button, name),
      );
      button.addEventListener("mouseleave", () =>
        stopIconAnimation(button, name),
      );
      button.addEventListener("focus", () => playIconAnimation(button, name));
      button.addEventListener("blur", () => stopIconAnimation(button, name));
      button.addEventListener("click", () => {
        draft.icon = name;
        renderSelectedIcon();
        renderIcons();
      });
      iconGrid.append(button);
    }
    iconEmpty.hidden = filtered.length > 0;
    moreIcons.hidden =
      filtered.length === 0 || visible.length >= filtered.length;
    moreIcons.textContent = `Show more icons (${filtered.length - visible.length} remaining)`;
  }

  function targetsPayload() {
    return draft.targetMode === "all"
      ? { type: "all_loaded" }
      : {
          type: "window_numbers",
          window_numbers: draft.windowNumbers.length
            ? draft.windowNumbers
            : [1],
        };
  }

  async function ensureSettingsInitialized() {
    if (!settingsInitialization) {
      settingsInitialization = (async () => {
        try {
          const received = await client.getSettings();
          applySettings(received?.settings || received || {});
        } catch {
          applySettings({});
        }
        renderBoxes();
      })();
    }
    await settingsInitialization;
  }

  async function requestMappings() {
    await ensureSettingsInitialized();
    const requestId = String(++requestSequence);
    latestRequestId = requestId;
    mappingBusy = true;
    mappingValidated = false;
    mappingError.hidden = true;
    renderMappings();
    try {
      await client.send("sendToPlugin", {
        type: "list-hotkey-mappings",
        requestId,
        targets: targetsPayload(),
      });
    } catch {
      if (latestRequestId !== requestId) return;
      mappingBusy = false;
      mappingError.textContent =
        "Stream Deck could not request EQ key mappings.";
      mappingError.hidden = false;
      renderMappings();
    }
  }

  function applyState(payload) {
    if (payload.requestId && payload.requestId !== latestRequestId) return;
    boxes = Array.isArray(payload.boxes) ? payload.boxes : [];
    renderBoxes();
    if (payload.requestId) {
      mappingBusy = false;
      mappings = Array.isArray(payload.mappings) ? payload.mappings : [];
      mappingError.hidden = true;
      mappingValidated = !payload.mappingError && payload.capabilityAvailable;
      if (payload.mappingError) {
        mappingError.textContent = payload.mappingError;
        mappingError.hidden = false;
      } else if (!payload.capabilityAvailable) {
        mappingError.textContent =
          "Update Stonemite to configure mapped hotkeys.";
        mappingError.hidden = false;
      }
      renderMappings();
    }
  }

  function iconSvg(name, frame) {
    const definition = iconCatalog[name] || iconCatalog.keyboard;
    if (!definition) return "";
    if (typeof definition === "string") return definition;
    return Number.isSafeInteger(frame)
      ? definition.frames?.[frame] || definition.normal
      : definition.normal;
  }

  function playIconAnimation(element, name) {
    stopIconAnimation(element, name);
    const definition = iconCatalog[name];
    if (reducedMotion || !definition || !Array.isArray(definition.frames))
      return;
    let frame = 0;
    element.innerHTML = iconSvg(name, frame);
    const timer = setInterval(() => {
      frame += 1;
      if (frame >= definition.frames.length) {
        stopIconAnimation(element, name);
        return;
      }
      element.innerHTML = iconSvg(name, frame);
    }, 125);
    iconTimers.set(element, timer);
  }

  function stopIconAnimation(element, name) {
    const timer = iconTimers.get(element);
    if (timer) clearInterval(timer);
    iconTimers.delete(element);
    element.innerHTML = iconSvg(name);
  }

  function formatIconName(value) {
    return value
      .split("-")
      .filter(Boolean)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ");
  }

  function formatMappingName(value) {
    const hotbar = value.match(/^HOT(\d+)_(\d+)$/);
    if (hotbar) return `Hotbar ${hotbar[1]} button ${hotbar[2]}`;
    const spell = value.match(/^CAST(\d+)$/);
    if (spell) return `Spell gem ${spell[1]}`;
    return value
      .replace(/^CMD_/, "")
      .split("_")
      .filter(Boolean)
      .map((part) => part.toLowerCase())
      .join(" ")
      .replace(/^./, (character) => character.toUpperCase());
  }

  targetModes.forEach((radio) => {
    radio.addEventListener("change", () => {
      if (!radio.checked) return;
      draft.targetMode = radio.value === "selected" ? "selected" : "all";
      updateTargetVisibility();
      void requestMappings();
    });
  });

  mappingSearch.addEventListener("input", renderMappings);
  mappingSelect.addEventListener("change", () => {
    draft.mapping = mappingSelect.value;
    formError.hidden = true;
    updateSaveAvailability();
  });
  colorSwatches.forEach((swatch) => {
    swatch.addEventListener("click", () => setColor(swatch.dataset.color));
  });
  tileColor.addEventListener("input", () => setColor(tileColor.value));
  iconSearch.addEventListener("input", () => {
    iconLimit = 72;
    renderIcons();
  });
  moreIcons.addEventListener("click", () => {
    iconLimit += 72;
    renderIcons();
  });
  refreshMappings.addEventListener("click", () => void requestMappings());

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    formError.hidden = true;
    saveStatus.textContent = "";
    draft.label = tileLabel.value.trim().slice(0, 14);
    if (draft.targetMode === "selected" && draft.windowNumbers.length === 0) {
      targetError.textContent = "Select at least one Stonemite box.";
      targetError.hidden = false;
      return;
    }
    if (!draft.mapping) {
      formError.textContent = "Choose an EQ action before saving this tile.";
      formError.hidden = false;
      mappingSelect.focus();
      return;
    }
    if (mappingBusy) {
      formError.textContent = "Wait for the selected boxes to finish checking.";
      formError.hidden = false;
      return;
    }
    if (
      mappingValidated &&
      !mappings.some((mapping) => mapping.value === draft.mapping)
    ) {
      formError.textContent =
        "Choose an action mapped for every selected loaded box.";
      formError.hidden = false;
      mappingSelect.focus();
      return;
    }
    saving = true;
    updateSaveAvailability();
    saveStatus.textContent = "Saving…";
    try {
      await client.setSettings({
        targetMode: draft.targetMode,
        windowNumbers: draft.windowNumbers,
        mapping: draft.mapping,
        label: draft.label,
        icon: draft.icon,
        color: draft.color,
      });
      saveStatus.textContent = "Tile saved.";
    } catch {
      formError.textContent = "Stream Deck could not save this tile.";
      formError.hidden = false;
      saveStatus.textContent = "";
    } finally {
      saving = false;
      updateSaveAvailability();
    }
  });

  client.sendToPropertyInspector.subscribe((event) => {
    if (event?.payload?.type === "hotkey-state") applyState(event.payload);
  });

  renderIcons();
  void requestMappings();
})();
