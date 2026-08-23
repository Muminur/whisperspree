// T0.5 — compile-time frontend mirror for every PRD §9.2 event payload.
//
// The Tauri event bridge is outside Vitest. This test therefore makes event
// names and payload properties a TypeScript contract before UI consumers can
// subscribe to them.

import { describe, expect, expectTypeOf, it } from "vitest";

import type {
  AppErrorPayload,
  AudioLevelPayload,
  InjectDonePayload,
  IpcEvent,
  IpcEventPayloads,
  LanguageDetectedPayload,
  ModelDownloadProgressPayload,
  PostprocessDonePayload,
  SessionStatePayload,
  TranscriptFinalPayload,
  TranscriptPartialPayload,
  TranscriptSegmentPayload,
  WordTiming,
} from "./ipc";

type Equal<Left, Right> = (<Value>() => Value extends Left ? 1 : 2) extends (<Value>() => Value extends Right
  ? 1
  : 2)
  ? true
  : false;
type Assert<Condition extends true> = Condition;
type IsOptional<ObjectType, Key extends keyof ObjectType> = {} extends Pick<ObjectType, Key> ? true : false;

const eventNameContract: Assert<Equal<keyof IpcEventPayloads, IpcEvent>> = true;
const sessionStateUnionContract: Assert<
  Equal<
    SessionStatePayload["state"],
    "idle" | "arming" | "listening" | "finalizing" | "post_processing" | "injecting" | "cancelled" | "error"
  >
> = true;
const optionalSessionStateMetadataContract: Assert<
  Equal<IsOptional<SessionStatePayload, "engine">, true> &
    Equal<IsOptional<SessionStatePayload, "styleId">, true>
> = true;
const wordTimingContract: Assert<Equal<WordTiming, { w: string; s: number; e: number }>> = true;

const PRD_EVENT_PAYLOADS: IpcEventPayloads = {
  "session:state": {
    sessionId: "session-1",
    state: "listening",
    engine: "local",
    styleId: "professional",
  } satisfies SessionStatePayload,
  "transcript:partial": {
    sessionId: "session-1",
    text: "hello…",
  } satisfies TranscriptPartialPayload,
  "transcript:segment": {
    sessionId: "session-1",
    text: "hello",
    words: [{ w: "hello", s: 120, e: 480 }],
  } satisfies TranscriptSegmentPayload,
  "transcript:final": {
    sessionId: "session-1",
    rawText: "hello world",
    words: [{ w: "hello", s: 120, e: 480 }],
    language: "en",
  } satisfies TranscriptFinalPayload,
  "postprocess:done": {
    sessionId: "session-1",
    text: "Hello, world.",
    personaId: "professional",
    fallbackUsed: false,
    latencyMs: 321,
  } satisfies PostprocessDonePayload,
  "inject:done": {
    sessionId: "session-1",
    method: "paste",
  } satisfies InjectDonePayload,
  "audio:level": {
    rms: 0.25,
    peak: 0.75,
  } satisfies AudioLevelPayload,
  "language:detected": {
    code: "en",
    confidence: 0.5,
  } satisfies LanguageDetectedPayload,
  "model:download:progress": {
    id: "small",
    received: 123,
    total: 456,
  } satisfies ModelDownloadProgressPayload,
  "app:error": {
    code: "NET-STREAM",
    message: "connection lost",
    recoverable: true,
  } satisfies AppErrorPayload,
};

describe("PRD §9.2 TypeScript event contract", () => {
  it("fr_0_5_all_events_have_exported_payload_interfaces_and_a_complete_map", () => {
    expect(eventNameContract).toBe(true);
    expect(sessionStateUnionContract).toBe(true);
    expect(optionalSessionStateMetadataContract).toBe(true);
    expect(wordTimingContract).toBe(true);
    expectTypeOf(PRD_EVENT_PAYLOADS).toEqualTypeOf<IpcEventPayloads>();
    expect(PRD_EVENT_PAYLOADS).toEqual({
      "session:state": {
        sessionId: "session-1",
        state: "listening",
        engine: "local",
        styleId: "professional",
      },
      "transcript:partial": { sessionId: "session-1", text: "hello…" },
      "transcript:segment": {
        sessionId: "session-1",
        text: "hello",
        words: [{ w: "hello", s: 120, e: 480 }],
      },
      "transcript:final": {
        sessionId: "session-1",
        rawText: "hello world",
        words: [{ w: "hello", s: 120, e: 480 }],
        language: "en",
      },
      "postprocess:done": {
        sessionId: "session-1",
        text: "Hello, world.",
        personaId: "professional",
        fallbackUsed: false,
        latencyMs: 321,
      },
      "inject:done": { sessionId: "session-1", method: "paste" },
      "audio:level": { rms: 0.25, peak: 0.75 },
      "language:detected": { code: "en", confidence: 0.5 },
      "model:download:progress": { id: "small", received: 123, total: 456 },
      "app:error": { code: "NET-STREAM", message: "connection lost", recoverable: true },
    });
  });

  it("fr_0_5_word_timing_rejects_non_numeric_end_offsets_at_compile_time", () => {
    const invalid: WordTiming = {
      w: "hello",
      s: 120,
      // @ts-expect-error PRD §8.2 e is a millisecond number, never an arbitrary JSON value.
      e: "480",
    };
    expect(invalid.w).toBe("hello");
  });
});
