"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	AppWindow,
	ArrowLeft,
	Ban,
	FileCode2,
	Lock,
	RefreshCw,
	Search,
	TriangleAlert,
	Users,
	X,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../../../hooks/use-invoke";
import { GlobalPermission } from "../../../../lib/permission/global-permission";
import { useBackend } from "../../../../state/backend-state";
import {
	Alert,
	AlertDescription,
	AlertTitle,
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
import { FlowScriptFailureDetailSheet } from "./flowscript-failure-detail-sheet";
import { FlowScriptOutcomeBadge } from "./flowscript-failures-shared";
import { EmptyState, StatTile } from "./telemetry-shared";
import type {
	IFlowScriptFailureFacet,
	IFlowScriptFailureRecord,
	IFlowScriptFailureResponse,
} from "./types";

const PAGE_SIZE = 25;
const DEFAULT_HOURS = 720;
const ALL = "all";

const OUTCOME_OPTIONS = ["error", "blocked", "partial"] as const;
const SOURCE_OPTIONS = ["desktop", "web"] as const;
const ORIGIN_OPTIONS = ["editor", "agent"] as const;
/**
 * FlowPilot applies through the same board pipeline a person does, and it retries. Defaulting to
 * the editor keeps this page answering the question it exists for — what people expected — with
 * the agent's own attempts one filter away.
 */
const DEFAULT_ORIGIN = "editor";
const SUMMARY_TILES = [
	"total",
	"errors",
	"blocked",
	"partial",
	"users",
	"apps",
] as const;

interface FlowScriptFailureFilterState {
	hours: number;
	outcome: string;
	source: string;
	origin: string;
	userId: string;
	appId: string;
	query: string;
}

function decodeFiltersFromSearch(params: {
	get(name: string): string | null;
}): FlowScriptFailureFilterState {
	const outcome = params.get("outcome") ?? "";
	const source = params.get("source") ?? "";
	const origin = params.get("origin") ?? DEFAULT_ORIGIN;
	return {
		hours:
			Number.parseInt(params.get("hours") ?? String(DEFAULT_HOURS), 10) ||
			DEFAULT_HOURS,
		outcome: (OUTCOME_OPTIONS as readonly string[]).includes(outcome)
			? outcome
			: ALL,
		source: (SOURCE_OPTIONS as readonly string[]).includes(source)
			? source
			: ALL,
		origin: (ORIGIN_OPTIONS as readonly string[]).includes(origin)
			? origin
			: ALL,
		userId: params.get("user") ?? "",
		appId: params.get("app") ?? "",
		query: params.get("query") ?? "",
	};
}

function encodeFiltersToSearch(
	filters: FlowScriptFailureFilterState,
	failureId: string | null,
): string {
	const p = new URLSearchParams();
	if (filters.hours !== DEFAULT_HOURS) p.set("hours", String(filters.hours));
	if (filters.outcome !== ALL) p.set("outcome", filters.outcome);
	if (filters.source !== ALL) p.set("source", filters.source);
	if (filters.origin !== DEFAULT_ORIGIN) p.set("origin", filters.origin);
	if (filters.userId) p.set("user", filters.userId);
	if (filters.appId) p.set("app", filters.appId);
	if (filters.query) p.set("query", filters.query);
	if (failureId) p.set("failure", failureId);
	return p.toString();
}

/** A breakdown whose rows narrow the list when clicked. */
function FacetBreakdown({
	rows,
	emptyMessage,
	onSelect,
}: {
	readonly rows: IFlowScriptFailureFacet[];
	readonly emptyMessage: string;
	readonly onSelect?: (key: string) => void;
}) {
	const max = Math.max(1, ...rows.map((r) => r.count));
	if (rows.length === 0) return <EmptyState message={emptyMessage} />;
	return (
		<ul className="space-y-2.5">
			{rows.map((row) => {
				const label = row.label ?? row.key;
				return (
					<li key={row.key} className="space-y-1">
						<div className="flex items-center justify-between gap-2 text-xs">
							{onSelect ? (
								<button
									type="button"
									onClick={() => onSelect(row.key)}
									className="min-w-0 truncate text-left font-medium hover:underline"
									title={label}
								>
									{label}
								</button>
							) : (
								<span className="min-w-0 truncate font-medium" title={label}>
									{label}
								</span>
							)}
							<span className="shrink-0 tabular-nums text-muted-foreground">
								{row.count.toLocaleString()}
							</span>
						</div>
						<div className="relative h-2 overflow-hidden rounded-full bg-muted">
							<div
								className="absolute inset-y-0 left-0 rounded-full bg-primary/60"
								style={{ width: `${(row.count / max) * 100}%` }}
							/>
						</div>
					</li>
				);
			})}
		</ul>
	);
}

function FailureRow({
	record,
	onOpen,
}: {
	readonly record: IFlowScriptFailureRecord;
	readonly onOpen: (record: IFlowScriptFailureRecord) => void;
}) {
	const { t } = useTranslation("admin");
	return (
		<button
			type="button"
			onClick={() => onOpen(record)}
			className="flex w-full items-start gap-4 border-b px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-muted/50"
		>
			<div className="min-w-0 flex-1 space-y-1">
				<div className="flex flex-wrap items-center gap-2">
					<FlowScriptOutcomeBadge outcome={record.outcome} />
					<Badge variant="outline" className="text-[10px]">
						{record.source}
					</Badge>
					{record.origin === "agent" ? (
						<Badge variant="secondary" className="text-[10px]">
							{t("flowPilot", "FlowPilot")}
						</Badge>
					) : null}
					{record.userName || record.userId ? (
						<span className="text-[11px] text-muted-foreground">
							{record.userName ?? record.userId}
						</span>
					) : null}
				</div>
				<div className="line-clamp-2 text-sm font-medium">{record.cause}</div>
				<div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
					<RelativeTime value={record.createdAt} />
					<span>·</span>
					<span className="font-mono">{record.appId}</span>
					<span>·</span>
					<span>
						{t("valLines", "{{val}} chars", {
							val: record.flowscriptChars.toLocaleString(),
						})}
					</span>
					{record.diagnosticCount > 0 ? (
						<>
							<span>·</span>
							<span>
								{t("valDiagnostics", "{{val}} diagnostics", {
									val: record.diagnosticCount,
								})}
							</span>
						</>
					) : null}
					{record.commandCount > 0 ? (
						<>
							<span>·</span>
							<span>
								{t("valCommandsApplied", "{{val}} commands applied", {
									val: record.commandCount,
								})}
							</span>
						</>
					) : null}
				</div>
			</div>
		</button>
	);
}

interface AdminFlowScriptFailuresPageProps {
	basePath?: string;
}

export function AdminFlowScriptFailuresPage({
	basePath = "/admin/telemetry/flowscript-failures",
}: Readonly<AdminFlowScriptFailuresPageProps>) {
	const { t } = useTranslation("admin");
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
	const initialFailure = searchParams?.get("failure") ?? null;

	const [filters, setFilters] =
		useState<FlowScriptFailureFilterState>(initialFilters);
	const [page, setPage] = useState(0);
	const [selectedId, setSelectedId] = useState<string | null>(initialFailure);
	const [showDetail, setShowDetail] = useState(Boolean(initialFailure));

	const debouncedQuery = useDebounce(filters.query, 300);

	const hourOptions = useMemo(
		() => [
			{ value: 24, label: t("last24Hours", "Last 24 hours") },
			{ value: 72, label: t("last3Days", "Last 3 days") },
			{ value: 168, label: t("last7Days", "Last 7 days") },
			{ value: 720, label: t("last30Days", "Last 30 days") },
			{ value: 2160, label: t("last90Days", "Last 90 days") },
		],
		[t],
	);

	const queryParams = useMemo(() => {
		const p = new URLSearchParams({
			hours: String(filters.hours),
			page: String(page),
			page_size: String(PAGE_SIZE),
		});
		if (filters.outcome !== ALL) p.set("outcome", filters.outcome);
		if (filters.source !== ALL) p.set("source", filters.source);
		if (filters.origin !== ALL) p.set("origin", filters.origin);
		if (filters.userId) p.set("user_id", filters.userId);
		if (filters.appId) p.set("app_id", filters.appId);
		if (debouncedQuery) p.set("query", debouncedQuery);
		return p.toString();
	}, [
		debouncedQuery,
		filters.appId,
		filters.hours,
		filters.origin,
		filters.outcome,
		filters.source,
		filters.userId,
		page,
	]);

	const failures = useQuery<IFlowScriptFailureResponse>({
		queryKey: [
			"admin",
			"telemetry",
			"flowscript-failures",
			"list",
			queryParams,
		],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<IFlowScriptFailureResponse>(
				profile.data,
				`admin/telemetry/flowscript-failures?${queryParams}`,
			);
		},
		enabled: !!profile.data,
	});

	useEffect(() => {
		const qs = encodeFiltersToSearch(filters, showDetail ? selectedId : null);
		router.replace(qs ? `${basePath}?${qs}` : basePath);
	}, [filters, selectedId, showDetail, router, basePath]);

	const setFilterValue = useCallback(
		<K extends keyof FlowScriptFailureFilterState>(
			key: K,
			value: FlowScriptFailureFilterState[K],
		) => {
			setFilters((prev) => ({ ...prev, [key]: value }));
			setPage(0);
		},
		[],
	);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "telemetry", "flowscript-failures"],
		});
	}, [queryClient]);

	const openFailure = useCallback((record: IFlowScriptFailureRecord) => {
		setSelectedId(record.id);
		setShowDetail(true);
	}, []);

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
							{t("insufficientPermissions", "Insufficient permissions")}
						</CardTitle>
						<CardDescription>
							<Trans i18nKey="youNeedTheAdminPermissionToViewCapturedFlowscriptApplies">
								You need the <b>Admin</b> permission to view captured FlowScript
								applies.
							</Trans>
						</CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	const summary = failures.data?.summary;
	const items = failures.data?.items ?? [];
	const total = failures.data?.total ?? 0;
	const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
	const pinnedFilters = [
		filters.userId ? { key: "userId" as const, value: filters.userId } : null,
		filters.appId ? { key: "appId" as const, value: filters.appId } : null,
	].filter((entry) => entry !== null);

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<FileCode2 className="h-7 w-7 text-primary" />
								{t("flowScriptApplies", "FlowScript applies")}
							</h1>
							<p className="text-muted-foreground">
								{t(
									"whatUsersTriedToApplyAndWhyItDidNotLandSourcesAreStoredRedacted",
									"What users tried to apply and why it did not land. Sources are stored redacted: declared values dropped, long literals generalized.",
								)}
							</p>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							<Button asChild variant="ghost" size="sm">
								<Link href="/admin/telemetry">
									<ArrowLeft className="mr-1 h-3.5 w-3.5" />
									{t("telemetry", "Telemetry")}
								</Link>
							</Button>
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								{t("refresh", "Refresh")}
							</Button>
						</div>
					</div>

					{failures.isError ? (
						<Alert variant="destructive">
							<TriangleAlert className="h-4 w-4" />
							<AlertTitle>
								{t(
									"couldNotLoadCapturedApplies",
									"Could not load captured applies",
								)}
							</AlertTitle>
							<AlertDescription className="flex flex-col items-start gap-2">
								<span>
									{failures.error instanceof Error
										? failures.error.message
										: t("theRequestFailed", "The request failed.")}
								</span>
								<Button
									variant="outline"
									size="sm"
									onClick={() => failures.refetch()}
								>
									<RefreshCw className="mr-1 h-3.5 w-3.5" />
									{t("tryAgain", "Try again")}
								</Button>
							</AlertDescription>
						</Alert>
					) : failures.isLoading || !summary ? (
						<div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
							{SUMMARY_TILES.map((tile) => (
								<Skeleton key={tile} className="h-16 w-full" />
							))}
						</div>
					) : (
						<div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
							<StatTile
								label={t("captured", "Captured")}
								value={summary.total.toLocaleString()}
								icon={<FileCode2 className="h-4 w-4" />}
								hint={t(
									"matchingTheCurrentFilters",
									"Matching the current filters",
								)}
							/>
							<StatTile
								label={t("errors", "Errors")}
								value={summary.errors.toLocaleString()}
								icon={<TriangleAlert className="h-4 w-4" />}
								hint={t("theApplyThrew", "The apply threw")}
							/>
							<StatTile
								label={t("blocked", "Blocked")}
								value={summary.blocked.toLocaleString()}
								icon={<Ban className="h-4 w-4" />}
								hint={t("nothingWasApplied", "Nothing was applied")}
							/>
							<StatTile
								label={t("partial", "Partial")}
								value={summary.partial.toLocaleString()}
								icon={<TriangleAlert className="h-4 w-4" />}
								hint={t("appliedWithWarnings", "Applied, but part was skipped")}
							/>
							<StatTile
								label={t("users", "Users")}
								value={summary.users.toLocaleString()}
								icon={<Users className="h-4 w-4" />}
							/>
							<StatTile
								label={t("apps", "Apps")}
								value={summary.apps.toLocaleString()}
								icon={<AppWindow className="h-4 w-4" />}
							/>
						</div>
					)}

					<div className="grid gap-4 lg:grid-cols-3">
						<Card className="lg:col-span-2">
							<CardHeader className="pb-3">
								<CardTitle className="text-base">
									{t("byCause", "By cause")}
								</CardTitle>
								<CardDescription>
									{t(
										"theErrorOrFirstDiagnosticBehindEachCaptureHighestFirst",
										"The error or first diagnostic behind each capture, most frequent first.",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<FacetBreakdown
									rows={summary?.byCause ?? []}
									emptyMessage={t(
										"noDataInTheSelectedWindow",
										"No data in the selected window.",
									)}
									onSelect={(key) => setFilterValue("query", key)}
								/>
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">
									{t("byUser", "By user")}
								</CardTitle>
								<CardDescription>
									{t(
										"whoIsHittingThisSelectOneToSeeOnlyTheirAttempts",
										"Who is hitting this. Select one to see only their attempts.",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<FacetBreakdown
									rows={summary?.byUser ?? []}
									emptyMessage={t(
										"noDataInTheSelectedWindow",
										"No data in the selected window.",
									)}
									onSelect={(key) => setFilterValue("userId", key)}
								/>
							</CardContent>
						</Card>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<div className="flex flex-wrap items-center gap-2">
								<div className="relative min-w-[14rem] flex-1">
									<Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
									<Input
										value={filters.query}
										onChange={(e) => setFilterValue("query", e.target.value)}
										placeholder={t(
											"searchCauseErrorOrSource",
											"Search cause, error or source",
										)}
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
										{hourOptions.map((o) => (
											<SelectItem key={o.value} value={String(o.value)}>
												{o.label}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={filters.outcome}
									onValueChange={(v) => setFilterValue("outcome", v)}
								>
									<SelectTrigger className="w-36">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>
											{t("allOutcomes", "All outcomes")}
										</SelectItem>
										{OUTCOME_OPTIONS.map((outcome) => (
											<SelectItem key={outcome} value={outcome}>
												{outcome}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={filters.origin}
									onValueChange={(v) => setFilterValue("origin", v)}
								>
									<SelectTrigger className="w-36">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>
											{t("allOrigins", "All origins")}
										</SelectItem>
										{ORIGIN_OPTIONS.map((origin) => (
											<SelectItem key={origin} value={origin}>
												{origin}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={filters.source}
									onValueChange={(v) => setFilterValue("source", v)}
								>
									<SelectTrigger className="w-36">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>
											{t("allSources", "All sources")}
										</SelectItem>
										{SOURCE_OPTIONS.map((source) => (
											<SelectItem key={source} value={source}>
												{source}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
							{pinnedFilters.length > 0 ? (
								<div className="flex flex-wrap items-center gap-1.5">
									{pinnedFilters.map((entry) => (
										<Button
											key={entry.key}
											variant="secondary"
											size="sm"
											className="h-7 gap-1 font-mono text-[11px]"
											onClick={() => setFilterValue(entry.key, "")}
										>
											{entry.value}
											<X className="h-3 w-3" />
										</Button>
									))}
								</div>
							) : null}
							<CardDescription>
								{t("valCapturedApplies", "{{val}} captured applies", {
									val: total.toLocaleString(),
								})}
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							{failures.isLoading ? (
								<div className="space-y-2 p-4">
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
								</div>
							) : failures.isError ? (
								<EmptyState
									message={t(
										"theCapturesCouldNotBeLoaded",
										"The captures could not be loaded.",
									)}
									className="m-4 py-10 text-sm"
								/>
							) : items.length === 0 ? (
								<EmptyState
									message={t(
										"noCapturedAppliesInThisWindowEveryFlowscriptApplyMatchingTheseFiltersDidWhatItsAuthorAsked",
										"No captures in this window — every FlowScript apply matching these filters did what its author asked.",
									)}
									className="m-4 py-10 text-sm"
								/>
							) : (
								<div>
									{items.map((record) => (
										<FailureRow
											key={record.id}
											record={record}
											onOpen={openFailure}
										/>
									))}
								</div>
							)}
						</CardContent>
					</Card>

					{totalPages > 1 && (
						<div className="flex items-center justify-between">
							<div className="text-sm text-muted-foreground">
								{t("pagePageOfTotalpages", "Page {{page}} of {{totalPages}}", {
									page: page + 1,
									totalPages,
								})}
							</div>
							<div className="flex gap-2">
								<Button
									variant="outline"
									size="sm"
									onClick={() => setPage((p) => Math.max(0, p - 1))}
									disabled={page === 0}
								>
									{t("previous", "Previous")}
								</Button>
								<Button
									variant="outline"
									size="sm"
									onClick={() =>
										setPage((p) => Math.min(totalPages - 1, p + 1))
									}
									disabled={page >= totalPages - 1}
								>
									{t("next", "Next")}
								</Button>
							</div>
						</div>
					)}
				</div>
			</div>

			<FlowScriptFailureDetailSheet
				failureId={selectedId}
				open={showDetail}
				onOpenChange={setShowDetail}
				profile={profile.data}
			/>
		</main>
	);
}
