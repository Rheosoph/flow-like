"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	ArrowLeft,
	GitBranch,
	Lock,
	RefreshCw,
	Search,
	Timer,
	TriangleAlert,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../../../hooks/use-invoke";
import { GlobalPermission } from "../../../../lib/permission/global-permission";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Input,
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
} from "../../../ui";
import { EmptyState, StatTile } from "./telemetry-shared";
import { TelemetryTraceDetailSheet } from "./traces-detail-sheet";
import {
	MIN_DURATION_OPTIONS,
	SpanStatusBadge,
	TRACE_HOUR_OPTIONS,
	TRACE_SOURCE_OPTIONS,
	TRACE_STATUS_OPTIONS,
	formatDurationMs,
	isErrorStatus,
} from "./traces-shared";
import type { ITelemetryTraceSummary, ITelemetryTracesResponse } from "./types";

const PAGE_SIZE = 25;

const ALL = "all";

const DEFAULT_HOURS = 24;

interface TraceFilterState {
	hours: number;
	name: string;
	source: string;
	status: string;
	minDurationMs: number;
}

function decodeFiltersFromSearch(params: {
	get(name: string): string | null;
}): TraceFilterState {
	const source = params.get("source") ?? "";
	const status = params.get("status") ?? "";
	const minDuration = Number.parseInt(params.get("min_duration_ms") ?? "0", 10);
	return {
		hours:
			Number.parseInt(params.get("hours") ?? String(DEFAULT_HOURS), 10) ||
			DEFAULT_HOURS,
		name: params.get("name") ?? "",
		source: (TRACE_SOURCE_OPTIONS as readonly string[]).includes(source)
			? source
			: ALL,
		status: (TRACE_STATUS_OPTIONS as readonly string[]).includes(status)
			? status
			: ALL,
		minDurationMs:
			Number.isFinite(minDuration) && minDuration > 0 ? minDuration : 0,
	};
}

function encodeFiltersToSearch(
	filters: TraceFilterState,
	traceId: string | null,
): string {
	const p = new URLSearchParams();
	if (filters.hours !== DEFAULT_HOURS) p.set("hours", String(filters.hours));
	if (filters.name) p.set("name", filters.name);
	if (filters.source !== ALL) p.set("source", filters.source);
	if (filters.status !== ALL) p.set("status", filters.status);
	if (filters.minDurationMs > 0)
		p.set("min_duration_ms", String(filters.minDurationMs));
	if (traceId) p.set("trace", traceId);
	return p.toString();
}

function TraceRow({
	trace,
	maxDurationMs,
	onOpen,
}: {
	readonly trace: ITelemetryTraceSummary;
	readonly maxDurationMs: number;
	readonly onOpen: (trace: ITelemetryTraceSummary) => void;
}) {
	const error = isErrorStatus(trace.status);
	const widthPercent = Math.max(
		2,
		Math.min(100, (trace.durationMs / Math.max(1, maxDurationMs)) * 100),
	);
	return (
		<button
			type="button"
			onClick={() => onOpen(trace)}
			className="flex w-full items-center gap-4 border-b px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-muted/50"
		>
			<div className="min-w-0 flex-1 space-y-1">
				<div className="flex flex-wrap items-center gap-2">
					<span className="truncate font-mono text-sm font-semibold">
						{trace.rootName}
					</span>
					<SpanStatusBadge status={trace.status} />
					<Badge variant="outline" className="font-mono text-[10px]">
						{trace.source}
					</Badge>
				</div>
				<div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
					<RelativeTime value={trace.startedAt} />
					<span>·</span>
					<span className="tabular-nums">{trace.spanCount} spans</span>
					<span>·</span>
					<span
						className="truncate font-mono"
						title={trace.traceId}
					>{`trace ${trace.traceId.slice(0, 16)}`}</span>
				</div>
			</div>
			<div className="hidden w-48 shrink-0 sm:block">
				<div className="relative h-2 overflow-hidden rounded-full bg-muted">
					<div
						className="h-full rounded-full"
						style={{
							width: `${widthPercent}%`,
							background: error
								? "color-mix(in oklab, var(--destructive) 70%, transparent)"
								: "color-mix(in oklab, var(--chart-1) 60%, transparent)",
						}}
					/>
				</div>
			</div>
			<div className="w-20 shrink-0 text-right text-sm font-semibold tabular-nums">
				{formatDurationMs(trace.durationMs)}
			</div>
		</button>
	);
}

