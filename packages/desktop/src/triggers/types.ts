// TypeScript mirror of the native trigger schema (crates/eqtrigger).
// Field names match the Rust serde camelCase output exactly.

export type PresentationTarget =
  "source" | "activeClient" | "allClients" | "global";

export type TimerKind = "countdown" | "fastCountdown" | "progress" | "looping";

export type TimerRestartMode =
  | "startNew"
  | "restartAll"
  | "restartSameName"
  | "ignoreIfAnyRunning"
  | "ignoreIfSameNameRunning";

export type VariableOp = "setValue" | "setCounter" | "clear";

export interface Pattern {
  text: string;
  useRegex: boolean;
}

export interface TimerStageActions {
  displayText: string | null;
  speakText: string | null;
  sound: string | null;
}

export interface TimerBehavior {
  kind: TimerKind;
  timerName: string;
  durationSeconds: number;
  resetDurationSeconds: number;
  timesToLoop: number;
  restartMode: TimerRestartMode;
  warningSeconds: number;
  warning: TimerStageActions;
  end: TimerStageActions;
  earlyEnd: TimerStageActions;
  endEarlyPatterns: Pattern[];
  endEarlyRepeatedCount: number;
  endClearVariables: string[];
}

export interface VariableAction {
  op: VariableOp;
  name: string;
  value: string;
  step: number;
  initialValue: number;
  timeToLiveSeconds: number;
}

export interface Quarantine {
  reason: string;
  detail: string;
}

export interface Trigger {
  id: string;
  name: string;
  folder: string | null;
  index: number;
  enabled: boolean;
  comments: string;
  pattern: Pattern;
  previousPattern: Pattern | null;
  condition: string;
  lockoutSeconds: number;
  repeatedResetSeconds: number;
  variableActions: VariableAction[];
  displayText: string | null;
  speakText: string | null;
  sound: string | null;
  timer: TimerBehavior | null;
  priority: number;
  voiceRate: number;
  volume: number;
  textOverlays: string[];
  timerOverlays: string[];
  fontColor: string | null;
  activeColor: string | null;
  idleColor: string | null;
  resetColor: string | null;
  target: PresentationTarget;
  quarantine: Quarantine | null;
  passthrough: Record<string, unknown>;
}

export interface Folder {
  id: string;
  name: string;
  parent: string | null;
  index: number;
  expanded: boolean;
}

export interface CharacterSelector {
  character: string;
  server: string;
}

export type ProfileAssignment =
  { kind: "global" } | { kind: "characters"; characters: CharacterSelector[] };

export interface Profile {
  id: string;
  name: string;
  enabled: boolean;
  assignment: ProfileAssignment;
  triggers: string[];
  folders: string[];
  voice: string | null;
  voiceRate: number;
  volume: number;
}

export interface TextOverlayPreset {
  id: string;
  name: string;
  isDefault: boolean;
  fontSize: string;
  fontColor: string;
  backgroundColor: string;
  fadeDelaySeconds: number;
  passthrough: Record<string, unknown>;
}

export type TimerOverlayMode = "standard" | "cooldown";

export interface TimerOverlayPreset {
  id: string;
  name: string;
  isDefault: boolean;
  mode: TimerOverlayMode;
  sortBy: number;
  fontColor: string;
  activeColor: string;
  idleColor: string;
  resetColor: string;
  backgroundColor: string;
  showMillis: boolean;
  passthrough: Record<string, unknown>;
}

export interface AssetRecord {
  name: string;
  fileName: string;
  sha256: string;
  size: number;
}

export interface TriggerLibrary {
  schemaVersion: number;
  folders: Folder[];
  triggers: Trigger[];
  profiles: Profile[];
  textOverlays: TextOverlayPreset[];
  timerOverlays: TimerOverlayPreset[];
  assets: AssetRecord[];
}

export type CompatSeverity = "info" | "warning" | "error";

export interface CompatIssue {
  severity: CompatSeverity;
  subject: string;
  code: string;
  detail: string;
}

export interface CompatReport {
  issues: CompatIssue[];
  triggersImported: number;
  foldersImported: number;
  overlaysImported: number;
  triggersQuarantined: number;
}

export interface BuiltinSoundOption {
  id: string;
  label: string;
}

export interface TriggerLibraryPayload {
  library: TriggerLibrary;
  report: CompatReport;
  builtinSounds: BuiltinSoundOption[];
}

export type ImportKind = "eqlpTriggers" | "eqlpOverlays" | "gina" | "stonemite";

export interface ImportPreview {
  kind: ImportKind;
  fileName: string;
  folderCount: number;
  triggerCount: number;
  overlayCount: number;
  assetCount: number;
  quarantined: number;
  triggerNames: string[];
  report: CompatReport;
}

export interface ImportOptions {
  newFolderName: string | null;
  replaceSameNames: boolean;
  enable: boolean;
}

export interface ImportSummary {
  triggersAdded: number;
  triggersReplaced: number;
  foldersAdded: number;
  overlaysAdded: number;
  assetsAdded: number;
}

export type ExportFormat = "eqlpTriggers" | "eqlpOverlays" | "stonemite";

export interface ExportScope {
  format: ExportFormat;
  fullLibrary: boolean;
  folderIds: string[];
  triggerIds: string[];
}

export interface ExportOutcome {
  path: string | null;
  report: CompatReport;
}

// --- Test bench ---

export interface TriggerTrace {
  triggerId: string | null;
  triggerName: string;
  matched: boolean;
  matchSpans: [number, number][];
  captures: [string, string][];
  previousLineMatched: boolean | null;
  conditionPassed: boolean | null;
  constraintsPassed: boolean;
  lockoutBlocked: boolean;
  variableMutations: [string, string | null][];
  actions: string[];
}

export interface LineTrace {
  line: string;
  entries: TriggerTrace[];
}

export type ActionPhase =
  "initial" | "timerWarning" | "timerEnd" | "timerEndEarly";

export type TriggerActionValue =
  | {
      kind: "displayText";
      text: string;
      overlays: string[];
      fontColor: string | null;
    }
  | { kind: "playSound"; sound: string; volume: number }
  | { kind: "speak"; text: string; rate: number | null; volume: number };

export interface ActionEvent {
  character: string;
  triggerId: string;
  triggerName: string;
  phase: ActionPhase;
  target: PresentationTarget;
  priority: number;
  action: TriggerActionValue;
}

export interface TimerSnapshot {
  character: string;
  triggerId: string;
  kind: TimerKind;
  displayName: string;
  beginMs: number;
  endMs: number;
  durationMs: number;
  resetAtMs: number | null;
  warned: boolean;
  target: PresentationTarget;
  timerOverlays: string[];
  fontColor: string | null;
  activeColor: string | null;
  idleColor: string | null;
  resetColor: string | null;
}

export interface TestBenchRequest {
  lines: string[];
  character: string;
  server: string;
  includeDisabled: boolean;
}

export interface TestBenchLine {
  line: string;
  atMs: number;
  trace: LineTrace;
  events: ActionEvent[];
  timersAfter: TimerSnapshot[];
}

export interface TestBenchResult {
  lines: TestBenchLine[];
  compileErrors: string[];
  activeTriggers: number;
}
