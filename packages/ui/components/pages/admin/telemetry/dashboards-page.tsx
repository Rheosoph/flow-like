"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowDown,
	ArrowLeft,
	ArrowUp,
	BarChart3,
	Download,
	LayoutDashboard,
	Lock,
	Plus,
	RefreshCw,
	SlidersHorizontal,
	Table2,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useInvoke } from "../../../../hooks/use-invoke";
import { GlobalPermission } from "../../../../lib/permission/global-permission";
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
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
} from "../../../ui";
import {
	TelemetryQueryResultView,
	downloadTelemetryQueryCsv,
	useTelemetryQueryResult,
} from "./query-result-view";
import {
	type ITelemetryDashboard,
	type ITelemetryDashboardTile,
	type ITelemetryDashboardTileWidth,
	type ITelemetryDashboardsResponse,
	type ITelemetryQueryView,
	type ITelemetrySavedQueriesResponse,
	type ITelemetrySavedQuery,
	TELEMETRY_DASHBOARD_MAX_TILES,
	TELEMETRY_DASHBOARD_MAX_TILE_TITLE,
	describeTelemetryQuery,
	normalizeTelemetryQuery,
} from "./query-types";
import { EmptyState } from "./telemetry-shared";

const DASHBOARDS_KEY = ["admin", "telemetry", "dashboards"];

const SAVED_QUERIES_KEY = ["admin", "telemetry", "saved-queries"];

