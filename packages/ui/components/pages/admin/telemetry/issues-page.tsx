"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	ArrowLeft,
	Bug,
	Lock,
	MonitorSmartphone,
	RefreshCw,
	Search,
	Zap,
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
import { TelemetryIssueDetailSheet } from "./issues-detail-sheet";
import {
	ISSUE_HOUR_OPTIONS,
	ISSUE_LEVEL_OPTIONS,
	ISSUE_SOURCE_OPTIONS,
	ISSUE_STATUS_OPTIONS,
	IssueLevelBadge,
	IssueStatusBadge,
} from "./issues-shared";
import { EmptyState } from "./telemetry-shared";
import type { ITelemetryIssue, ITelemetryIssuesResponse } from "./types";

const PAGE_SIZE = 25;

const ALL = "all";

interface IssueFilterState {
	hours: number;
	status: string;
	source: string;
	level: string;
	query: string;
}

function decodeFiltersFromSearch(params: {
	get(name: string): string | null;
}): IssueFilterState {
	const status = params.get("status") ?? "";
	const source = params.get("source") ?? "";
	const level = params.get("level") ?? "";
	return {
		hours: Number.parseInt(params.get("hours") ?? "168", 10) || 168,
		status: (ISSUE_STATUS_OPTIONS as readonly string[]).includes(status)
			? status
			: ALL,
		source: (ISSUE_SOURCE_OPTIONS as readonly string[]).includes(source)
			? source
			: ALL,
		level: (ISSUE_LEVEL_OPTIONS as readonly string[]).includes(level)
			? level
			: ALL,
		query: params.get("query") ?? "",
	};
}

function encodeFiltersToSearch(
	filters: IssueFilterState,
	issueId: string | null,
): string {
	const p = new URLSearchParams();
	if (filters.hours !== 168) p.set("hours", String(filters.hours));
	if (filters.status !== ALL) p.set("status", filters.status);
	if (filters.source !== ALL) p.set("source", filters.source);
	if (filters.level !== ALL) p.set("level", filters.level);
	if (filters.query) p.set("query", filters.query);
	if (issueId) p.set("issue", issueId);
	return p.toString();
}

function IssueRow({
	issue,
	onOpen,
}: {
	readonly issue: ITelemetryIssue;
	readonly onOpen: (issue: ITelemetryIssue) => void;
}) {
	const { t } = useTranslation("admin");
	return (
		<button
			type="button"
			onClick={() => onOpen(issue)}
			className="flex w-full items-start gap-4 border-b px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-muted/50"
		>
			<div className="min-w-0 flex-1 space-y-1">
				<div className="flex flex-wrap items-center gap-2">
					<IssueLevelBadge level={issue.level} />
					<span className="truncate text-sm font-semibold">{issue.kind}</span>
					<IssueStatusBadge status={issue.status} />
					<Badge variant="outline" className="font-mono text-[10px]">
						{issue.source}
					</Badge>
					{issue.platform ? (
						<span className="text-[11px] text-muted-foreground">
							{issue.platform}
						</span>
					) : null}
				</div>
				<div className="truncate text-sm text-muted-foreground">
					{issue.title}
				</div>
				{issue.culprit ? (
					<div
						className="truncate font-mono text-[11px] text-muted-foreground"
						title={issue.culprit}
					>
						{issue.culprit}
					</div>
				) : null}
				<div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
					<RelativeTime value={issue.lastSeen} />
					<span>·</span>
					<span>{t('firstSeen', 'first seen')}</span>
					<RelativeTime value={issue.firstSeen} />
					{issue.lastRelease ? (
						<>
							<span>·</span>
							<span className="font-mono">{issue.lastRelease}</span>
						</>
					) : null}
				</div>
			</div>
			<div className="flex shrink-0 items-center gap-6 pt-1">
				<div className="w-16 text-right">
					<div className="text-sm font-semibold tabular-nums">
						{issue.eventCount.toLocaleString()}
					</div>
					<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
						events
					</div>
				</div>
				<div className="w-16 text-right">
					<div className="text-sm font-semibold tabular-nums">
						{issue.installCount.toLocaleString()}
					</div>
					<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
						installs
					</div>
				</div>
			</div>
		</button>
	);
}

interface AdminTelemetryIssuesPageProps {
	basePath?: string;
}

