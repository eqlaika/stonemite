import { APP_IMAGE, CLASS_IMAGES } from "./assets.generated";
import { BADGE_COLORS, SLOT_COLORS, type GridCell } from "../state/layout";

const W = 72;
const COLORS = {
  bg: "#171a1f",
  bg2: "#20242b",
  text: "#f5f7fa",
  muted: "#a7b0bb",
  cyan: "#59d8d0",
  amber: "#ffc75c",
  green: "#80df89",
  red: "#ff826f",
};

export function renderCell(cell: GridCell): string {
  const body = (() => {
    switch (cell.type) {
      case "unsupported":
        return `${base(COLORS.amber)}<text x="36" y="27" class="text center" font-size="17">5 × 3</text><text x="36" y="43" fill="${COLORS.amber}" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="8" font-weight="850" text-anchor="middle">LAYOUT REQUIRED</text><text x="36" y="58" class="muted center tiny">MOVE THIS KEY</text>`;
      case "boot":
        return renderBoot(cell.row, cell.column, cell.stage);
      case "feedback":
        return renderFeedback(cell.feedback.kind, cell.feedback.message);
      case "character":
        return renderCharacter(cell.client, cell.slot, cell.enabled);
      case "empty":
        return `${base("#3c424b")}<text x="36" y="35" class="muted center medium">Slot ${cell.slot}</text><text x="36" y="49" class="quiet center tiny">NOT LOADED</text>`;
      case "utility":
        return renderUtility(cell.top, cell.main, cell.bottom, cell.accent);
      case "broadcast":
        return renderBroadcast(cell.available, cell.enabled);
      case "ambient":
        return renderAmbient(cell.label, cell.position);
    }
  })();

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${W}" viewBox="0 0 ${W} ${W}" role="img"><style>${styles()}</style>${body}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export function escapeXml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => {
    const entities: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "'": "&apos;",
      '"': "&quot;",
    };
    return entities[character] ?? character;
  });
}

function styles(): string {
  return `.text{fill:${COLORS.text};font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-weight:800}.muted{fill:${COLORS.muted};font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-weight:750}.quiet{fill:#77818d;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-weight:750}.center{text-anchor:middle}.tiny{font-size:7px;letter-spacing:.35px}.small{font-size:8px}.medium{font-size:10px}.large{font-size:20px}`;
}

function base(accent: string): string {
  return `<defs><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop stop-color="${COLORS.bg2}"/><stop offset="1" stop-color="${COLORS.bg}"/></linearGradient></defs><rect width="72" height="72" rx="7" fill="url(#bg)"/><rect width="72" height="3" fill="${accent}"/>`;
}

function renderCharacter(
  client: Extract<GridCell, { type: "character" }>["client"],
  slot: number,
  enabled: boolean,
): string {
  const color = SLOT_COLORS[(slot - 1) % SLOT_COLORS.length] ?? SLOT_COLORS[0];
  const badge =
    BADGE_COLORS[(slot - 1) % BADGE_COLORS.length] ?? BADGE_COLORS[0];
  const name = client.character ?? `Client ${slot}`;
  const nameSize = name.length > 10 ? 10 : name.length > 7 ? 12 : 15;
  const classCode = client.class_code?.toUpperCase();
  const icon = classCode ? CLASS_IMAGES[classCode] : undefined;
  const active = client.active
    ? `<rect x="1.5" y="1.5" width="69" height="69" rx="5.5" fill="none" stroke="#fff" stroke-width="3"/>`
    : "";
  const unavailable = !enabled
    ? `<rect width="72" height="72" rx="7" fill="#080a0d" opacity=".24"/>`
    : "";
  const ready = client.input_ready
    ? `<circle cx="63" cy="63" r="3" fill="${COLORS.green}"/><title>Input ready</title>`
    : `<circle cx="63" cy="63" r="3" fill="${COLORS.amber}"/><title>Input not ready</title>`;
  const identity = icon
    ? `<image href="${icon}" x="39" y="4" width="29" height="29" preserveAspectRatio="xMidYMid meet"/>`
    : `<rect x="40" y="5" width="27" height="27" rx="3" fill="#080a0d" opacity=".58"/><text x="53.5" y="22.5" class="text center small">${escapeXml(classCode ?? "?")}</text>`;

  return `<defs><linearGradient id="slot" x1="0" y1="0" x2="1" y2="1"><stop stop-color="${color}"/><stop offset="1" stop-color="#20242b" stop-opacity=".28"/></linearGradient></defs><rect width="72" height="72" rx="7" fill="url(#slot)"/><circle cx="17" cy="18" r="13" fill="${badge}"/><text x="17" y="25" class="text center large">${slot}</text>${identity}<text x="36" y="61" class="text center" font-size="${nameSize}px">${escapeXml(name)}</text>${ready}${unavailable}${active}`;
}