function createTileId(): string {
	if (
		typeof crypto !== "undefined" &&
		typeof crypto.randomUUID === "function"
	) {
		return crypto.randomUUID();
	}
	return `tile-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function moveTile(
	tiles: readonly ITelemetryDashboardTile[],
	index: number,
	delta: number,
): ITelemetryDashboardTile[] {
	const target = index + delta;
	if (target < 0 || target >= tiles.length) return [...tiles];
	const next = [...tiles];
	const [moved] = next.splice(index, 1);
	next.splice(target, 0, moved);
	return next;
}

function DashboardTile({
	tile,
	index,
	total,
	savedQuery,
	profile,
	onMove,
	onUpdate,
	onRemove,
}: {
	readonly tile: ITelemetryDashboardTile;
	readonly index: number;
	readonly total: number;
	readonly savedQuery: ITelemetrySavedQuery | undefined;
	readonly profile: IProfile | undefined;
	readonly onMove: (index: number, delta: number) => void;
	readonly onUpdate: (index: number, next: ITelemetryDashboardTile) => void;
	readonly onRemove: (index: number) => void;
}) {
	const { t } = useTranslation("admin");
	const request = useMemo(
		() => (savedQuery ? normalizeTelemetryQuery(savedQuery.definition) : null),
		[savedQuery],
	);
	const result = useTelemetryQueryResult(profile, request);

	return (
		<Card className={`min-w-0 ${tile.width === "full" ? "md:col-span-2" : ""}`}>
			<CardHeader className="pb-3">
				<div className="flex flex-wrap items-start justify-between gap-2">
					<div className="min-w-0 space-y-1">
						<CardTitle className="truncate text-sm">{tile.title}</CardTitle>
						<CardDescription className="truncate text-[11px]">
							{savedQuery
								? describeTelemetryQuery(savedQuery.definition)
								: t('theSavedQueryBehindThisTileNoLongerExists', 'The saved query behind this tile no longer exists.')}
						</CardDescription>
					</div>
					<div className="flex shrink-0 items-center gap-0.5">
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							disabled={index === 0}
							onClick={() => onMove(index, -1)}
							aria-label={t('moveTileUp', 'Move tile up')}
						>
							<ArrowUp className="h-3.5 w-3.5" />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							disabled={index >= total - 1}
							onClick={() => onMove(index, 1)}
							aria-label={t('moveTileDown', 'Move tile down')}
						>
							<ArrowDown className="h-3.5 w-3.5" />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							onClick={() =>
								onUpdate(index, {
									...tile,
									view: tile.view === "chart" ? "table" : "chart",
								})
							}
							aria-label={t('toggleTileView', 'Toggle tile view')}
						>
							{tile.view === "chart" ? (
								<Table2 className="h-3.5 w-3.5" />
							) : (
								<BarChart3 className="h-3.5 w-3.5" />
							)}
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							disabled={!result.data || result.data.rows.length === 0}
							onClick={() =>
								result.data &&
								downloadTelemetryQueryCsv(tile.title, result.data)
							}
							aria-label={t('downloadTileCsv', 'Download tile CSV')}
						>
							<Download className="h-3.5 w-3.5" />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							onClick={() => onRemove(index)}
							aria-label={t('removeTile', 'Remove tile')}
						>
							<Trash2 className="h-3.5 w-3.5" />
						</Button>
					</div>
				</div>
			</CardHeader>
			<CardContent>
				{request ? (
					<TelemetryQueryResultView
						request={request}
						response={result.data}
						loading={result.isLoading}
						error={result.error}
						view={tile.view}
						name={tile.title}
						compact
						showToolbar={false}
					/>
				) : (
					<EmptyState message="Saved query missing — remove this tile." />
				)}
			</CardContent>
		</Card>
	);
}

export function AdminTelemetryDashboardsPage() {
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

	const [activeId, setActiveId] = useState<string | null>(null);
	const [createOpen, setCreateOpen] = useState(false);
	const [createName, setCreateName] = useState("");
	const [tileOpen, setTileOpen] = useState(false);
	const [tileQueryId, setTileQueryId] = useState<string>("");
	const [tileTitle, setTileTitle] = useState("");
	const [tileWidth, setTileWidth] =
		useState<ITelemetryDashboardTileWidth>("half");
	const [tileView, setTileView] = useState<ITelemetryQueryView>("chart");

	const dashboards = useQuery<ITelemetryDashboardsResponse>({
		queryKey: DASHBOARDS_KEY,
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryDashboardsResponse>(
				profile.data,
				"admin/telemetry/dashboards",
			);
		},
		enabled: !!profile.data,
	});

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

	const rows = useMemo(
		() => dashboards.data?.dashboards ?? [],
		[dashboards.data?.dashboards],
	);
	const savedQueries = saved.data?.savedQueries ?? [];

	useEffect(() => {
		if (rows.length === 0) return;
		if (activeId && rows.some((row) => row.id === activeId)) return;
		setActiveId(rows[0].id);
	}, [activeId, rows]);

	const active = rows.find((row) => row.id === activeId);
	const tiles = useMemo(() => active?.tiles ?? [], [active?.tiles]);

	const createDashboard = useMutation({
		mutationFn: async (name: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<ITelemetryDashboard>(
				profile.data,
				"admin/telemetry/dashboards",
				{ name, tiles: [] },
			);
		},
		onSuccess: async (dashboard) => {
			await queryClient.invalidateQueries({ queryKey: DASHBOARDS_KEY });
			setActiveId(dashboard?.id ?? null);
			setCreateOpen(false);
			setCreateName("");
			toast.success("Dashboard created");
		},
		onError: (error) => {
			toast.error(
				error instanceof Error
					? error.message
					: t('failedToCreateTheDashboard', 'Failed to create the dashboard'),
			);
		},
	});

	const updateDashboard = useMutation({
		mutationFn: async (body: {
			id: string;
			tiles: ITelemetryDashboardTile[];
		}) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.patch<ITelemetryDashboard>(
				profile.data,
				`admin/telemetry/dashboards/${encodeURIComponent(body.id)}`,
				{ tiles: body.tiles },
			);
		},
		onMutate: async (body) => {
			await queryClient.cancelQueries({ queryKey: DASHBOARDS_KEY });
			const previous =
				queryClient.getQueryData<ITelemetryDashboardsResponse>(DASHBOARDS_KEY);
			queryClient.setQueryData<ITelemetryDashboardsResponse>(
				DASHBOARDS_KEY,
				(prev) =>
					prev
						? {
								dashboards: prev.dashboards.map((dashboard) =>
									dashboard.id === body.id
										? { ...dashboard, tiles: body.tiles }
										: dashboard,
								),
							}
						: prev,
			);
			return { previous };
		},
		onError: (error, _body, context) => {
			if (context?.previous) {
				queryClient.setQueryData(DASHBOARDS_KEY, context.previous);
			}
			toast.error(
				error instanceof Error ? error.message : t('failedToSaveTheLayout', 'Failed to save the layout'),
			);
		},
		onSettled: async () => {
			await queryClient.invalidateQueries({ queryKey: DASHBOARDS_KEY });
		},
	});

	const deleteDashboard = useMutation({
		mutationFn: async (id: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.del(
				profile.data,
				`admin/telemetry/dashboards/${encodeURIComponent(id)}`,
			);
		},
		onSuccess: async (_data, id) => {
			await queryClient.invalidateQueries({ queryKey: DASHBOARDS_KEY });
			setActiveId((current) => (current === id ? null : current));
			toast.success("Dashboard deleted");
		},
		onError: (error) => {
			toast.error(
				error instanceof Error
					? error.message
					: t('failedToDeleteTheDashboard', 'Failed to delete the dashboard'),
			);
		},
	});

	const applyTiles = useCallback(
		(next: ITelemetryDashboardTile[]) => {
			if (!active) return;
			updateDashboard.mutate({ id: active.id, tiles: next });
		},
		[active, updateDashboard],
	);

	const onMove = useCallback(
		(index: number, delta: number) => applyTiles(moveTile(tiles, index, delta)),
		[applyTiles, tiles],
	);

	const onUpdateTile = useCallback(
		(index: number, next: ITelemetryDashboardTile) =>
			applyTiles(tiles.map((tile, i) => (i === index ? next : tile))),
		[applyTiles, tiles],
	);

	const onRemoveTile = useCallback(
		(index: number) => applyTiles(tiles.filter((_, i) => i !== index)),
		[applyTiles, tiles],
	);

	const addTile = useCallback(() => {
		const source = savedQueries.find((query) => query.id === tileQueryId);
		if (!source) return;
		applyTiles([
			...tiles,
			{
				id: createTileId(),
				savedQueryId: source.id,
				title: tileTitle.trim() || source.name,
				width: tileWidth,
				view: tileView,
			},
		]);
		setTileOpen(false);
		setTileTitle("");
	}, [
		applyTiles,
		savedQueries,
		tileQueryId,
		tileTitle,
		tileView,
		tileWidth,
		tiles,
	]);

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
				<div className="mt-4 grid gap-4 lg:grid-cols-[18rem_1fr]">
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
						<CardDescription><Trans i18nKey="youNeedTheBadminbPermissionToManageTelemetryDashboards">You need the <b>Admin</b> permission to manage telemetry
							dashboards.</Trans></CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<LayoutDashboard className="h-7 w-7 text-primary" />
								{t('telemetryDashboards', 'Telemetry dashboards')}
							</h1>
							<p className="text-muted-foreground">
								{t('pinSavedQueriesAsTilesAndArrangeThemIntoASharedBoard', 'Pin saved queries as tiles and arrange them into a shared board.')}
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
								<Link href="/admin/telemetry/query">
									<SlidersHorizontal className="mr-1 h-3.5 w-3.5" />
									{t('queryBuilder', 'Query builder')}
								</Link>
							</Button>
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								{t('refresh', 'Refresh')}
							</Button>
						</div>
					</div>

					<div className="grid gap-4 lg:grid-cols-[18rem_1fr]">
						<Card className="h-fit">
							<CardHeader className="pb-3">
								<div className="flex items-center justify-between gap-2">
									<CardTitle className="text-base">{t('dashboards', 'Dashboards')}</CardTitle>
									<Button
										variant="outline"
										size="sm"
										onClick={() => setCreateOpen(true)}
									>
										<Plus className="mr-1 h-3.5 w-3.5" />
										{t('new', 'New')}
									</Button>
								</div>
							</CardHeader>
							<CardContent>
								{dashboards.isLoading ? (
									<div className="space-y-1.5">
										<Skeleton className="h-9 w-full" />
										<Skeleton className="h-9 w-full" />
										<Skeleton className="h-9 w-full" />
									</div>
								) : rows.length === 0 ? (
									<EmptyState message="No dashboards yet — create one to start pinning tiles." />
								) : (
									<ul className="space-y-1">
										{rows.map((dashboard) => (
											<li key={dashboard.id}>
												<button
													type="button"
													onClick={() => setActiveId(dashboard.id)}
													className={`flex w-full items-center justify-between gap-2 rounded-lg border px-2 py-1.5 text-left ${
														dashboard.id === activeId
															? "border-primary/50 bg-primary/5"
															: ""
													}`}
												>
													<span className="truncate text-xs font-medium">
														{dashboard.name}
													</span>
													<Badge
														variant="outline"
														className="shrink-0 text-[10px] tabular-nums"
													>
														{(dashboard.tiles ?? []).length}
													</Badge>
												</button>
											</li>
										))}
									</ul>
								)}
							</CardContent>
						</Card>

						<div className="min-w-0 space-y-4">
							{active ? (
								<>
									<div className="flex flex-wrap items-center justify-between gap-2">
										<h2 className="truncate text-xl font-semibold">
											{active.name}
										</h2>
										<div className="flex items-center gap-2">
											<Button
												variant="outline"
												size="sm"
												disabled={
													savedQueries.length === 0 ||
													tiles.length >= TELEMETRY_DASHBOARD_MAX_TILES
												}
												title={
													tiles.length >= TELEMETRY_DASHBOARD_MAX_TILES
														? t('aDashboardCarriesAtMostTelemetry_dashboard_max_tilesTiles', 'A dashboard carries at most {{TELEMETRY_DASHBOARD_MAX_TILES}} tiles.', { TELEMETRY_DASHBOARD_MAX_TILES })
														: undefined
												}
												onClick={() => {
													setTileQueryId(savedQueries[0]?.id ?? "");
													setTileTitle("");
													setTileWidth("half");
													setTileView("chart");
													setTileOpen(true);
												}}
											>
												<Plus className="mr-1 h-3.5 w-3.5" />
												{t('addTile', 'Add tile')}
											</Button>
											<Button
												variant="ghost"
												size="sm"
												onClick={() => deleteDashboard.mutate(active.id)}
											>
												<Trash2 className="mr-1 h-3.5 w-3.5" />
												{t('delete', 'Delete')}
											</Button>
										</div>
									</div>

									{tiles.length === 0 ? (
										<EmptyState
											message={
												t('noTilesYetAddASavedQueryToThisDashboard', { defaultValue_zero: 'Save a query in the query builder first, then pin it here.', defaultValue_other: 'No tiles yet — add a saved query to this dashboard.', count: savedQueries.length })
											}
											className="py-12 text-sm"
										/>
									) : (
										<div className="grid gap-4 md:grid-cols-2">
											{tiles.map((tile, index) => (
												<DashboardTile
													key={tile.id}
													tile={tile}
													index={index}
													total={tiles.length}
													savedQuery={savedQueries.find(
														(query) => query.id === tile.savedQueryId,
													)}
													profile={profile.data}
													onMove={onMove}
													onUpdate={onUpdateTile}
													onRemove={onRemoveTile}
												/>
											))}
										</div>
									)}
								</>
							) : (
								<EmptyState
									message="Select or create a dashboard to get started."
									className="py-12 text-sm"
								/>
							)}
						</div>
					</div>
				</div>
			</div>

			<Dialog open={createOpen} onOpenChange={setCreateOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>{t('newDashboard', 'New dashboard')}</DialogTitle>
						<DialogDescription>
							{t('dashboardsGroupSavedTelemetryQueriesIntoOneView', 'Dashboards group saved telemetry queries into one view.')}
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-1.5">
						<Label htmlFor="telemetry-dashboard-name">Name</Label>
						<Input
							id="telemetry-dashboard-name"
							value={createName}
							onChange={(e) => setCreateName(e.target.value)}
							placeholder={t('releaseHealth', 'Release health')}
						/>
					</div>
					<DialogFooter>
						<Button variant="ghost" onClick={() => setCreateOpen(false)}>
							{t('cancel', 'Cancel')}
						</Button>
						<Button
							disabled={
								createName.trim().length === 0 || createDashboard.isPending
							}
							onClick={() => createDashboard.mutate(createName.trim())}
						>
							{t('create', 'Create')}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<Dialog open={tileOpen} onOpenChange={setTileOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>{t('addTile', 'Add tile')}</DialogTitle>
						<DialogDescription>
							{t('pinASavedQueryTo', 'Pin a saved query to')} {active?.name ?? "this dashboard"}.
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-3">
						<div className="space-y-1.5">
							<Label>{t('savedQuery', 'Saved query')}</Label>
							<Select value={tileQueryId} onValueChange={setTileQueryId}>
								<SelectTrigger className="w-full">
									<SelectValue placeholder={t('savedQuery', 'Saved query')} />
								</SelectTrigger>
								<SelectContent>
									{savedQueries.map((query) => (
										<SelectItem key={query.id} value={query.id}>
											{query.name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1.5">
							<Label htmlFor="telemetry-tile-title">{t('title', 'Title')}</Label>
							<Input
								id="telemetry-tile-title"
								value={tileTitle}
								onChange={(e) => setTileTitle(e.target.value)}
								maxLength={TELEMETRY_DASHBOARD_MAX_TILE_TITLE}
								placeholder={
									savedQueries.find((query) => query.id === tileQueryId)
										?.name ?? "Tile title"
								}
							/>
						</div>
						<div className="grid gap-3 sm:grid-cols-2">
							<div className="space-y-1.5">
								<Label>{t('width', 'Width')}</Label>
								<Select
									value={tileWidth}
									onValueChange={(v) =>
										setTileWidth(v as ITelemetryDashboardTileWidth)
									}
								>
									<SelectTrigger className="w-full">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="half">{t('halfWidth', 'Half width')}</SelectItem>
										<SelectItem value="full">{t('fullWidth', 'Full width')}</SelectItem>
									</SelectContent>
								</Select>
							</div>
							<div className="space-y-1.5">
								<Label>{t('view', 'View')}</Label>
								<Select
									value={tileView}
									onValueChange={(v) => setTileView(v as ITelemetryQueryView)}
								>
									<SelectTrigger className="w-full">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="chart">{t('chart', 'Chart')}</SelectItem>
										<SelectItem value="table">{t('table', 'Table')}</SelectItem>
									</SelectContent>
								</Select>
							</div>
						</div>
					</div>
					<DialogFooter>
						<Button variant="ghost" onClick={() => setTileOpen(false)}>
							{t('cancel', 'Cancel')}
						</Button>
						<Button disabled={!tileQueryId} onClick={addTile}>
							{t('addTile', 'Add tile')}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</main>
	);
}
