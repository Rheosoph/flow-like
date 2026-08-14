"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	BookmarkPlus,
	LayoutDashboard,
	Lock,
	RefreshCw,
	SlidersHorizontal,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useInvoke } from "../../../../hooks/use-invoke";
import { GlobalPermission } from "../../../../lib/permission/global-permission";
import { useBackend } from "../../../../state/backend-state";
import {
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	RelativeTime,
	Skeleton,
} from "../../../ui";
import { TelemetryQueryBuilderForm } from "./query-builder-form";
import {
	TelemetryQueryResultView,
	useTelemetryQueryResult,
} from "./query-result-view";
import {
	type ITelemetryQueryRequest,
	type ITelemetryQueryView,
	type ITelemetrySavedQueriesResponse,
	type ITelemetrySavedQuery,
	defaultTelemetryQuery,
	describeTelemetryQuery,
	normalizeTelemetryQuery,
	validateTelemetryQuery,
} from "./query-types";
import { EmptyState } from "./telemetry-shared";

const SAVED_QUERIES_KEY = ["admin", "telemetry", "saved-queries"];

function SavedQueryList({
	queries,
	loading,
	activeId,
	onLoad,
	onDelete,
}: {
	readonly queries: ITelemetrySavedQuery[];
	readonly loading: boolean;
	readonly activeId: string | null;
	readonly onLoad: (query: ITelemetrySavedQuery) => void;
	readonly onDelete: (query: ITelemetrySavedQuery) => void;
}) {
	const { t } = useTranslation("admin");
	if (loading) {
		return (
			<div className="space-y-1.5">
				<Skeleton className="h-10 w-full" />
				<Skeleton className="h-10 w-full" />
				<Skeleton className="h-10 w-full" />
			</div>
		);
	}

	if (queries.length === 0) {
		return (
			<EmptyState message="No saved queries yet — build one and save it." />
		);
	}

	return (
		<ul className="space-y-1">
			{queries.map((query) => (
				<li
					key={query.id}
					className={`flex items-center gap-1 rounded-lg border px-2 py-1.5 ${
						activeId === query.id ? "border-primary/50 bg-primary/5" : ""
					}`}
				>
					<button
						type="button"
						className="min-w-0 flex-1 text-left"
						onClick={() => onLoad(query)}
					>
						<span className="block truncate text-xs font-medium">
							{query.name}
						</span>
						<span className="block truncate text-[10px] text-muted-foreground">
							{describeTelemetryQuery(query.definition)}
						</span>
					</button>
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7 shrink-0"
						onClick={() => onDelete(query)}
						aria-label={`Delete ${query.name}`}
					>
						<Trash2 className="h-3.5 w-3.5" />
					</Button>
				</li>
			))}
		</ul>
	);
}

