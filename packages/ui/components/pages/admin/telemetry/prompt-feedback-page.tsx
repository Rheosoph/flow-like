"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	ArrowLeft,
	Gauge,
	Lock,
	MessageSquare,
	MessagesSquare,
	RefreshCw,
	Search,
	Sparkles,
	ThumbsDown,
	ThumbsUp,
	TriangleAlert,
	Users,
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
import { PromptFeedbackDetailSheet } from "./prompt-feedback-detail-sheet";
import {
	PromptFeedbackRatingBadge,
	isFailedOutcome,
} from "./prompt-feedback-shared";
import { EmptyState, StatTile } from "./telemetry-shared";
import { formatDurationMs } from "./traces-shared";
import type {
	IPromptFeedbackFacetCount,
	IPromptFeedbackRecord,
	IPromptFeedbackResponse,
} from "./types";

const PAGE_SIZE = 25;

const DEFAULT_HOURS = 720;

const ALL = "all";

const RATING_OPTIONS = ["positive", "negative"] as const;

const SUMMARY_TILES = [
	"total",
	"positive",
	"negative",
	"satisfaction",
	"raters",
	"conversations",
	"withComment",
] as const;

interface PromptFeedbackFilterState {
	hours: number;
	rating: string;
	model: string;
	provider: string;
	outcome: string;
	query: string;
}

function decodeFiltersFromSearch(params: {
	get(name: string): string | null;
}): PromptFeedbackFilterState {
	const rating = params.get("rating") ?? "";
	return {
		hours:
			Number.parseInt(params.get("hours") ?? String(DEFAULT_HOURS), 10) ||
			DEFAULT_HOURS,
		rating: (RATING_OPTIONS as readonly string[]).includes(rating)
			? rating
			: ALL,
		model: params.get("model") || ALL,
		provider: params.get("provider") || ALL,
		outcome: params.get("outcome") || ALL,
		query: params.get("query") ?? "",
	};
}

function encodeFiltersToSearch(
	filters: PromptFeedbackFilterState,
	feedbackId: string | null,
): string {
	const p = new URLSearchParams();
	if (filters.hours !== DEFAULT_HOURS) p.set("hours", String(filters.hours));
	if (filters.rating !== ALL) p.set("rating", filters.rating);
	if (filters.model !== ALL) p.set("model", filters.model);
	if (filters.provider !== ALL) p.set("provider", filters.provider);
	if (filters.outcome !== ALL) p.set("outcome", filters.outcome);
	if (filters.query) p.set("query", filters.query);
	if (feedbackId) p.set("feedback", feedbackId);
	return p.toString();
}

/** Keeps a value that came in from the URL selectable before the window's facets have loaded. */
function withSelected(
	options: readonly string[] | undefined,
	selected: string,
) {
	const list = options ? [...options] : [];
	if (selected !== ALL && !list.includes(selected)) list.push(selected);
	return list;
}

function FacetBreakdown({
	rows,
	emptyMessage,
}: {
	readonly rows: IPromptFeedbackFacetCount[];
	readonly emptyMessage: string;
}) {
	const { t } = useTranslation("admin");
	const max = Math.max(1, ...rows.map((r) => r.count));
	if (rows.length === 0) return <EmptyState message={emptyMessage} />;
	return (
		<ul className="space-y-2.5">
			{rows.map((row) => (
				<li key={row.key} className="space-y-1">
					<div className="flex items-center justify-between gap-2 text-xs">
						<span className="truncate font-mono font-medium" title={row.key}>
							{row.key}
						</span>
						<span className="shrink-0 tabular-nums text-muted-foreground">
							{t(
								"ratedNegativeCounts",
								"{{rated}} rated · {{negative}} negative",
								{
									rated: row.count.toLocaleString(),
									negative: row.negative.toLocaleString(),
								},
							)}
						</span>
					</div>
					<div className="relative h-2 overflow-hidden rounded-full bg-muted">
						<div
							className="absolute inset-y-0 left-0 rounded-full bg-primary/50"
							style={{ width: `${(row.count / max) * 100}%` }}
						/>
						<div
							className="absolute inset-y-0 left-0 rounded-full bg-destructive"
							style={{ width: `${(row.negative / max) * 100}%` }}
						/>
					</div>
				</li>
			))}
		</ul>
	);
}

