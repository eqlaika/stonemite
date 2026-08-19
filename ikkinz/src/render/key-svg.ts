import { APP_IMAGE, CLASS_IMAGES } from "./assets.generated";
import {
  renderConfigurableLucideIcon,
  renderLucideActionIcon,
  type ActionIcon,
  type LucideAnimatedIcon,
} from "./action-icons";
import { BADGE_COLORS, SLOT_COLORS, type KeyCell } from "../state/layout";

const W = 72;
const COLORS = {
  bg: "#171a1f",
  bg2: "#20242b",
  text: "#f5f7fa",
  ink: "#101615",
  muted: "#a7b0bb",
  cyan: "#59d8d0",
  amber: "#ffc75c",
  green: "#80df89",
  red: "#ff826f",
  disabled: "#6d737c",
  notification: "#203040",
};

export const ACTION_COLORS = {
  group: SLOT_COLORS[0],
  follow: COLORS.green,
  assist: SLOT_COLORS[3],
  use: SLOT_COLORS[4],
  swap: COLORS.cyan,
} satisfies Record<ActionIcon, string>;

export function renderCell(cell: KeyCell, motionFrame = 0): string {
  const body = (() => {
    switch (cell.type) {
      case "boot":
        return renderBoot(cell.stage);
      case "feedback":
        return renderFeedback(
          cell.feedback.kind,
          cell.feedback.message,
          cell.feedback.motion,
          motionFrame,
        );
      case "character":
        return renderCharacter(
          cell.client,
          cell.slot,
          cell.enabled,
          cell.interaction,
        );
      case "empty":
        return `${base()}<circle cx="36" cy="36" r="13" fill="${COLORS.notification}"/><text x="36" y="43" class="text center large">${cell.slot}</text>`;
      case "blank":
        return base();
      case "logo":
        return renderLogo(cell.connection);
      case "group":
        return renderActionTile(
          "group",
          "Group",
          cell.available ? ACTION_COLORS.group : COLORS.disabled,
        );
      case "follow":
        return renderActionTile(
          "follow",
          "Follow",
          cell.available ? ACTION_COLORS.follow : COLORS.disabled,
        );
      case "assist":
        return renderActionTile(
          "assist",
          "Assist",
          cell.available ? ACTION_COLORS.assist : COLORS.disabled,
        );
      case "use":
        return renderActionTile(
          "use",
          "Use",
          cell.available ? ACTION_COLORS.use : COLORS.disabled,
        );
      case "broadcast":
        return renderBroadcast(cell.available, cell.enabled);
      case "swap":
        return renderSwap(cell.available, cell.armed, motionFrame);
    }
  })();

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${W}" viewBox="0 0 ${W} ${W}" role="img"><style>${styles()}</style>${body}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export interface HotkeyTile {
  configured: boolean;
  label: string;
  icon: LucideAnimatedIcon;
  color?: string;
  targets: string;
  status?: string;
  available: boolean;
  active: boolean;
}

export function renderHotkeyTile(tile: HotkeyTile, motionFrame = 0): string {
  const chosenColor = normalizeHotkeyColor(tile.color);
  const accent = tile.available ? chosenColor : COLORS.disabled;
  const active = tile.active && tile.available;
  const activeForeground = contrastingHotkeyForeground(chosenColor);
  const surface = active
    ? `<rect width="72" height="72" rx="7" fill="${accent}"/>`
    : base();
  const foreground = active
    ? activeForeground
    : tile.configured
      ? accent
      : COLORS.amber;
  const icon = tile.configured ? tile.icon : "keyboard";
  const label = tile.configured ? tile.label : "Configure";
  const targetCopy = tile.status ?? tile.targets;
  const target = tile.configured
    ? `<text x="56" y="22" class="${active ? "active-text" : "muted"} center"${active ? ` style="fill:${activeForeground}"` : ""} font-size="15"${fittedTextAttributes(targetCopy, 3, 24)}>${escapeXml(targetCopy)}</text>`
    : "";
  const body = `${surface}${renderConfigurableLucideIcon(
    icon,
    foreground,
    motionFrame,
    active,
    tile.configured ? "translate(8 7) scale(1.35)" : undefined,
  )}${target}<text x="36" y="65" class="${active ? "active-text" : "text"} center"${active ? ` style="fill:${activeForeground}"` : ""} font-size="15"${fittedTextAttributes(label, 7, 62)}>${escapeXml(label)}</text>`;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${W}" viewBox="0 0 ${W} ${W}" role="img"><style>${styles()}</style>${body}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export function contrastingHotkeyForeground(color: string): string {
  const background = normalizeHotkeyColor(color);
  const luminance = relativeLuminance(background);
  const whiteContrast = 1.05 / (luminance + 0.05);
  const darkContrast = (luminance + 0.05) / 0.05;
  return whiteContrast >= darkContrast ? "#ffffff" : "#000000";
}

