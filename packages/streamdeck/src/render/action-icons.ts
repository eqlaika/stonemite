// Lucide icon geometry (ISC) with motion adapted from Lucide Animated (MIT).
// The full notices ship in co.laikasoft.stonemite.sdPlugin/THIRD_PARTY_NOTICES.md.

import {
  LUCIDE_ANIMATED_ICONS,
  type LucideAnimatedIcon,
} from "./lucide-animated.generated";

export type { LucideAnimatedIcon };

export const LUCIDE_ANIMATED_ICON_NAMES = Object.keys(
  LUCIDE_ANIMATED_ICONS,
) as LucideAnimatedIcon[];

export function isLucideAnimatedIcon(
  value: unknown,
): value is LucideAnimatedIcon {
  return (
    typeof value === "string" &&
    Object.prototype.hasOwnProperty.call(LUCIDE_ANIMATED_ICONS, value)
  );
}

export function renderConfigurableLucideIcon(
  icon: LucideAnimatedIcon,
  color: string,
  motionFrame: number,
  active: boolean,
  transform = "translate(18 5.5) scale(1.5)",
): string {
  const definition = LUCIDE_ANIMATED_ICONS[icon];
  const frame = normalizedMotionFrame(motionFrame);
  const svg = active
    ? (definition.frames[frame] ?? definition.normal)
    : definition.normal;
  return `<g data-icon="${icon}" data-icon-set="lucide-animated" data-active="${active}" data-frame="${frame}" color="${color}" transform="${transform}">${inlineLucideSvg(svg, color)}</g>`;
}

// Stream Deck drops nested SVG roots in key images, so flatten each catalog frame.
function inlineLucideSvg(svg: string, color: string): string {
  const match = svg.match(/^<svg([^>]*)>([\s\S]*)<\/svg>$/u);
  if (!match) return svg.replaceAll("currentColor", color);

  const attributes = match[1] ?? "";
  const content = (match[2] ?? "").replaceAll("currentColor", color);
  const viewBoxValues = attributes
    .match(/\sviewBox="([^"]+)"/u)?.[1]
    ?.trim()
    .split(/\s+/u)
    .map(Number);
  const viewBox =
    viewBoxValues?.length === 4 &&
    viewBoxValues.every(Number.isFinite) &&
    viewBoxValues[2] !== 0 &&
    viewBoxValues[3] !== 0
      ? (viewBoxValues as [number, number, number, number])
      : ([0, 0, 24, 24] as const);
  const normalizedAttributes = attributes
    .replace(/\s(?:height|viewBox|width|xmlns)="[^"]*"/gu, "")
    .replaceAll("currentColor", color);
  const [minX, minY, width, height] = viewBox;
  const scaleX = 24 / width;
  const scaleY = 24 / height;
  const needsViewBoxTransform =
    minX !== 0 || minY !== 0 || width !== 24 || height !== 24;
  const normalizedContent = needsViewBoxTransform
    ? `<g transform="matrix(${scaleX} 0 0 ${scaleY} ${-minX * scaleX} ${-minY * scaleY})">${content}</g>`
    : content;

  return `<g${normalizedAttributes}>${normalizedContent}</g>`;
}

const FRAME_COUNT = 8;
const ICON_TRANSFORM = "translate(18 5.5) scale(1.5)";
const TOP_ARROW_SCALE = [1, 0.9, 1.15, 1.04, 1, 1, 1, 1] as const;
const BOTTOM_ARROW_SCALE = [1, 1, 1, 0.9, 1.15, 1.04, 1, 1] as const;

export function renderSwapIcon(
  color: string,
  motionFrame: number,
  active: boolean,
): string {
  const frame = active ? normalizedMotionFrame(motionFrame) : 0;
  const attributes = `data-icon="swap" data-icon-set="lucide-animated" data-active="${active}" data-frame="${frame}" fill="none" stroke="${color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" transform="${ICON_TRANSFORM}"`;
  const topScale = active ? TOP_ARROW_SCALE[frame]! : 1;
  const bottomScale = active ? BOTTOM_ARROW_SCALE[frame]! : 1;

  return `<g ${attributes}><g data-motion-part="top-arrow" transform="translate(12 7) scale(${topScale}) translate(-12 -7)"><path d="M8 3 4 7l4 4"/><path d="M4 7h16"/></g><g data-motion-part="bottom-arrow" transform="translate(12 17) scale(${bottomScale}) translate(-12 -17)"><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/></g></g>`;
}

function normalizedMotionFrame(motionFrame: number): number {
  const frame = Math.trunc(motionFrame) % FRAME_COUNT;
  return frame < 0 ? frame + FRAME_COUNT : frame;
}