export function AdminTelemetryQueryBuilderPage() {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const auth = useAuth();
	const queryClient = useQueryClient();

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

	const [draft, setDraft] = useState<ITelemetryQueryRequest>(
		defaultTelemetryQuery,
	);
	const [submitted, setSubmitted] = useState<ITelemetryQueryRequest>(
		defaultTelemetryQuery,
	);
	const [view, setView] = useState<ITelemetryQueryView>("chart");
	const [activeSavedId, setActiveSavedId] = useState<string | null>(null);
	const [saveOpen, setSaveOpen] = useState(false);
	const [saveName, setSaveName] = useState("");

	const errors = useMemo(() => validateTelemetryQuery(draft), [draft]);
	const result = useTelemetryQueryResult(profile.data, submitted);

	const saved = useQuery<ITelemetrySavedQueriesResponse>({
		queryKey: SAVED_QUERIES_KEY,
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetrySavedQueriesResponse>(
				profile.data,
				"admin/telemetry/saved-queries",
			);
		},
		enabled: !!profile.data,
	});

	const createSaved = useMutation({
		mutationFn: async (body: {
			name: string;
			definition: ITelemetryQueryRequest;
		}) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<ITelemetrySavedQuery>(
				profile.data,
				"admin/telemetry/saved-queries",
				body,
			);
		},
		onSuccess: async (query) => {
			await queryClient.invalidateQueries({ queryKey: SAVED_QUERIES_KEY });
			setActiveSavedId(query?.id ?? null);
			setSaveOpen(false);
			toast.success("Query saved");
		},
		onError: (error) => {
			toast.error(
				error instanceof Error ? error.message : t('failedToSaveTheQuery', 'Failed to save the query'),
			);
		},
	});

	const deleteSaved = useMutation({
		mutationFn: async (id: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.del(
				profile.data,
				`admin/telemetry/saved-queries/${encodeURIComponent(id)}`,
			);
		},
		onSuccess: async (_data, id) => {
			await queryClient.invalidateQueries({ queryKey: SAVED_QUERIES_KEY });
			setActiveSavedId((current) => (current === id ? null : current));
			toast.success("Query deleted");
		},
		onError: (error) => {
			toast.error(
				error instanceof Error ? error.message : t('failedToDeleteTheQuery', 'Failed to delete the query'),
			);
		},
	});

	const run = useCallback(() => {
		if (errors.length > 0) return;
		setSubmitted(normalizeTelemetryQuery(draft));
	}, [draft, errors.length]);

	const loadSaved = useCallback((query: ITelemetrySavedQuery) => {
		const definition = normalizeTelemetryQuery(query.definition);
		setDraft(definition);
		setSubmitted(definition);
		setActiveSavedId(query.id);
	}, []);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "telemetry", "query"],
		});
	}, [queryClient]);

	const perms = useMemo(
		() => new GlobalPermission(info.data?.permission ?? 0),
		[info.data?.permission],
	);
	const hasAccess = perms.hasPermission(GlobalPermission.Admin);

	if (info.isLoading) {
		return (
			<main className="flex h-full min-h-0 w-full grow flex-col bg-background p-6">
				<Skeleton className="h-12 w-72" />
				<div className="mt-4 grid gap-4 lg:grid-cols-[22rem_1fr]">
					<Skeleton className="h-96 w-full" />
					<Skeleton className="h-96 w-full" />
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
						<CardDescription><Trans i18nKey="youNeedTheBadminbPermissionToRunTelemetryQueries">You need the <b>Admin</b> permission to run telemetry queries.</Trans></CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	const savedQueries = saved.data?.savedQueries ?? [];
	const activeSaved = savedQueries.find((query) => query.id === activeSavedId);

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<SlidersHorizontal className="h-7 w-7 text-primary" />
								{t('queryBuilder', 'Query builder')}
							</h1>
							<p className="text-muted-foreground">
								{`Compose ad-hoc breakdowns over anonymous telemetry — pick a dataset, a metric and filters from the allowed fields.`}
							</p>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							<Button asChild variant="ghost" size="sm">
								<Link href="/admin/telemetry">
									<ArrowLeft className="mr-1 h-3.5 w-3.5" />
									{t('telemetry', 'Telemetry')}
								</Link>
							</Button>
							<Button asChild variant="ghost" size="sm">
								<Link href="/admin/telemetry/dashboards">
									<LayoutDashboard className="mr-1 h-3.5 w-3.5" />
									{t('dashboards', 'Dashboards')}
								</Link>
							</Button>
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								{t('refresh', 'Refresh')}
							</Button>
						</div>
					</div>

					<div className="grid gap-4 lg:grid-cols-[22rem_1fr]">
						<div className="space-y-4">
							<Card>
								<CardHeader className="pb-3">
									<CardTitle className="text-base">{t('query', 'Query')}</CardTitle>
									<CardDescription>
										{t('fieldsAreLimitedToTheServerAllowlistForTheSelectedDataset', "Fields are limited to the server allowlist for the selected dataset.")}
									</CardDescription>
								</CardHeader>
								<CardContent>
									<TelemetryQueryBuilderForm
										value={draft}
										onChange={setDraft}
										onRun={run}
										running={result.isFetching}
										errors={errors}
									/>
								</CardContent>
							</Card>

							<Card>
								<CardHeader className="pb-3">
									<div className="flex items-center justify-between gap-2">
										<CardTitle className="text-base">{t('savedQueries', 'Saved queries')}</CardTitle>
										<Button
											variant="outline"
											size="sm"
											disabled={errors.length > 0}
											onClick={() => {
												setSaveName(activeSaved?.name ?? "");
												setSaveOpen(true);
											}}
										>
											<BookmarkPlus className="mr-1 h-3.5 w-3.5" />
											{t('save', 'Save')}
										</Button>
									</div>
								</CardHeader>
								<CardContent>
									<SavedQueryList
										queries={savedQueries}
										loading={saved.isLoading}
										activeId={activeSavedId}
										onLoad={loadSaved}
										onDelete={(query) => deleteSaved.mutate(query.id)}
									/>
								</CardContent>
							</Card>
						</div>

						<Card className="min-w-0">
							<CardHeader className="pb-3">
								<div className="flex flex-wrap items-start justify-between gap-2">
									<div className="min-w-0 space-y-1">
										<CardTitle className="truncate text-base">
											{activeSaved?.name ?? "Results"}
										</CardTitle>
										<CardDescription className="truncate">
											{describeTelemetryQuery(submitted)}
										</CardDescription>
									</div>
									{activeSaved ? (
										<span className="text-[11px] text-muted-foreground">
											{t('saved', 'Saved')} <RelativeTime value={activeSaved.updatedAt} />
										</span>
									) : null}
								</div>
							</CardHeader>
							<CardContent>
								<TelemetryQueryResultView
									request={submitted}
									response={result.data}
									loading={result.isLoading || result.isFetching}
									error={result.error}
									view={view}
									onViewChange={setView}
									name={activeSaved?.name ?? "telemetry-query"}
								/>
							</CardContent>
						</Card>
					</div>
				</div>
			</div>

			<Dialog open={saveOpen} onOpenChange={setSaveOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>{t('saveQuery', 'Save query')}</DialogTitle>
						<DialogDescription>
							{describeTelemetryQuery(draft)}
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-1.5">
						<Label htmlFor="telemetry-save-query-name">Name</Label>
						<Input
							id="telemetry-save-query-name"
							value={saveName}
							onChange={(e) => setSaveName(e.target.value)}
							placeholder={t('crashRateByRelease', 'Crash rate by release')}
						/>
					</div>
					<DialogFooter>
						<Button variant="ghost" onClick={() => setSaveOpen(false)}>
							{t('cancel', 'Cancel')}
						</Button>
						<Button
							disabled={saveName.trim().length === 0 || createSaved.isPending}
							onClick={() =>
								createSaved.mutate({
									name: saveName.trim(),
									definition: normalizeTelemetryQuery(draft),
								})
							}
						>
							{t('save', 'Save')}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</main>
	);
}
