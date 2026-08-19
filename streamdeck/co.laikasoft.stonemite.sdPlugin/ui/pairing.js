(() => {
  const client = SDPIComponents.streamDeckClient;
  const connection = document.querySelector("#connection");
  const title = document.querySelector("#status-title");
  const detail = document.querySelector("#status-detail");
  const mode = document.querySelector("#connection-mode");
  const recovery = document.querySelector("#recovery");
  const recoveryDetail = document.querySelector("#recovery-detail");
  const retry = document.querySelector("#retry");
  const lanPanel = document.querySelector("#lan-panel");
  const lanSummary = document.querySelector("#lan-summary");
  const lanCurrent = document.querySelector("#lan-current");
  const lanAddress = document.querySelector("#lan-address");
  const lanReconnect = document.querySelector("#lan-reconnect");
  const replaceLan = document.querySelector("#replace-lan");
  const useLocal = document.querySelector("#use-local");
  const form = document.querySelector("#pair-form");
  const address = document.querySelector("#address");
  const code = document.querySelector("#code");
  const addressError = document.querySelector("#address-error");
  const codeError = document.querySelector("#code-error");
  const connectionError = document.querySelector("#connection-error");
  const pair = document.querySelector("#pair");
  const cancelReplace = document.querySelector("#cancel-replace");

  let lanPaired = false;
  let pairedAddress = "";
  let replacingLan = false;
  let pairingAttempted = false;
  let operationBusy = false;
  let connectionState = "connecting";

  function updateControls() {
    const busy = operationBusy || connectionState === "pairing";
    const showRecovery = ["idle", "reconnecting", "error"].includes(
      connectionState,
    );

    connection.setAttribute("aria-busy", busy ? "true" : "false");
    form.setAttribute("aria-busy", busy ? "true" : "false");
    mode.textContent =
      lanPaired || connectionState === "pairing" ? "LAN" : "This PC";
    lanSummary.textContent = lanPaired
      ? "Manage LAN connection"
      : "Connect to another PC";
    lanCurrent.hidden = !lanPaired;
    form.hidden = lanPaired && !replacingLan;
    cancelReplace.hidden = !replacingLan;
    lanAddress.textContent = pairedAddress;

    recovery.hidden = !showRecovery;
    recoveryDetail.textContent = lanPaired
      ? "Check that Stonemite is running on the paired PC and that both PCs are on the same private network."
      : "Stonemite connects automatically on this PC. Open Stonemite and confirm Integrations is enabled if it stays offline.";

    pair.disabled = busy;
    retry.disabled = busy;
    lanReconnect.disabled = busy;
    replaceLan.disabled = busy;
    useLocal.disabled = busy || !lanPaired;
    cancelReplace.disabled = busy;
    address.disabled = busy;
    code.disabled = busy;
  }

  function setOperationBusy(busy) {
    operationBusy = busy;
    updateControls();
  }

  function clearErrors() {
    for (const element of [addressError, codeError, connectionError]) {
      element.textContent = "";
      element.hidden = true;
    }
    address.removeAttribute("aria-invalid");
    code.removeAttribute("aria-invalid");
  }

  function setFieldError(field, message) {
    clearErrors();
    const element = field === "address" ? addressError : codeError;
    const input = field === "address" ? address : code;
    element.textContent = message;
    element.hidden = false;
    input.setAttribute("aria-invalid", "true");
  }

  function setConnectionError(message) {
    connectionError.textContent = message;
    connectionError.hidden = false;
  }

  function showStatus(status) {
    connectionState = status?.state || "connecting";
    connection.dataset.state = connectionState;
    title.textContent = status?.title || "Connecting";
    detail.textContent = status?.detail || "Looking for Stonemite on this PC.";
    operationBusy = false;

    if (connectionState === "connected") {
      pairingAttempted = false;
      clearErrors();
    } else if (connectionState === "error" && pairingAttempted) {
      setConnectionError(status?.detail || "LAN pairing failed.");
    }
    updateControls();
  }

  function applyGlobalSettings(saved) {
    const wasLanPaired = lanPaired;
    lanPaired =
      typeof saved?.address === "string" &&
      saved.address.length > 0 &&
      typeof saved?.authToken === "string" &&
      saved.authToken.length > 0;
    pairedAddress = lanPaired ? saved.address : "";

    if (lanPaired) {
      address.value = pairedAddress;
      replacingLan = false;
      pairingAttempted = false;
      if (!wasLanPaired) lanPanel.open = false;
    } else if (wasLanPaired) {
      address.value = "";
      code.value = "";
      replacingLan = false;
      pairingAttempted = false;
      lanPanel.open = false;
    }
    updateControls();
  }

  function formatCode() {
    const digits = code.value.replace(/\D/g, "").slice(0, 6);
    code.value =
      digits.length > 3 ? `${digits.slice(0, 3)} ${digits.slice(3)}` : digits;
    clearErrors();
  }

  async function reconnect() {
    clearErrors();
    setOperationBusy(true);
    try {
      await client.send("sendToPlugin", { type: "reconnect" });
    } catch {
      setOperationBusy(false);
      setConnectionError("Stream Deck could not retry the connection.");
    }
  }

  code.addEventListener("input", formatCode);
  address.addEventListener("input", clearErrors);

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const host = address.value.trim();
    const digits = code.value.replace(/\D/g, "");
    if (!host) {
      setFieldError("address", "Enter the address shown by Stonemite.");
      address.focus();
      return;
    }
    if (!/^\d{6}$/.test(digits)) {
      setFieldError("code", "Enter all six digits from Stonemite.");
      code.focus();
      return;
    }

    clearErrors();
    pairingAttempted = true;
    setOperationBusy(true);
    code.value = "";
    try {
      await client.send("sendToPlugin", {
        type: "pair",
        address: host,
        code: digits,
      });
    } catch {
      setOperationBusy(false);
      setConnectionError("Stream Deck could not send the LAN pairing request.");
    }
  });

  retry.addEventListener("click", reconnect);
  lanReconnect.addEventListener("click", reconnect);

  replaceLan.addEventListener("click", () => {
    clearErrors();
    replacingLan = true;
    pairingAttempted = false;
    lanPanel.open = true;
    code.value = "";
    updateControls();
    address.focus();
    address.select();
  });

  cancelReplace.addEventListener("click", () => {
    clearErrors();
    replacingLan = false;
    pairingAttempted = false;
    address.value = pairedAddress;
    code.value = "";
    updateControls();
  });

  useLocal.addEventListener("click", async () => {
    clearErrors();
    setOperationBusy(true);
    code.value = "";
    try {
      await client.send("sendToPlugin", { type: "forget" });
    } catch {
      setOperationBusy(false);
      setConnectionError("Stream Deck could not switch back to this PC.");
    }
  });

  client.sendToPropertyInspector.subscribe((event) => {
    if (event?.payload?.type === "connection-status") {
      showStatus(event.payload.status);
    }
  });

  client.didReceiveGlobalSettings.subscribe((event) => {
    applyGlobalSettings(event?.payload?.settings);
  });

  void (async () => {
    applyGlobalSettings(await client.getGlobalSettings());
    await client.send("sendToPlugin", { type: "get-status" });
  })();
})();
