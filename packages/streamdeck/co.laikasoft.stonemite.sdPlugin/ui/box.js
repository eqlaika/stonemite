(() => {
  const client = SDPIComponents.streamDeckClient;
  const form = document.querySelector("#box-form");
  const choices = [...document.querySelectorAll('input[name="box"]')];
  const status = document.querySelector("#status");
  let settings = {};
  let saving = false;
  let pendingWindowNumber;

  for (const choice of choices) choice.disabled = true;

  function normalizeWindowNumber(value) {
    return Number.isSafeInteger(value) && value >= 1 && value <= 6 ? value : 1;
  }

  async function initialize() {
    try {
      settings = (await client.getSettings()) || {};
      const selected = normalizeWindowNumber(settings.windowNumber);
      const choice = choices.find(
        (candidate) => Number(candidate.value) === selected,
      );
      if (choice) choice.checked = true;
    } catch {
      status.textContent = "Stream Deck could not load this control.";
    } finally {
      for (const choice of choices) choice.disabled = false;
    }
  }

  async function save(windowNumber) {
    pendingWindowNumber = windowNumber;
    if (saving) return;
    saving = true;
    status.textContent = "Saving…";
    let failed = false;
    while (pendingWindowNumber !== undefined) {
      const nextWindowNumber = pendingWindowNumber;
      pendingWindowNumber = undefined;
      settings = { ...settings, windowNumber: nextWindowNumber };
      try {
        await client.setSettings(settings);
        failed = false;
      } catch {
        failed = true;
      }
    }
    saving = false;
    status.textContent = failed
      ? "Stream Deck could not save this box."
      : "Saved.";
  }

  for (const choice of choices) {
    choice.addEventListener("change", () => {
      if (choice.checked)
        void save(normalizeWindowNumber(Number(choice.value)));
    });
  }
  form.addEventListener("submit", (event) => event.preventDefault());
  void initialize();
})();
