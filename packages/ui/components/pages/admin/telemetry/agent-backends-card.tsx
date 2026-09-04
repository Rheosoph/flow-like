"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Terminal } from "lucide-react";
import { useMemo } from "react";
import { parseDateValue } from "../../../../lib/date";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { type IApiState, useBackend } from "../../../../state/backend-state";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Skeleton,
} from "../../../ui";
import type {
	IAgentBackendErrorKindCount,
	IAgentBackendId,
	IAgentBackendStats,
} from "./llm-types";
import { EmptyState } from "./telemetry-shared";
import { formatDurationMs, ratingTone } from "./traces-shared";
import type { ITelemetryEventRow, ITelemetryEventsResponse } from "./types";

const AGENT_BACKENDS: { id: IAgentBackendId; label: string }[] = [
	{ id: "claude_code", label: "Claude Code" },
	{ id: "codex", label: "Codex" },
	{ id: "github_copilot", label: "GitHub Copilot" },
];

const AGENT_START_EVENT = "agent_backend_start";
const AGENT_ERROR_EVENT = "agent_backend_error";
const EVENT_PAGE_SIZE = 100;
const MAX_EVENT_PAGES = 4;
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

		const errorKind = readString(event.props, "error_kind");
		const duration = readNumber(event.props, "duration_ms");

		if (event.name === AGENT_START_EVENT) {
			acc.calls += 1;
			if (readString(event.props, "outcome") === "error") {
				acc.errors += 1;
			} else {
				acc.successes += 1;
			}
			if (duration != null) acc.durations.push(duration);
		} else if (event.name === AGENT_ERROR_EVENT) {
			acc.stageErrors += 1;
		}

		if (errorKind) {
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

export function errorRateRating(errorRate: number): string {
	if (errorRate < 0.01) return "good";
	if (errorRate < 0.05) return "needs-improvement";
	return "poor";
}

export function RatePill({
	rate,
	kind,
	digits = 1,
}: {
	readonly rate: number | null | undefined;
	readonly kind: "error" | "success";
	readonly digits?: number;
}) {
	if (rate == null || Number.isNaN(rate)) {
		return <span className="text-xs text-muted-foreground">—</span>;
	}
	const errorRate = kind === "error" ? rate : 1 - rate;
	const tone = ratingTone(errorRateRating(errorRate));
	return (
		<span
			className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium tabular-nums ${tone.tile} ${tone.text}`}
		>
			<span className={`h-1.5 w-1.5 rounded-full ${tone.dot}`} />
			{(rate * 100).toFixed(digits)}% {kind === "error" ? "errors" : "success"}
		</span>
	);
}

async function fetchAgentEvents(
	apiState: IApiState,
	profile: IProfile,
	name: string,
	sinceMs: number,
): Promise<ITelemetryEventRow[]> {
	const collected: ITelemetryEventRow[] = [];

	for (let page = 0; page < MAX_EVENT_PAGES; page += 1) {
		const params = new URLSearchParams({
			name,
			page: String(page),
			page_size: String(EVENT_PAGE_SIZE),
		});
		const response = await apiState.get<ITelemetryEventsResponse>(
			profile,
			`admin/telemetry/events?${params.toString()}`,
		);
		const batch = response.events ?? [];
		if (batch.length === 0) break;

		for (const event of batch) {
			const at = parseDateValue(event.createdAt)?.getTime();
			if (at !== undefined && at >= sinceMs) collected.push(event);
		}

		const oldest = batch[batch.length - 1];
		if (
			batch.length < EVENT_PAGE_SIZE ||
			(parseDateValue(oldest.createdAt)?.getTime() ?? 0) < sinceMs
		) {
			break;
		}
	}

	return collected;
}

function BackendTile({ stats }: { readonly stats: IAgentBackendStats }) {
	const { t } = useTranslation("admin");
	const observed = stats.calls > 0 || stats.stageErrors > 0;

	return (
		<div className="rounded-xl border border-border bg-muted/40 p-4">
			<div className="flex items-center justify-between gap-2">
				<span className="truncate text-sm font-medium">{stats.label}</span>
				<span className="font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
					{stats.backend}
				</span>
			</div>

			{observed ? (
				<>
					<div className="mt-2 flex items-baseline gap-2">
						<span className="truncate text-2xl font-bold tabular-nums">
							{stats.calls.toLocaleString()}
						</span>
						<span className="text-[11px] text-muted-foreground">
							{stats.calls === 1 ? "start" : "starts"}
						</span>
					</div>
					<div className="mt-2 flex flex-wrap items-center gap-2">
						<RatePill rate={stats.successRate} kind="success" />
						<span className="text-[11px] tabular-nums text-muted-foreground">
							p95{" "}
							{stats.p95DurationMs == null
								? "—"
								: formatDurationMs(stats.p95DurationMs)}
						</span>
					</div>
					<div className="mt-1 text-[11px] tabular-nums text-muted-foreground">
						{stats.errors.toLocaleString()} {t("failed", "failed ·")}{" "}
						{stats.stageErrors.toLocaleString()}{" "}
						{t("stageErrors", "stage errors")}
					</div>
					{stats.topErrorKinds.length > 0 ? (
						<ul className="mt-3 space-y-1 border-t pt-2">
							{stats.topErrorKinds.map((entry) => (
								<li
									key={entry.kind}
									className="flex items-center justify-between gap-2 text-[11px]"
								>
									<span
										className="truncate font-mono text-muted-foreground"
										title={entry.kind}
									>
										{entry.kind}
									</span>
									<span className="tabular-nums">
										{entry.count.toLocaleString()}
									</span>
								</li>
							))}
						</ul>
					) : null}
				</>
			) : (
				<EmptyState
					message="No reported activity."
					className="mt-3 py-4 text-[11px]"
				/>
			)}
		</div>
	);
}

interface AgentBackendsCardProps {
	profile: IProfile | undefined;
	hours: number;
}

export function AgentBackendsCard({
	profile,
	hours,
}: Readonly<AgentBackendsCardProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const events = useQuery<ITelemetryEventRow[]>({
		queryKey: ["admin", "telemetry", "agent-backends", hours],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			const sinceMs = Date.now() - hours * 3_600_000;
			const batches = await Promise.all(
				[AGENT_START_EVENT, AGENT_ERROR_EVENT].map((name) =>
					fetchAgentEvents(backend.apiState, profile, name, sinceMs),
				),
			);
			return batches.flat();
		},
		enabled: !!profile,
	});

	const stats = useMemo(
		() => aggregateAgentBackends(events.data ?? []),
		[events.data],
	);
	const observed = stats.some((row) => row.calls > 0 || row.stageErrors > 0);

	return (
		<Card>
			<CardHeader className="pb-3">
				<CardTitle className="flex items-center gap-2 text-base">
					<Terminal className="h-4 w-4 text-primary" />
					{t("agentBackends", "Agent backends")}
				</CardTitle>
				<CardDescription>
					{`Local agent CLI health from anonymous aggregate events — start outcomes, durations and classified error kinds only.`}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{events.isLoading ? (
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
						{AGENT_BACKENDS.map((entry) => (
							<Skeleton key={entry.id} className="h-40" />
						))}
					</div>
				) : !observed ? (
					<EmptyState
						message="No agent backend telemetry in this window — desktop installs report these once usage telemetry is enabled."
						className="py-10 text-sm"
					/>
				) : (
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
						{stats.map((row) => (
							<BackendTile key={row.backend} stats={row} />
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}
