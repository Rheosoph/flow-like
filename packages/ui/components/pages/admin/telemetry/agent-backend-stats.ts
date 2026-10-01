import type {
	IAgentBackendErrorKindCount,
	IAgentBackendId,
	IAgentBackendStats,
} from "./llm-types";
import type { ITelemetryEventRow } from "./types";

export const AGENT_BACKENDS: { id: IAgentBackendId; label: string }[] = [
	{ id: "claude_code", label: "Claude Code" },
	{ id: "codex", label: "Codex" },
	{ id: "github_copilot", label: "GitHub Copilot" },
];

// The existing event name covers every completed backend stage, including runs.
export const AGENT_START_EVENT = "agent_backend_start";
export const AGENT_ERROR_EVENT = "agent_backend_error";
const TOP_ERROR_KINDS = 3;

function readString(
	props: Record<string, unknown> | null | undefined,
	key: string,
): string | null {
	const value = props?.[key];
	return typeof value === "string" && value.length > 0 ? value : null;
}

function readNumber(
	props: Record<string, unknown> | null | undefined,
	key: string,
): number | null {
	const value = props?.[key];
	return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function percentile(values: number[], p: number): number | null {
	if (values.length === 0) return null;
	const sorted = [...values].sort((a, b) => a - b);
	const rank = Math.ceil((p / 100) * sorted.length);
	return sorted[Math.min(sorted.length - 1, Math.max(0, rank - 1))];
}

interface BackendAccumulator {
	calls: number;
	successes: number;
	errors: number;
	stageErrors: number;
	durations: number[];
	errorKinds: Map<string, number>;
}

function emptyAccumulator(): BackendAccumulator {
	return {
		calls: 0,
		successes: 0,
		errors: 0,
		stageErrors: 0,
		durations: [],
		errorKinds: new Map(),
	};
}

function topErrorKinds(
	kinds: Map<string, number>,
): IAgentBackendErrorKindCount[] {
	return [...kinds.entries()]
		.map(([kind, count]) => ({ kind, count }))
		.sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind))
		.slice(0, TOP_ERROR_KINDS);
}

export function aggregateAgentBackends(
	events: ITelemetryEventRow[],
): IAgentBackendStats[] {
	const accumulators = new Map<IAgentBackendId, BackendAccumulator>(
		AGENT_BACKENDS.map((backend) => [backend.id, emptyAccumulator()]),
	);

	for (const event of events) {
		const backend = readString(
			event.props,
			"backend",
		) as IAgentBackendId | null;
		if (!backend) continue;
		const acc = accumulators.get(backend);
		if (!acc) continue;

		if (event.name === AGENT_START_EVENT) {
			const outcome = readString(event.props, "outcome");
			if (outcome !== "ok" && outcome !== "error") continue;
			acc.calls += 1;
			if (outcome === "error") {
				acc.errors += 1;
			} else {
				acc.successes += 1;
			}
			const duration = readNumber(event.props, "duration_ms");
			if (duration != null && duration >= 0) acc.durations.push(duration);
		} else if (event.name === AGENT_ERROR_EVENT) {
			// A failed operation emits both events. Only its error report counts
			// toward the kind breakdown, including reports without a lifecycle row.
			acc.stageErrors += 1;
			const errorKind = readString(event.props, "error_kind") ?? "unknown";
			acc.errorKinds.set(errorKind, (acc.errorKinds.get(errorKind) ?? 0) + 1);
		}
	}

	return AGENT_BACKENDS.map(({ id, label }) => {
		const acc = accumulators.get(id) ?? emptyAccumulator();
		return {
			backend: id,
			label,
			calls: acc.calls,
			successes: acc.successes,
			errors: acc.errors,
			stageErrors: acc.stageErrors,
			successRate: acc.calls > 0 ? acc.successes / acc.calls : null,
			p95DurationMs: percentile(acc.durations, 95),
			topErrorKinds: topErrorKinds(acc.errorKinds),
		};
	});
}
