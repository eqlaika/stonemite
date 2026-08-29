import type { JsonObject } from "@elgato/utils";
import type { CharacterSlot } from "../state/layout";

export interface BoxSettings extends JsonObject {
  windowNumber?: number;
}

export function normalizeWindowNumber(value: unknown): CharacterSlot {
  return Number.isSafeInteger(value) && Number(value) >= 1 && Number(value) <= 6
    ? (Number(value) as CharacterSlot)
    : 1;
}

export function characterKeyForWindow(
  windowNumber: CharacterSlot,
): `character-${CharacterSlot}` {
  return `character-${windowNumber}`;
}
