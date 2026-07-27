"use client";

import { useQuery } from "@tanstack/react-query";
import { ExternalLink, GitBranch, Timer, TriangleAlert } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Skeleton,
} from "../../../ui";
import { EmptyState, StatTile } from "./telemetry-shared";
import { formatDurationMs } from "./traces-shared";
import type { ITelemetrySpanStatsResponse } from "./types";

const WINDOW_HOURS = 24;

const PREVIEW_LIMIT = 5;

interface DashboardTelemetryTracesWidgetProps {
	profile: IProfile | undefined;
}

export function DashboardTelemetryTracesWidget({
	profile,
}: Readonly<DashboardTelemetryTracesWidgetProps>) {
	const backend = useBackend();

	const stats = useQuery<ITelemetrySpanStatsResponse>({
		queryKey: ["admin", "telemetry", "span-stats", "dashboard"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetrySpanStatsResponse>(
				profile,
				`admin/telemetry/span-stats?hours=${WINDOW_HOURS}`,
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const operations = useMemo(
		() =>
			[...(stats.data?.operations ?? [])]
				.sort((a, b) => b.p95 - a.p95)
				.slice(0, PREVIEW_LIMIT),
		[stats.data?.operations],
	);

	const maxP95 = Math.max(1, ...operations.map((operation) => operation.p95));
	const totalOperations = stats.data?.operations.length ?? 0;
	const worstErrorRate = (stats.data?.operations ?? []).reduce(
		(max, operation) => Math.max(max, operation.errorRate),
		0,
	);

	return (
		<Card className="overflow-hidden border-primary/20">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<GitBranch className="h-4 w-4 text-primary" />
						Slow operations
						<Badge variant="outline" className="text-[10px]">
							24h
						</Badge>
					</CardTitle>
					<CardDescription>
						Slowest traced operations by p95 latency
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="outline">
					<Link href="/admin/telemetry/traces">
						Open Traces
						<ExternalLink className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-2 sm:grid-cols-3">
					<StatTile
						label="Operations"
						value={stats.isLoading ? "…" : totalOperations.toLocaleString()}
						icon={<GitBranch className="h-4 w-4" />}
						hint="Distinct span names"
					/>
					<StatTile
						label="Slowest p95"
						value={
							stats.isLoading
								? "…"
								: operations.length > 0
									? formatDurationMs(operations[0].p95)
									: "—"
						}
						icon={<Timer className="h-4 w-4" />}
						hint={operations[0]?.name ?? "No spans yet"}
					/>
					<StatTile
						label="Worst error rate"
						value={
							stats.isLoading ? "…" : `${(worstErrorRate * 100).toFixed(1)}%`
						}
						icon={<TriangleAlert className="h-4 w-4" />}
						hint="Across traced operations"
					/>
				</div>

				{stats.isLoading ? (
					<div className="space-y-2">
						<Skeleton className="h-8 w-full" />
						<Skeleton className="h-8 w-full" />
						<Skeleton className="h-8 w-full" />
					</div>
				) : operations.length === 0 ? (
					<EmptyState
						message="No traced operations yet — spans appear once sampling is enabled."
						className="py-8 text-sm"
					/>
				) : (
					<ul className="space-y-1">
						{operations.map((operation) => (
							<li key={operation.name}>
								<Link
									href={`/admin/telemetry/traces?name=${encodeURIComponent(operation.name)}`}
									className="group flex items-center gap-2 rounded border bg-card/50 px-2 py-1.5 hover:bg-muted/50"
								>
									<span
										className="w-40 shrink-0 truncate font-mono text-xs font-medium group-hover:text-primary"
										title={operation.name}
									>
										{operation.name}
									</span>
									<div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
										<div
											className="h-full rounded-full"
											style={{
												width: `${(operation.p95 / maxP95) * 100}%`,
												background:
													"color-mix(in oklab, var(--chart-1) 60%, transparent)",
											}}
										/>
									</div>
									{operation.errorRate > 0 ? (
										<span className="shrink-0 text-[11px] tabular-nums text-destructive">
											{(operation.errorRate * 100).toFixed(1)}% err
										</span>
									) : null}
									<span className="w-16 shrink-0 text-right text-xs tabular-nums text-foreground">
										{formatDurationMs(operation.p95)}
									</span>
								</Link>
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}
