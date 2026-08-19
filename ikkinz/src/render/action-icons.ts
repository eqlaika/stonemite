// Lucide icon geometry (ISC) with motion adapted from Lucide Animated (MIT).
// The full notices ship in co.laikasoft.ikkinz.sdPlugin/THIRD_PARTY_NOTICES.md.

export type ActionIcon = "group" | "follow" | "assist" | "use" | "swap";

const FRAME_COUNT = 8;
const ICON_TRANSFORM = "translate(18 5.5) scale(1.5)";
const JOIN_PROGRESS = [0, 0.2, 0.55, 0.85, 1, 1, 0.7, 0.3] as const;
const ROUTE_PROGRESS = [0, 0.18, 0.42, 0.68, 0.9, 1, 0.72, 0.35] as const;
const TOP_ARROW_SCALE = [1, 0.9, 1.15, 1.04, 1, 1, 1, 1] as const;
const BOTTOM_ARROW_SCALE = [1, 1, 1, 0.9, 1.15, 1.04, 1, 1] as const;
const TARGET_SCALE = [1, 0.9, 1.14, 1.04, 1, 1, 1, 1] as const;
const USE_SCALE = [1, 0.94, 1.08, 1.02, 1, 1, 1, 1] as const;
const USE_RAY_OPACITY = [1, 0.4, 1, 0.7, 1, 1, 1, 1] as const;

export function renderLucideActionIcon(
  icon: ActionIcon,
  color: string,
  motionFrame: number,
  active: boolean,
): string {
  const frame = active ? normalizedMotionFrame(motionFrame) : 0;
  const attributes = `data-icon="${icon}" data-icon-set="lucide-animated" data-active="${active}" data-frame="${frame}" fill="none" stroke="${color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" transform="${ICON_TRANSFORM}"`;

  if (icon === "group") return renderUsersRound(attributes, frame, active);
  if (icon === "follow") return renderRoute(attributes, frame, active);
  if (icon === "assist") return renderTarget(attributes, frame, active);
  if (icon === "use") return renderMousePointerClick(attributes, frame, active);
  return renderArrowLeftRight(attributes, frame, active);
}

function renderUsersRound(
  attributes: string,
  frame: number,
  active: boolean,
): string {
  const progress = active ? JOIN_PROGRESS[frame]! : 1;
  const translateX = formatNumber(-4 * (1 - progress));
  const opacity = formatNumber(0.2 + progress * 0.8);

  return `<g ${attributes}><path d="M18 21a8 8 0 0 0-16 0"/><circle cx="10" cy="8" r="5"/><path data-motion-part="joining-user" d="M22 20c0-3.37-2-6.5-4-8a5 5 0 0 0-.45-8.3" transform="translate(${translateX} 0)" opacity="${opacity}"/></g>`;
}

function renderRoute(
  attributes: string,
  frame: number,
  active: boolean,
): string {
  const progress = active ? ROUTE_PROGRESS[frame]! : 1;
  const startProgress = clamp01(progress * 4);
  const pathProgress = clamp01((progress - 0.12) / 0.68);
  const endProgress = clamp01((progress - 0.72) / 0.28);

  return `<g ${attributes}><circle cx="6" cy="19" r="3"${drawAttributes(startProgress, active)}/><path data-motion-part="route" d="M9 19h8.5a3.5 3.5 0 0 0 0-7h-11a3.5 3.5 0 0 1 0-7H15"${drawAttributes(pathProgress, active)}/><circle cx="18" cy="5" r="3"${drawAttributes(endProgress, active)}/></g>`;
}

function renderTarget(
  attributes: string,
  frame: number,
  active: boolean,
): string {
  const outerScale = active ? TARGET_SCALE[frame]! : 1;
  const middleScale = active ? TARGET_SCALE[(frame + 6) % FRAME_COUNT]! : 1;
  const innerScale = active ? TARGET_SCALE[(frame + 4) % FRAME_COUNT]! : 1;

  return `<g ${attributes}><circle data-motion-part="outer-target" cx="12" cy="12" r="10" transform="translate(12 12) scale(${outerScale}) translate(-12 -12)"/><circle data-motion-part="middle-target" cx="12" cy="12" r="6" transform="translate(12 12) scale(${middleScale}) translate(-12 -12)"/><circle data-motion-part="inner-target" cx="12" cy="12" r="2" transform="translate(12 12) scale(${innerScale}) translate(-12 -12)"/></g>`;
}

function renderMousePointerClick(
  attributes: string,
  frame: number,
  active: boolean,
): string {
  const scale = active ? USE_SCALE[frame]! : 1;
  const rayOpacity = active ? USE_RAY_OPACITY[frame]! : 1;

  return `<g ${attributes}><g data-motion-part="pointer" transform="translate(13 14) scale(${scale}) translate(-13 -14)"><path d="m9 9 5 12 1.8-5.2L21 14Z"/></g><g data-motion-part="click-rays" opacity="${rayOpacity}"><path d="M7.2 2.2 8 5.1"/><path d="m5.1 8-2.9-.8"/><path d="M14 4.1 12 6"/><path d="m6 12-1.9 2"/></g></g>`;
}

function renderArrowLeftRight(
  attributes: string,
  frame: number,
  active: boolean,
): string {
  const topScale = active ? TOP_ARROW_SCALE[frame]! : 1;
  const bottomScale = active ? BOTTOM_ARROW_SCALE[frame]! : 1;

  return `<g ${attributes}><g data-motion-part="top-arrow" transform="translate(12 7) scale(${topScale}) translate(-12 -7)"><path d="M8 3 4 7l4 4"/><path d="M4 7h16"/></g><g data-motion-part="bottom-arrow" transform="translate(12 17) scale(${bottomScale}) translate(-12 -17)"><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/></g></g>`;
}

function drawAttributes(progress: number, active: boolean): string {
  if (!active) return "";
  const offset = formatNumber(1 - progress);
  const opacity = formatNumber(0.2 + progress * 0.8);
  return ` pathLength="1" stroke-dasharray="1" stroke-dashoffset="${offset}" opacity="${opacity}"`;
}

function normalizedMotionFrame(motionFrame: number): number {
  const frame = Math.trunc(motionFrame) % FRAME_COUNT;
  return frame < 0 ? frame + FRAME_COUNT : frame;
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function formatNumber(value: number): string {
  return Number(value.toFixed(2)).toString();
}