export function AdminTelemetryIssuesPage({
	basePath = "/admin/telemetry/issues",
}: Readonly<AdminTelemetryIssuesPageProps>) {
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
	const initialIssue = searchParams?.get("issue") ?? null;

	const [filters, setFilters] = useState<IssueFilterState>(initialFilters);
	const [page, setPage] = useState(0);
	const [selectedIssue, setSelectedIssue] = useState<string | null>(
		initialIssue,
	);
	const [showDetail, setShowDetail] = useState(Boolean(initialIssue));

	const debouncedQuery = useDebounce(filters.query, 300);

	const queryParams = useMemo(() => {
		const p = new URLSearchParams({
			hours: String(filters.hours),
			page: String(page),
			page_size: String(PAGE_SIZE),
		});
		if (filters.status !== ALL) p.set("status", filters.status);
		if (filters.source !== ALL) p.set("source", filters.source);
		if (filters.level !== ALL) p.set("level", filters.level);
		if (debouncedQuery) p.set("query", debouncedQuery);
		return p.toString();
	}, [
		debouncedQuery,
		filters.hours,
		filters.level,
		filters.source,
		filters.status,
		page,
	]);

	const issues = useQuery<ITelemetryIssuesResponse>({
		queryKey: ["admin", "telemetry", "issues", "list", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryIssuesResponse>(
				profile.data,
				`admin/telemetry/issues?${queryParams}`,
			);
		},
		enabled: !!profile.data,
	});

	useEffect(() => {
		const qs = encodeFiltersToSearch(
			filters,
			showDetail ? selectedIssue : null,
		);
		router.replace(qs ? `${basePath}?${qs}` : basePath);
	}, [filters, selectedIssue, showDetail, router, basePath]);

	const setFilterValue = useCallback(
		<K extends keyof IssueFilterState>(key: K, value: IssueFilterState[K]) => {
			setFilters((prev) => ({ ...prev, [key]: value }));
			setPage(0);
		},
		[],
	);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "telemetry", "issues"],
		});
	}, [queryClient]);

	const openIssue = useCallback((issue: ITelemetryIssue) => {
		setSelectedIssue(issue.id);
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
							{t('insufficientPermissions', 'Insufficient permissions')}
						</CardTitle>
						<CardDescription><Trans i18nKey="youNeedTheBadminbPermissionToViewCrashIssues">You need the <b>Admin</b> permission to view crash issues.</Trans></CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	const total = issues.data?.total ?? 0;
	const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
	const totalEvents = (issues.data?.issues ?? []).reduce(
		(sum, issue) => sum + issue.eventCount,
		0,
	);
	const totalInstalls = (issues.data?.issues ?? []).reduce(
		(sum, issue) => sum + issue.installCount,
		0,
	);

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<Bug className="h-7 w-7 text-primary" />
								{t('issues', 'Issues')}
							</h1>
							<p className="text-muted-foreground">
								{t('groupedCrashesAndErrorsReportedByInstallsAnonymousNoUserIdentity', "Grouped crashes and errors reported by installs — anonymous, no user identity.")}
							</p>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							<Button asChild variant="ghost" size="sm">
								<Link href="/admin/telemetry">
									<ArrowLeft className="mr-1 h-3.5 w-3.5" />
									{t('telemetry', 'Telemetry')}
								</Link>
							</Button>
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								{t('refresh', 'Refresh')}
							</Button>
						</div>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<div className="flex flex-wrap items-center gap-2">
								<div className="relative min-w-[14rem] flex-1">
									<Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
									<Input
										value={filters.query}
										onChange={(e) => setFilterValue("query", e.target.value)}
										placeholder={t('searchTitleKindOrCulprit', 'Search title, kind or culprit')}
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
										{ISSUE_HOUR_OPTIONS.map((o) => (
											<SelectItem key={o.value} value={String(o.value)}>
												{o.label}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={filters.status}
									onValueChange={(v) => setFilterValue("status", v)}
								>
									<SelectTrigger className="w-36">
										<SelectValue placeholder="Status" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>{t('allStatuses', 'All statuses')}</SelectItem>
										{ISSUE_STATUS_OPTIONS.map((status) => (
											<SelectItem key={status} value={status}>
												{status}
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
										<SelectItem value={ALL}>{t('allSources', 'All sources')}</SelectItem>
										{ISSUE_SOURCE_OPTIONS.map((source) => (
											<SelectItem key={source} value={source}>
												{source}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<Select
									value={filters.level}
									onValueChange={(v) => setFilterValue("level", v)}
								>
									<SelectTrigger className="w-32">
										<SelectValue placeholder="Level" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>{t('allLevels', 'All levels')}</SelectItem>
										{ISSUE_LEVEL_OPTIONS.map((level) => (
											<SelectItem key={level} value={level}>
												{level}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
							<CardDescription className="flex flex-wrap items-center gap-3">
								<span>{total.toLocaleString()} {t('matchingIssues', 'matching issues')}</span>
								<span className="inline-flex items-center gap-1">
									<Zap className="h-3 w-3" />
									{totalEvents.toLocaleString()} {t('eventsOnThisPage', 'events on this page')}
								</span>
								<span className="inline-flex items-center gap-1">
									<MonitorSmartphone className="h-3 w-3" />
									{totalInstalls.toLocaleString()} {t('installsAffected', 'installs affected')}
								</span>
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							{issues.isLoading ? (
								<div className="space-y-2 p-4">
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
								</div>
							) : (issues.data?.issues.length ?? 0) === 0 ? (
								<EmptyState
									message="No issues match the current filters."
									className="m-4 py-10 text-sm"
								/>
							) : (
								<div>
									{issues.data?.issues.map((issue) => (
										<IssueRow key={issue.id} issue={issue} onOpen={openIssue} />
									))}
								</div>
							)}
						</CardContent>
					</Card>

					{totalPages > 1 && (
						<div className="flex items-center justify-between">
							<div className="text-sm text-muted-foreground">
								{t('pagePageOfTotalpages', 'Page {{page}} of {{totalPages}}', { page: page + 1, totalPages })}</div>
							<div className="flex gap-2">
								<Button
									variant="outline"
									size="sm"
									onClick={() => setPage((p) => Math.max(0, p - 1))}
									disabled={page === 0}
								>
									{t('previous', 'Previous')}
								</Button>
								<Button
									variant="outline"
									size="sm"
									onClick={() =>
										setPage((p) => Math.min(totalPages - 1, p + 1))
									}
									disabled={page >= totalPages - 1}
								>
									{t('next', 'Next')}
								</Button>
							</div>
						</div>
					)}
				</div>
			</div>

			<TelemetryIssueDetailSheet
				issueId={selectedIssue}
				open={showDetail}
				onOpenChange={setShowDetail}
				profile={profile.data}
			/>
		</main>
	);
}
