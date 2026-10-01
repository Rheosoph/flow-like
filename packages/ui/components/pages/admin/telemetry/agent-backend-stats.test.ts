import { describe, expect, test } from "bun:test";
import {
	AGENT_ERROR_EVENT,
	AGENT_START_EVENT,
	aggregateAgentBackends,
} from "./agent-backend-stats";
import type { ITelemetryEventRow } from "./types";

function event(
	name: string,
	props: Record<string, unknown>,
): ITelemetryEventRow {
	return {
		id: crypto.randomUUID(),
		name,
		props,
		source: "desktop",
		anonId: "test-install",
		createdAt: "2026-09-30T12:00:00Z",
	};
}

function failure(backend: string, stage: string): ITelemetryEventRow[] {
	const props = { backend, stage, duration_ms: 0, error_kind: "unknown" };
	return [
		event(AGENT_START_EVENT, { ...props, outcome: "error" }),
		event(AGENT_ERROR_EVENT, props),
	];
}

describe("agent backend statistics", () => {
	test("counts each failure once when both telemetry streams contain it", () => {
		const events = Array.from({ length: 30 }, () =>
			event(AGENT_START_EVENT, {
				backend: "codex",
				stage: "run",
				outcome: "ok",
				duration_ms: 270_000,
			}),
		);
		events.push(
			...failure("codex", "run"),
			...failure("github_copilot", "models"),
			...failure("github_copilot", "auth"),
		);

		const [claude, codex, copilot] = aggregateAgentBackends(events);
		expect(claude).toMatchObject({ calls: 0, successRate: null });
		expect(codex).toMatchObject({
			calls: 31,
			successes: 30,
			errors: 1,
			stageErrors: 1,
			successRate: 30 / 31,
			p95DurationMs: 270_000,
			topErrorKinds: [{ kind: "unknown", count: 1 }],
		});
		expect(copilot).toMatchObject({
			calls: 2,
			errors: 2,
			stageErrors: 2,
			successRate: 0,
			p95DurationMs: 0,
			topErrorKinds: [{ kind: "unknown", count: 2 }],
		});
	});

	test("includes all operation stages in outcomes and duration", () => {
		const stages = ["spawn", "auth", "models", "run", "stop"];
		const [, codex] = aggregateAgentBackends(
			stages.map((stage, index) =>
				event(AGENT_START_EVENT, {
					backend: "codex",
					stage,
					outcome: "ok",
					duration_ms: index * 100,
				}),
			),
		);
		expect(codex).toMatchObject({
			calls: 5,
			successRate: 1,
			p95DurationMs: 400,
		});
	});

	test("retains error reports when the lifecycle row is outside the fetched window", () => {
		const [, , copilot] = aggregateAgentBackends([
			event(AGENT_ERROR_EVENT, {
				backend: "github_copilot",
				stage: "models",
				error_kind: "auth_required",
				duration_ms: 30,
			}),
			event(AGENT_ERROR_EVENT, { backend: "github_copilot", stage: "auth" }),
		]);
		expect(copilot).toMatchObject({
			calls: 0,
			stageErrors: 2,
			successRate: null,
			p95DurationMs: null,
			topErrorKinds: [
				{ kind: "auth_required", count: 1 },
				{ kind: "unknown", count: 1 },
			],
		});
	});

	test("ignores unrelated events and malformed outcomes or durations", () => {
		const [, codex] = aggregateAgentBackends([
			event("other_event", { backend: "codex", error_kind: "unrelated" }),
			event(AGENT_START_EVENT, { backend: "codex", duration_ms: 90_000 }),
			event(AGENT_START_EVENT, { backend: "codex", outcome: "unexpected" }),
			event(AGENT_ERROR_EVENT, { backend: "other", error_kind: "unknown" }),
			...[Number.NaN, Number.POSITIVE_INFINITY, -1, "30"].map((duration) =>
				event(AGENT_START_EVENT, {
					backend: "codex",
					outcome: "ok",
					duration_ms: duration,
				}),
			),
		]);
		expect(codex).toMatchObject({
			calls: 4,
			successRate: 1,
			stageErrors: 0,
			p95DurationMs: null,
			topErrorKinds: [],
		});
	});
});