function FeedbackRow({
	record,
	onOpen,
}: {
	readonly record: IPromptFeedbackRecord;
	readonly onOpen: (record: IPromptFeedbackRecord) => void;
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
					<PromptFeedbackRatingBadge rating={record.rating} />
					{record.model ? (
						<Badge variant="outline" className="font-mono text-[10px]">
							{record.model}
						</Badge>
					) : null}
					{record.provider ? (
						<span className="text-[11px] text-muted-foreground">
							{record.provider}
						</span>
					) : null}
					{record.outcome ? (
						<Badge
							variant={
								isFailedOutcome(record.outcome) ? "destructive" : "secondary"
							}
							className="text-[10px]"
						>
							{record.outcome}
						</Badge>
					) : null}
					{record.comment ? (
						<span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground">
							<MessageSquare className="h-3 w-3" />
							{t("commented", "commented")}
						</span>
					) : null}
				</div>
				<div className="line-clamp-2 text-sm font-medium">
					{record.promptPreview ||
						t("noPromptCaptured", "No prompt captured for this turn")}
				</div>
				{record.comment ? (
					<div
						className="line-clamp-2 text-xs italic text-muted-foreground"
						title={record.comment}
					>
						{record.comment}
					</div>
				) : null}
				<div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
					<RelativeTime value={record.createdAt} />
					{record.durationMs != null ? (
						<>
							<span>·</span>
							<span>{formatDurationMs(record.durationMs)}</span>
						</>
					) : null}
					{record.totalTokens != null ? (
						<>
							<span>·</span>
							<span>
								{t("valTokens", "{{val}} tokens", {
									val: record.totalTokens.toLocaleString(),
								})}
							</span>
						</>
					) : null}
					{record.autoMode ? (
						<>
							<span>·</span>
							<span>{t("autoModeInline", "auto mode")}</span>
						</>
					) : null}
					{record.surface ? (
						<>
							<span>·</span>
							<span className="font-mono">{record.surface}</span>
						</>
					) : null}
				</div>
			</div>
		</button>
	);
}

interface AdminPromptFeedbackPageProps {
	basePath?: string;
}

