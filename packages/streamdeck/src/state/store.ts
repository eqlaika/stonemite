import type { TrusharState } from "../types/trushar";

export type ConnectionPhase =
  "idle" | "pairing" | "connecting" | "connected" | "reconnecting" | "error";

export interface ConnectionStatus {
  state: ConnectionPhase;
  title: string;
  detail: string;
}

export interface Feedback {
  kind: "pending" | "error";
  message: string;
  until: number;
}

export interface DashboardView {
  connection: ConnectionStatus;
  snapshot: TrusharState | null;
  feedback: ReadonlyMap<string, Feedback>;
  bootStage: number;
}

const INITIAL_STATUS: ConnectionStatus = {
  state: "connecting",
  title: "Connecting",
  detail: "Looking for Stonemite on this PC.",
};

export class DashboardStore {
  #connection = INITIAL_STATUS;
  #snapshot: TrusharState | null = null;
  #feedback = new Map<string, Feedback>();
  #bootStage = 0;
  #listeners = new Set<(view: DashboardView) => void>();
  #feedbackTimers = new Map<string, ReturnType<typeof setTimeout>>();

  get view(): DashboardView {
    return {
      connection: this.#connection,
      snapshot: this.#snapshot,
      feedback: this.#feedback,
      bootStage: this.#bootStage,
    };
  }

  subscribe(listener: (view: DashboardView) => void): () => void {
    this.#listeners.add(listener);
    listener(this.view);
    return () => this.#listeners.delete(listener);
  }

  setConnection(connection: ConnectionStatus): void {
    this.#connection = connection;
    if (connection.state !== "connected") this.#snapshot = null;
    this.#emit();
  }

  setSnapshot(snapshot: TrusharState): void {
    if (this.#snapshot && snapshot.revision < this.#snapshot.revision) return;
    this.#snapshot = snapshot;
    this.#emit();
  }

  setBootStage(stage: number): void {
    this.#bootStage = Math.max(0, Math.min(3, Math.trunc(stage)));
    this.#emit();
  }

  setFeedback(
    key: string,
    feedback: Omit<Feedback, "until">,
    durationMs = 1500,
  ): void {
    const existing = this.#feedbackTimers.get(key);
    if (existing) clearTimeout(existing);
    const until = Date.now() + durationMs;
    this.#feedback.set(key, { ...feedback, until });
    this.#emit();
    const timer = setTimeout(() => {
      this.#feedback.delete(key);
      this.#feedbackTimers.delete(key);
      this.#emit();
    }, durationMs);
    timer.unref?.();
    this.#feedbackTimers.set(key, timer);
  }

  clearFeedback(key: string): void {
    const timer = this.#feedbackTimers.get(key);
    if (timer) {
      clearTimeout(timer);
      this.#feedbackTimers.delete(key);
    }
    if (this.#feedback.delete(key)) this.#emit();
  }

  clear(): void {
    this.#snapshot = null;
    for (const timer of this.#feedbackTimers.values()) clearTimeout(timer);
    this.#feedbackTimers.clear();
    this.#feedback.clear();
    this.#emit();
  }

  #emit(): void {
    const view = this.view;
    for (const listener of this.#listeners) listener(view);
  }
}