function normalizeHotkeyColor(value: string | undefined): string {
  return value && /^#[0-9a-f]{6}$/iu.test(value)
    ? value.toLowerCase()
    : COLORS.cyan;
}

function relativeLuminance(color: string): number {
  const channels = [1, 3, 5].map((offset) => {
    const channel = Number.parseInt(color.slice(offset, offset + 2), 16) / 255;
    return channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return (
    0.2126 * (channels[0] ?? 0) +
    0.7152 * (channels[1] ?? 0) +
    0.0722 * (channels[2] ?? 0)
  );
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
  return `.text{fill:${COLORS.text};font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-weight:800}.active-text{fill:${COLORS.ink};font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-weight:850}.muted{fill:${COLORS.muted};font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-weight:750}.quiet{fill:#77818d;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-weight:750}.center{text-anchor:middle}.tiny,.small,.medium{font-size:15px}.large{font-size:20px}`;
}

function base(): string {
  return `<defs><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop stop-color="${COLORS.bg2}"/><stop offset="1" stop-color="${COLORS.bg}"/></linearGradient></defs><rect width="72" height="72" rx="7" fill="url(#bg)"/>`;
}

function renderCharacter(
  client: Extract<KeyCell, { type: "character" }>["client"],
  slot: number,
  enabled: boolean,
  interaction: Extract<KeyCell, { type: "character" }>["interaction"],
): string {
  const color = SLOT_COLORS[(slot - 1) % SLOT_COLORS.length] ?? SLOT_COLORS[0];
  const badge =
    BADGE_COLORS[(slot - 1) % BADGE_COLORS.length] ?? BADGE_COLORS[0];
  const name = client.character ?? `Client ${slot}`;
  const nameFit = fittedTextAttributes(name, 7, 62);
  const classCode = client.class_code?.toUpperCase();
  const icon = classCode ? CLASS_IMAGES[classCode] : undefined;
  const surface = client.active
    ? `<rect width="72" height="72" rx="7" fill="${COLORS.text}"/><rect x="1.5" y="1.5" width="69" height="69" rx="5.5" fill="none" stroke="${color}" stroke-width="3"/>`
    : `<defs><linearGradient id="slot" x1="0" y1="0" x2="1" y2="1"><stop stop-color="${color}"/><stop offset="1" stop-color="#20242b" stop-opacity=".28"/></linearGradient></defs><rect width="72" height="72" rx="7" fill="url(#slot)"/>`;
  const interactionLabel =
    interaction === "swap"
      ? `<text x="36" y="44" class="${client.active ? "active-text" : "text"} center" font-size="15"${fittedTextAttributes(client.active ? "CURRENT" : "SELECT", 6, 60)}>${client.active ? "CURRENT" : "SELECT"}</text>`
      : client.active
        ? `<text x="36" y="44" class="active-text center" font-size="15">ACTIVE</text>`
        : "";
  const unavailable = !enabled
    ? `<rect width="72" height="72" rx="7" fill="#080a0d" opacity=".24"/>`
    : "";
  const readyStroke = client.active
    ? ` stroke="${COLORS.ink}" stroke-width="1"`
    : "";
  const ready = client.input_ready
    ? `<circle cx="63" cy="63" r="3" fill="${COLORS.green}"${readyStroke}/><title>Input ready</title>`
    : `<circle cx="63" cy="63" r="3" fill="${COLORS.amber}"${readyStroke}/><title>Input not ready</title>`;
  const identity = icon
    ? `<image href="${icon}" x="39" y="4" width="29" height="29" preserveAspectRatio="xMidYMid meet"/>`
    : `<rect x="40" y="5" width="27" height="27" rx="3" fill="#080a0d" opacity=".58"/><text x="53.5" y="23" class="text center small"${fittedTextAttributes(classCode ?? "?", 2, 23)}>${escapeXml(classCode ?? "?")}</text>`;
  const nameClass = client.active ? "active-text center" : "text center";

  return `${surface}<circle cx="17" cy="18" r="13" fill="${badge}"/><text x="17" y="25" class="text center large">${slot}</text>${identity}${interactionLabel}<text x="36" y="62" class="${nameClass}" font-size="15"${nameFit}>${escapeXml(name)}</text>${ready}${unavailable}`;
}

function renderSwap(
  available: boolean,
  armed: boolean,
  motionFrame: number,
): string {
  const accent = available ? ACTION_COLORS.swap : COLORS.disabled;
  return renderActionTile("swap", "Swap", accent, motionFrame, armed);
}

function renderActionTile(
  icon: ActionIcon,
  label: string,
  accent: string,
  motionFrame = 0,
  active = false,
): string {
  const surface = active
    ? `<rect width="72" height="72" rx="7" fill="${accent}"/>`
    : base();
  const foreground = active ? COLORS.ink : accent;
  const labelClass = active ? "active-text center" : "text center";
  return `${surface}${renderLucideActionIcon(icon, foreground, motionFrame, active)}<text x="36" y="65" class="${labelClass}" font-size="15"${fittedTextAttributes(label, 6, 62)}>${escapeXml(label)}</text>`;
}

function renderBroadcast(available: boolean, enabled: boolean): string {
  const lightning = `<path d="M40 10L25 31h9l-4 16 18-23H38z" fill="currentColor"/>`;
  if (enabled) {
    return `<rect width="72" height="72" rx="7" fill="#cc3020"/><g color="#fff">${lightning}</g><text x="36" y="65" class="text center small">Bcast</text>`;
  }
  const accent = available ? "#cc5040" : COLORS.disabled;
  const label = available ? "Bcast" : "UNAVAILABLE";
  return `${base()}<g color="${accent}">${lightning}</g><text x="36" y="65" class="text center small"${fittedTextAttributes(label, 6, 62)}>${label}</text>`;
}

function renderFeedback(
  kind: "pending" | "error",
  message: string,
  motion: "group" | "follow" | "assist" | "use" | undefined,
  motionFrame: number,
): string {
  if (kind === "pending" && motion) {
    const labels = {
      group: "Group",
      follow: "Follow",
      assist: "Assist",
      use: "Use",
    } as const;
    return renderActionTile(
      motion,
      labels[motion],
      ACTION_COLORS[motion],
      motionFrame,
      true,
    );
  }

  const accent = kind === "error" ? COLORS.red : COLORS.amber;
  const label = kind === "error" ? "FAILED" : "WORKING";
  const normalizedMessage = message.toUpperCase();
  return `${base()}<circle cx="36" cy="27" r="9" fill="none" stroke="${accent}" stroke-width="3"/><path d="${kind === "error" ? "M32 23l8 8m0-8-8 8" : "M36 18v9l6 4"}" fill="none" stroke="${accent}" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/><text x="36" y="52" fill="${accent}" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="15" font-weight="850" text-anchor="middle">${label}</text><text x="36" y="69" class="text center" font-size="15"${fittedTextAttributes(normalizedMessage, 7, 62)}>${escapeXml(normalizedMessage)}</text>`;
}

function fittedTextAttributes(
  value: string,
  maxCharacters: number,
  width: number,
): string {
  return value.length > maxCharacters
    ? ` textLength="${width}" lengthAdjust="spacingAndGlyphs"`
    : "";
}

function renderLogo(
  connection: Extract<KeyCell, { type: "logo" }>["connection"],
): string {
  const image = `<image href="${APP_IMAGE}" x="5" y="7" width="62" height="58" preserveAspectRatio="xMidYMid meet"/>`;
  if (connection === "connected") return `${base()}${image}`;

  const error = connection === "error" || connection === "idle";
  const label =
    connection === "pairing" ? "PAIRING" : error ? "SETUP" : "CONNECTING";
  const accent = error ? COLORS.red : COLORS.amber;
  return `${base()}<image href="${APP_IMAGE}" x="11" y="3" width="50" height="48" preserveAspectRatio="xMidYMid meet"/><circle cx="9" cy="62" r="3" fill="${accent}"/><text x="40" y="67" fill="${accent}" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="15" font-weight="850" text-anchor="middle"${fittedTextAttributes(label, 7, 54)}>${label}</text>`;
}

function renderBoot(stage: number): string {
  if (stage === 0) {
    return `${base()}<rect x="19" y="35" width="34" height="2" fill="#38414b"/>`;
  }
  const label = stage >= 2 ? "CONNECTING" : "STONEMITE";
  const accent = stage >= 2 ? COLORS.cyan : COLORS.muted;
  return `${base()}<text x="36" y="38" fill="${accent}" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="15" font-weight="850" text-anchor="middle"${fittedTextAttributes(label, 8, 62)}>${label}</text><rect x="21" y="50" width="30" height="4" rx="2" fill="${accent}"/>`;
}