interface AdminTelemetryTracesPageProps {
	basePath?: string;
}

export function AdminTelemetryTracesPage({
	basePath = "/admin/telemetry/traces",
}: Readonly<AdminTelemetryTracesPageProps>) {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const queryClient = useQueryClient();
	const searchParams = useSearchParams();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);

	const initialFilters = useMemo(
		() => decodeFiltersFromSearch(searchParams ?? new URLSearchParams()),
		[searchParams],
	);
	const initialTrace = searchParams?.get("trace") ?? null;

	const [filters, setFilters] = useState<TraceFilterState>(initialFilters);
	const [page, setPage] = useState(0);
	const [selectedTrace, setSelectedTrace] = useState<string | null>(
		initialTrace,
	);
	const [showDetail, setShowDetail] = useState(Boolean(initialTrace));

	const debouncedName = useDebounce(filters.name, 300);

	const queryParams = useMemo(() => {
		const p = new URLSearchParams({
			hours: String(filters.hours),
			page: String(page),
			page_size: String(PAGE_SIZE),
		});
		if (debouncedName) p.set("name", debouncedName);
		if (filters.source !== ALL) p.set("source", filters.source);
		if (filters.status !== ALL) p.set("status", filters.status);
		if (filters.minDurationMs > 0)
			p.set("min_duration_ms", String(filters.minDurationMs));
		return p.toString();
	}, [
		debouncedName,
		filters.hours,
		filters.minDurationMs,
		filters.source,
		filters.status,
		page,
	]);

	const traces = useQuery<ITelemetryTracesResponse>({
		queryKey: ["admin", "telemetry", "traces", "list", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryTracesResponse>(
				profile.data,
				`admin/telemetry/traces?${queryParams}`,
			);
		},
		enabled: !!profile.data,
	});

	useEffect(() => {
		const qs = encodeFiltersToSearch(
			filters,
			showDetail ? selectedTrace : null,
		);
		router.replace(qs ? `${basePath}?${qs}` : basePath);
	}, [filters, selectedTrace, showDetail, router, basePath]);

	const setFilterValue = useCallback(
		<K extends keyof TraceFilterState>(key: K, value: TraceFilterState[K]) => {
			setFilters((prev) => ({ ...prev, [key]: value }));
			setPage(0);
		},
		[],
	);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "telemetry", "traces"],
		});
	}, [queryClient]);

	const openTrace = useCallback((trace: ITelemetryTraceSummary) => {
		setSelectedTrace(trace.traceId);
		setShowDetail(true);
	}, []);

	const rows = traces.data?.traces ?? [];
	const maxDurationMs = useMemo(
		() => rows.reduce((max, trace) => Math.max(max, trace.durationMs), 1),
		[rows],
	);
	const errorCount = rows.filter((trace) => isErrorStatus(trace.status)).length;
	const slowest = rows.reduce<ITelemetryTraceSummary | null>(
		(worst, trace) =>
			worst == null || trace.durationMs > worst.durationMs ? trace : worst,
		null,
	);

	const perms = useMemo(
		() => new GlobalPermission(info.data?.permission ?? 0),
		[info.data?.permission],
	);
	const hasAccess = perms.hasPermission(GlobalPermission.Admin);

	if (info.isLoading) {
		return (
			<main className="flex h-full min-h-0 w-full grow flex-col bg-background p-6">
				<Skeleton className="h-12 w-72" />
				<div className="mt-4 space-y-2">
					<Skeleton className="h-16 w-full" />
					<Skeleton className="h-16 w-full" />
					<Skeleton className="h-16 w-full" />
				</div>
			</main>
		);
	}

	if (!hasAccess) {
		return (
			<main className="flex h-full w-full items-center justify-center bg-background p-6">
				<Card className="max-w-md text-center">
					<CardHeader>
						<CardTitle className="flex items-center justify-center gap-2 text-base">
							<Lock className="h-4 w-4" />
							Insufficient permissions
						</CardTitle>
						<CardDescription>
							You need the <b>Admin</b> permission to view traces.
						</CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	const total = traces.data?.total ?? 0;
	const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<GitBranch className="h-7 w-7 text-primary" />
								Traces
							</h1>
							<p className="text-muted-foreground">
								Sampled distributed traces — anonymous span waterfalls across
								desktop, web and backend.
							</p>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							<Button asChild variant="ghost" size="sm">
								<Link href="/admin/telemetry">
									<ArrowLeft className="mr-1 h-3.5 w-3.5" />
									Telemetry
								</Link>
							</Button>
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								Refresh
							</Button>
						</div>
					</div>

					<div className="grid gap-2 sm:grid-cols-3">
						<StatTile
							label="Matching traces"
							value={traces.isLoading ? "…" : total.toLocaleString()}
							icon={<GitBranch className="h-4 w-4" />}
							hint="In the selected window"
						/>
						<StatTile
							label="Slowest on page"
							value={
								traces.isLoading
									? "…"
									: slowest
										? formatDurationMs(slowest.durationMs)
										: "—"
							}
							icon={<Timer className="h-4 w-4" />}
							hint={slowest?.rootName ?? "No traces yet"}
						/>
						<StatTile
							label="Failing on page"
							value={traces.isLoading ? "…" : errorCount.toLocaleString()}
							icon={<TriangleAlert className="h-4 w-4" />}
							hint="Traces with an error root span"
						/>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<div className="flex flex-wrap items-center gap-2">
								<div className="relative min-w-[14rem] flex-1">
									<Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
									<Input
										value={filters.name}
										onChange={(e) => setFilterValue("name", e.target.value)}
										placeholder="Filter by span name"
										className="pl-8"
									/>
								</div>
								<Select
									value={String(filters.hours)}
									onValueChange={(v) =>
										setFilterValue("hours", Number.parseInt(v, 10))
									}
								>
									<SelectTrigger className="w-40">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										{TRACE_HOUR_OPTIONS.map((o) => (
											<SelectItem key={o.value} value={String(o.value)}>
												{o.label}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={filters.source}
									onValueChange={(v) => setFilterValue("source", v)}
								>
									<SelectTrigger className="w-40">
										<SelectValue placeholder="Source" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>All sources</SelectItem>
										{TRACE_SOURCE_OPTIONS.map((source) => (
											<SelectItem key={source} value={source}>
												{source}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={filters.status}
									onValueChange={(v) => setFilterValue("status", v)}
								>
									<SelectTrigger className="w-32">
										<SelectValue placeholder="Status" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>All statuses</SelectItem>
										{TRACE_STATUS_OPTIONS.map((status) => (
											<SelectItem key={status} value={status}>
												{status}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={String(filters.minDurationMs)}
									onValueChange={(v) =>
										setFilterValue("minDurationMs", Number.parseInt(v, 10))
									}
								>
									<SelectTrigger className="w-48">
										<SelectValue placeholder="Minimum duration" />
									</SelectTrigger>
									<SelectContent>
										{MIN_DURATION_OPTIONS.map((o) => (
											<SelectItem key={o.value} value={String(o.value)}>
												{o.label}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
							<CardDescription>
								Root span, total duration and span count per trace. Select a row
								for its flamegraph.
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							{traces.isLoading ? (
								<div className="space-y-2 p-4">
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
								</div>
							) : rows.length === 0 ? (
								<EmptyState
									message="No traces match the current filters — traces appear once sampled spans are reported."
									className="m-4 py-10 text-sm"
								/>
							) : (
								<div>
									{rows.map((trace) => (
										<TraceRow
											key={trace.traceId}
											trace={trace}
											maxDurationMs={maxDurationMs}
											onOpen={openTrace}
										/>
									))}
								</div>
							)}
						</CardContent>
					</Card>

					{totalPages > 1 && (
						<div className="flex items-center justify-between">
							<div className="text-sm text-muted-foreground">
								Page {page + 1} of {totalPages}
							</div>
							<div className="flex gap-2">
								<Button
									variant="outline"
									size="sm"
									onClick={() => setPage((p) => Math.max(0, p - 1))}
									disabled={page === 0}
								>
									Previous
								</Button>
								<Button
									variant="outline"
									size="sm"
									onClick={() =>
										setPage((p) => Math.min(totalPages - 1, p + 1))
									}
									disabled={page >= totalPages - 1}
								>
									Next
								</Button>
							</div>
						</div>
					)}
				</div>
			</div>

			<TelemetryTraceDetailSheet
				traceId={selectedTrace}
				open={showDetail}
				onOpenChange={setShowDetail}
				profile={profile.data}
			/>
		</main>
	);
}