function renderUtility(
  top: string,
  main: string,
  bottom: string,
  accent: string,
): string {
  const mainSize = main.length > 10 ? 10 : main.length > 7 ? 13 : 18;
  return `${base(accent)}<text x="6" y="15" class="center" text-anchor="start" fill="${accent}" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="8" font-weight="800">${escapeXml(top)}</text><text x="36" y="43" class="text center" font-size="${mainSize}px">${escapeXml(main)}</text><text x="36" y="59" class="muted center tiny">${escapeXml(bottom)}</text>`;
}

function renderBroadcast(available: boolean, enabled: boolean): string {
  const lightning = `<path d="M40 10L25 31h9l-4 16 18-23H38z" fill="currentColor"/>`;
  if (enabled) {
    return `<rect width="72" height="72" rx="7" fill="#cc3020"/><g color="#fff">${lightning}</g><text x="36" y="61" class="text center small">BROADCAST</text><text x="36" y="69" fill="#ffd6cf" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="7" font-weight="850" text-anchor="middle">ON</text>`;
  }
  const accent = available ? "#cc5040" : "#6d737c";
  return `${base(accent)}<g color="${accent}">${lightning}</g><text x="36" y="62" class="text center small">BROADCAST</text>${available ? "" : '<text x="36" y="69" class="quiet center tiny">UNAVAILABLE</text>'}`;
}

function renderFeedback(
  kind: "pending" | "success" | "error",
  message: string,
): string {
  const accent =
    kind === "success"
      ? COLORS.green
      : kind === "error"
        ? COLORS.red
        : COLORS.amber;
  const label =
    kind === "success" ? "DONE" : kind === "error" ? "FAILED" : "WORKING";
  return `${base(accent)}<circle cx="36" cy="29" r="10" fill="none" stroke="${accent}" stroke-width="3"/><path d="${kind === "success" ? "M31 29l4 4 7-9" : kind === "error" ? "M31 24l10 10m0-10L31 34" : "M36 19v10l6 4"}" fill="none" stroke="${accent}" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/><text x="36" y="51" fill="${accent}" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="8" font-weight="850" text-anchor="middle">${label}</text><text x="36" y="63" class="text center" font-size="${message.length > 10 ? 7 : 8}px">${escapeXml(message.toUpperCase())}</text>`;
}

function renderAmbient(label: string, position: "left" | "right"): string {
  const imageX = position === "left" ? 45 : -7;
  return `${base("#3d4650")}<image href="${APP_IMAGE}" x="${imageX}" y="19" width="34" height="34" opacity=".3"/><text x="36" y="44" class="text center" font-size="14" opacity=".78">${escapeXml(label)}</text><text x="36" y="59" class="quiet center tiny">CORE DECK</text>`;
}

function renderBoot(row: number, column: number, stage: number): string {
  const letters: Record<string, string> = {
    "1,0": "S",
    "1,1": "T",
    "1,2": "O",
    "1,3": "N",
    "1,4": "E",
    "2,0": "M",
    "2,1": "I",
    "2,2": "T",
    "2,3": "E",
  };
  const key = `${row},${column}`;
  if (stage === 0) {
    return `${base(COLORS.cyan)}<rect x="19" y="35" width="34" height="2" fill="#38414b"/>`;
  }
  if (row === 0 && column === 2) {
    return `${base(COLORS.cyan)}<image href="${APP_IMAGE}" x="8" y="8" width="56" height="56"/>`;
  }
  const letter = letters[key];
  if (letter)
    return `${base(COLORS.cyan)}<text x="36" y="50" class="text center" font-size="34">${letter}</text>`;
  if (row === 2 && column === 4) {
    return `${base(COLORS.cyan)}<text x="36" y="38" fill="${COLORS.cyan}" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="10" font-weight="850" text-anchor="middle">${stage >= 2 ? "READY" : "LINK"}</text><rect x="21" y="47" width="30" height="4" rx="2" fill="${stage >= 2 ? COLORS.green : COLORS.cyan}"/>`;
  }
  return `${base(COLORS.cyan)}<rect x="19" y="35" width="34" height="2" fill="#38414b"/>`;
}