export function AdminPromptFeedbackPage({
	basePath = "/admin/telemetry/prompt-feedback",
}: Readonly<AdminPromptFeedbackPageProps>) {
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
	const initialFeedback = searchParams?.get("feedback") ?? null;

	const [filters, setFilters] =
		useState<PromptFeedbackFilterState>(initialFilters);
	const [page, setPage] = useState(0);
	const [selectedId, setSelectedId] = useState<string | null>(initialFeedback);
	const [showDetail, setShowDetail] = useState(Boolean(initialFeedback));

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
		if (filters.rating !== ALL) p.set("rating", filters.rating);
		if (filters.model !== ALL) p.set("model", filters.model);
		if (filters.provider !== ALL) p.set("provider", filters.provider);
		if (filters.outcome !== ALL) p.set("outcome", filters.outcome);
		if (debouncedQuery) p.set("query", debouncedQuery);
		return p.toString();
	}, [
		debouncedQuery,
		filters.hours,
		filters.model,
		filters.outcome,
		filters.provider,
		filters.rating,
		page,
	]);

	const feedback = useQuery<IPromptFeedbackResponse>({
		queryKey: ["admin", "telemetry", "prompt-feedback", "list", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<IPromptFeedbackResponse>(
				profile.data,
				`admin/telemetry/prompt-feedback?${queryParams}`,
			);
		},
		enabled: !!profile.data,
	});

	useEffect(() => {
		const qs = encodeFiltersToSearch(filters, showDetail ? selectedId : null);
		router.replace(qs ? `${basePath}?${qs}` : basePath);
	}, [filters, selectedId, showDetail, router, basePath]);

	const setFilterValue = useCallback(
		<K extends keyof PromptFeedbackFilterState>(
			key: K,
			value: PromptFeedbackFilterState[K],
		) => {
			setFilters((prev) => ({ ...prev, [key]: value }));
			setPage(0);
		},
		[],
	);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "telemetry", "prompt-feedback"],
		});
	}, [queryClient]);

	const openFeedback = useCallback((record: IPromptFeedbackRecord) => {
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
							<Trans i18nKey="youNeedTheAdminPermissionToViewPromptFeedback">
								You need the <b>Admin</b> permission to view prompt feedback.
							</Trans>
						</CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	const summary = feedback.data?.summary;
	const facets = feedback.data?.filters;
	const items = feedback.data?.items ?? [];
	const total = feedback.data?.total ?? 0;
	const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
	const satisfaction = summary?.satisfaction;

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<Sparkles className="h-7 w-7 text-primary" />
								{t("promptFeedback", "Prompt feedback")}
							</h1>
							<p className="text-muted-foreground">
								{t(
									"assistantTurnsUsersRatedWithTheModelProviderAndOutcomeThatProducedThem",
									"Assistant turns users rated, with the model, provider and outcome that produced them.",
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

					{/* Error before loading: once a failed request settles, `isLoading` is false while
					    `data` stays undefined — gating on `!summary` alone would shimmer forever and
					    the list below would report the failure as an empty window. */}
					{feedback.isError ? (
						<Alert variant="destructive">
							<TriangleAlert className="h-4 w-4" />
							<AlertTitle>
								{t(
									"couldNotLoadPromptFeedback",
									"Could not load prompt feedback",
								)}
							</AlertTitle>
							<AlertDescription className="flex flex-col items-start gap-2">
								<span>
									{feedback.error instanceof Error
										? feedback.error.message
										: t("theRequestFailed", "The request failed.")}
								</span>
								<Button
									variant="outline"
									size="sm"
									onClick={() => feedback.refetch()}
								>
									<RefreshCw className="mr-1 h-3.5 w-3.5" />
									{t("tryAgain", "Try again")}
								</Button>
							</AlertDescription>
						</Alert>
					) : feedback.isLoading || !summary ? (
						<div className="grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-7">
							{SUMMARY_TILES.map((tile) => (
								<Skeleton key={tile} className="h-16 w-full" />
							))}
						</div>
					) : (
						<div className="grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-7">
							<StatTile
								label={t("ratedTurns", "Rated turns")}
								value={summary.total.toLocaleString()}
								icon={<Sparkles className="h-4 w-4" />}
								hint={t(
									"matchingTheCurrentFilters",
									"Matching the current filters",
								)}
							/>
							<StatTile
								label={t("positive", "Positive")}
								value={summary.positive.toLocaleString()}
								icon={<ThumbsUp className="h-4 w-4" />}
							/>
							<StatTile
								label={t("negative", "Negative")}
								value={summary.negative.toLocaleString()}
								icon={<ThumbsDown className="h-4 w-4" />}
							/>
							<StatTile
								label={t("satisfaction", "Satisfaction")}
								value={
									satisfaction == null ? "—" : `${satisfaction.toFixed(1)}%`
								}
								icon={<Gauge className="h-4 w-4" />}
								hint={t(
									"shareOfRatingsThatWerePositive",
									"Share of ratings that were positive",
								)}
							/>
							<StatTile
								label={t("raters", "Raters")}
								value={summary.raters.toLocaleString()}
								icon={<Users className="h-4 w-4" />}
								hint={t("distinctUsers", "Distinct users")}
							/>
							<StatTile
								label={t("conversations", "Conversations")}
								value={summary.conversations.toLocaleString()}
								icon={<MessagesSquare className="h-4 w-4" />}
							/>
							<StatTile
								label={t("withComment", "With comment")}
								value={summary.withComment.toLocaleString()}
								icon={<MessageSquare className="h-4 w-4" />}
								hint={t("ratingsThatCarriedText", "Ratings that carried text")}
							/>
						</div>
					)}

					{feedback.data?.truncated ? (
						<Alert variant="destructive">
							<TriangleAlert />
							<AlertTitle>
								{t("windowHitItsScanCap", "Window hit its scan cap")}
							</AlertTitle>
							<AlertDescription>
								{t(
									"theSummaryAndFacetsCoverOnlyTheMostRecentRatingsInThisWindowNarrowTheTimeRangeForACompletePicture",
									"The summary and breakdowns below cover only the most recent ratings in this window. Narrow the time range for a complete picture.",
								)}
							</AlertDescription>
						</Alert>
					) : null}

					<div className="grid gap-4 lg:grid-cols-2">
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">
									{t("byModel", "By model")}
								</CardTitle>
								<CardDescription>
									{t(
										"whichModelsAreProducingRatedAnswersAndHowManyOfThoseWereNegative",
										"Which models are producing rated answers, and how many of those were negative.",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<FacetBreakdown
									rows={summary?.byModel ?? []}
									emptyMessage={t(
										"noDataInTheSelectedWindow",
										"No data in the selected window.",
									)}
								/>
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">
									{t("byOutcome", "By outcome")}
								</CardTitle>
								<CardDescription>
									{t(
										"howTheRatedTurnsEndedAndHowManyOfEachEndingWereNegative",
										"How the rated turns ended, and how many of each ending were negative.",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<FacetBreakdown
									rows={summary?.byOutcome ?? []}
									emptyMessage={t(
										"noDataInTheSelectedWindow",
										"No data in the selected window.",
									)}
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
											"searchPromptResponseOrComment",
											"Search prompt, response or comment",
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
									value={filters.rating}
									onValueChange={(v) => setFilterValue("rating", v)}
								>
									<SelectTrigger className="w-36">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>
											{t("allRatings", "All ratings")}
										</SelectItem>
										<SelectItem value="positive">
											{t("positive", "Positive")}
										</SelectItem>
										<SelectItem value="negative">
											{t("negative", "Negative")}
										</SelectItem>
									</SelectContent>
								</Select>
								<Select
									value={filters.model}
									onValueChange={(v) => setFilterValue("model", v)}
								>
									<SelectTrigger className="w-48">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>
											{t("allModels", "All models")}
										</SelectItem>
										{withSelected(facets?.models, filters.model).map(
											(model) => (
												<SelectItem key={model} value={model}>
													{model}
												</SelectItem>
											),
										)}
									</SelectContent>
								</Select>
								<Select
									value={filters.provider}
									onValueChange={(v) => setFilterValue("provider", v)}
								>
									<SelectTrigger className="w-40">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value={ALL}>
											{t("allProviders", "All providers")}
										</SelectItem>
										{withSelected(facets?.providers, filters.provider).map(
											(provider) => (
												<SelectItem key={provider} value={provider}>
													{provider}
												</SelectItem>
											),
										)}
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
										{withSelected(facets?.outcomes, filters.outcome).map(
											(outcome) => (
												<SelectItem key={outcome} value={outcome}>
													{outcome}
												</SelectItem>
											),
										)}
									</SelectContent>
								</Select>
							</div>
							<CardDescription className="flex flex-wrap items-center gap-3">
								<span>
									{t("valMatchingRatings", "{{val}} matching ratings", {
										val: total.toLocaleString(),
									})}
								</span>
								<span className="inline-flex items-center gap-1">
									<ThumbsDown className="h-3 w-3" />
									{t("valNegative", "{{val}} negative", {
										val: (summary?.negative ?? 0).toLocaleString(),
									})}
								</span>
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							{feedback.isLoading ? (
								<div className="space-y-2 p-4">
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
								</div>
							) : feedback.isError ? (
								<EmptyState
									message={t(
										"theRatingsCouldNotBeLoaded",
										"The ratings could not be loaded.",
									)}
									className="m-4 py-10 text-sm"
								/>
							) : items.length === 0 ? (
								<EmptyState
									message={t(
										"noRatingsInThisWindowUsersHaveNotRatedAnyAssistantTurnsForTheSelectedFilters",
										"No ratings in this window — nobody rated an assistant turn matching these filters.",
									)}
									className="m-4 py-10 text-sm"
								/>
							) : (
								<div>
									{items.map((record) => (
										<FeedbackRow
											key={record.id}
											record={record}
											onOpen={openFeedback}
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

			<PromptFeedbackDetailSheet
				feedbackId={selectedId}
				open={showDetail}
				onOpenChange={setShowDetail}
				profile={profile.data}
			/>
		</main>
	);
}
