(() => {
  const client = SDPIComponents.streamDeckClient;
  const connection = document.querySelector("#connection");
  const title = document.querySelector("#status-title");
  const detail = document.querySelector("#status-detail");
  const form = document.querySelector("#pair-form");
  const address = document.querySelector("#address");
  const code = document.querySelector("#code");
  const addressError = document.querySelector("#address-error");
  const codeError = document.querySelector("#code-error");
  const connectionError = document.querySelector("#connection-error");
  const pair = document.querySelector("#pair");
  const reconnect = document.querySelector("#reconnect");
  const forget = document.querySelector("#forget");

  let paired = false;
  let operationBusy = false;
  let connectionState = "idle";

  function updateControls() {
    const connecting = ["pairing", "connecting"].includes(connectionState);
    const busy = operationBusy || connecting;
    connection.setAttribute("aria-busy", busy ? "true" : "false");
    form.setAttribute("aria-busy", busy ? "true" : "false");
    pair.disabled = busy || paired;
    reconnect.disabled = busy || !paired || connectionState === "connected";
    forget.disabled = busy || !paired;
    address.disabled = busy || paired;
    code.disabled = busy || paired;
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
    clearErrors();
    connectionError.textContent = message;
    connectionError.hidden = false;
  }

  function showStatus(status) {
    connectionState = status?.state || "idle";
    connection.dataset.state = connectionState;
    title.textContent = status?.title || "Not paired";
    detail.textContent = status?.detail || "Open Stonemite settings to begin.";
    operationBusy = false;
    if (connectionState === "error")
      setConnectionError(status.detail || "Connection failed.");
    else if (connectionState === "connected") clearErrors();
    updateControls();
  }

  function applyGlobalSettings(saved) {
    paired = typeof saved?.authToken === "string" && saved.authToken.length > 0;
    if (typeof saved?.address === "string" && saved.address)
      address.value = saved.address;
    updateControls();
  }

  code.addEventListener("input", () => {
    const digits = code.value.replace(/\D/g, "").slice(0, 6);
    code.value =
      digits.length > 3 ? `${digits.slice(0, 3)} ${digits.slice(3)}` : digits;
    clearErrors();
  });

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
      setConnectionError("Stream Deck could not send the pairing request.");
    }
  });

  reconnect.addEventListener("click", async () => {
    clearErrors();
    setOperationBusy(true);
    try {
      await client.send("sendToPlugin", {
        type: "reconnect",
        address: address.value.trim(),
      });
    } catch {
      setOperationBusy(false);
      setConnectionError("Stream Deck could not send the reconnect request.");
    }
  });

  forget.addEventListener("click", async () => {
    clearErrors();
    setOperationBusy(true);
    code.value = "";
    try {
      await client.send("sendToPlugin", { type: "forget" });
    } catch {
      setOperationBusy(false);
      setConnectionError("Stream Deck could not forget this device.");
    }
  });

  client.sendToPropertyInspector.subscribe((event) => {
    if (event?.payload?.type === "connection-status")
      showStatus(event.payload.status);
  });

  client.didReceiveGlobalSettings.subscribe((event) => {
    applyGlobalSettings(event?.payload?.settings);
  });

  void (async () => {
    applyGlobalSettings(await client.getGlobalSettings());
    await client.send("sendToPlugin", { type: "get-status" });
  })();
})();
