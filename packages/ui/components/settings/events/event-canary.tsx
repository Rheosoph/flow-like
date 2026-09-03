"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ActivityIcon,
	AlertTriangleIcon,
	CloudIcon,
	EyeIcon,
	LayoutIcon,
	Loader2Icon,
	PencilIcon,
	PinIcon,
	PlusIcon,
	RadioTowerIcon,
	RocketIcon,
	SplitIcon,
	Trash2Icon,
	WrenchIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { formatDuration } from "../../../lib/date";
import type { IBoardSummary } from "../../../lib/schema/flow/board-summary";
import {
	type BoardVersion,
	normalizeBoardVersion,
} from "../../../lib/schema/flow/board-version";
import type {
	IEvent,
	IEventVariant,
	ISystemTime,
} from "../../../lib/schema/flow/event";
import { useBackend } from "../../../state/backend-state";
import type {
	ICanaryExplainResult,
	IEventSetupInfo,
	IEventVariantStatsResult,
	IEventVariantStatsWindow,
} from "../../../state/backend-state/event-state";
import type {
	IPageBootstrap,
	PageListItem,
} from "../../../state/backend-state/page-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Slider,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui";

const MAX_LIVE_VARIANTS = 2;
const VARIANT_NAME_PATTERN = /^[a-z0-9-]+$/;
/** Debounce between a slider release and the PATCH, so a nudged thumb sends one request. */
const WEIGHT_PATCH_DEBOUNCE_MS = 500;

const clampShare = (value: number): number =>
	Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;

function liveWeight(variant: IEventVariant): number | null {
	return "Live" in variant.mode ? clampShare(variant.mode.Live.weight) : null;
}

function shadowSampleRate(variant: IEventVariant): number | null {
	return "Shadow" in variant.mode
		? clampShare(variant.mode.Shadow.sample_rate)
		: null;
}

/**
 * Mirrors the core `variant_set()` read model: the stored variants when
 * non-empty, else the legacy `canary` field as a single Live variant.
 */
function readVariantSet(event: IEvent): {
	variants: IEventVariant[];
	legacy: boolean;
} {
	if (event.variants?.length)
		return { variants: event.variants, legacy: false };
	const canary = event.canary;
	if (!canary) return { variants: [], legacy: false };
	return {
		legacy: true,
		variants: [
			{
				name: "canary",
				board_id: canary.board_id,
				board_version: canary.board_version ?? null,
				node_id: canary.node_id,
				variables: canary.variables ?? {},
				default_page_id: null,
				mode: { Live: { weight: clampShare(canary.weight) } },
				created_at: canary.created_at,
				updated_at: canary.updated_at,
			},
		],
	};
}

const nowSystemTime = (): ISystemTime => ({
	secs_since_epoch: Math.floor(Date.now() / 1000),
	nanos_since_epoch: 0,
});

const formatShare = (share: number): string =>
	`${Math.round(clampShare(share) * 1000) / 10}%`;

const sameBoardVersion = (
	a: readonly number[] | null | undefined,
	b: readonly number[] | null | undefined,
): boolean =>
	(normalizeBoardVersion(a)?.join(".") ?? null) ===
	(normalizeBoardVersion(b)?.join(".") ?? null);

const messageOf = (error: unknown): string =>
	error instanceof Error ? error.message : String(error);

// Never invoked — `enabled` requires the real method; this only satisfies
// useInvoke's non-optional function parameter (same pattern as EventHistory).
async function statsUnavailable(
	_appId: string,
	_eventId: string,
	_window?: IEventVariantStatsWindow,
): Promise<IEventVariantStatsResult> {
	throw new Error("Canary stats are not supported on this platform");
}

async function setupsUnavailable(
	_appId: string,
	_eventId: string,
): Promise<IEventSetupInfo[]> {
	throw new Error("Per-variant setup health is not supported on this platform");
}

async function bootstrapUnavailable(
	_appId: string,
	_route?: string,
	_eventId?: string,
): Promise<IPageBootstrap> {
	throw new Error("Page bootstrap is not supported on this platform");
}

/** The `EventSetup` row name for the primary target. */
const STABLE_SETUP_NAME = "stable";

const formatRatePct = (rate: number): string => `${(rate * 100).toFixed(1)}%`;

const formatRateDelta = (delta: number): string => {
	const points = (delta * 100).toFixed(1);
	return delta >= 0 ? `+${points}` : points;
};

export function EventCanary({
	appId,
	event,
	onReload,
}: Readonly<{
	appId: string;
	event: IEvent;
	onReload?: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	// Canary is a cloud feature: the desktop provider does not implement these,
	// which is exactly how the History section detects its platform too.
	const variantsSupported =
		typeof backend.eventState.putEventVariants === "function";
	const patchSupported = typeof backend.eventState.patchCanary === "function";
	const statsSupported =
		typeof backend.eventState.getCanaryStats === "function";
	const explainSupported =
		typeof backend.eventState.explainCanary === "function";
	const promoteSupported =
		typeof backend.eventState.promoteCanary === "function";
	const abortSupported = typeof backend.eventState.abortCanary === "function";
	const setupsSupported =
		typeof backend.eventState.listEventSetups === "function";
	const setupRunSupported = typeof backend.eventState.setupEvent === "function";

	const isPageEvent = !!event.default_page_id;
	const isInbound = event.event_type === "rest" || event.event_type === "mcp";
	const { variants, legacy } = useMemo(() => readVariantSet(event), [event]);
	const liveCount = useMemo(
		() => variants.filter((variant) => liveWeight(variant) !== null).length,
		[variants],
	);

	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId],
		Boolean(appId && variantsSupported),
	);
	const boardsMap = useMemo(() => {
		const map = new Map<string, string>();
		for (const summary of boards.data ?? []) map.set(summary.id, summary.name);
		return map;
	}, [boards.data]);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		Boolean(appId && variantsSupported && isPageEvent),
	);
	const pagesMap = useMemo(() => {
		const map = new Map<string, PageListItem>();
		for (const page of pages.data ?? []) map.set(page.pageId, page);
		return map;
	}, [pages.data]);

	const [statsWindow, setStatsWindow] =
		useState<IEventVariantStatsWindow>("24h");
	const stats = useInvoke<
		IEventVariantStatsResult,
		[string, string, IEventVariantStatsWindow]
	>(
		backend.eventState.getCanaryStats ?? statsUnavailable,
		backend.eventState,
		[appId, event.id, statsWindow],
		Boolean(appId && event.id && statsSupported && variants.length > 0),
	);

	const setups = useInvoke<IEventSetupInfo[], [string, string]>(
		backend.eventState.listEventSetups ?? setupsUnavailable,
		backend.eventState,
		[appId, event.id],
		Boolean(appId && event.id && setupsSupported && isInbound && !isPageEvent),
	);

	// Page canaries are assigned at bootstrap by caller subject and pinned by
	// the sealed page claims, so the viewer's own bootstrap is the only honest
	// "which variant serves me" probe — the dispatch explain endpoint answers
	// "primary" for every page event.
	const bootstrapSupported =
		typeof backend.pageState.getPageBootstrap === "function";
	const session = useInvoke<
		IPageBootstrap,
		[string, string | undefined, string]
	>(
		backend.pageState.getPageBootstrap ?? bootstrapUnavailable,
		backend.pageState,
		[appId, undefined, event.id],
		Boolean(
			appId &&
				event.id &&
				isPageEvent &&
				variantsSupported &&
				bootstrapSupported &&
				variants.length > 0,
		),
	);

	const [busy, setBusy] = useState(false);
	const [editorTarget, setEditorTarget] = useState<
		IEventVariant | "new" | null
	>(null);
	const [promoteTarget, setPromoteTarget] = useState<IEventVariant | null>(
		null,
	);
	const [abortTarget, setAbortTarget] = useState<IEventVariant | null>(null);
	/** The setup-card row (`stable` or a variant name) whose setup is running. */
	const [setupBusyFor, setSetupBusyFor] = useState<string | null>(null);

	// Weight drafts keep the slider responsive while the PATCH is debounced.
	const [draftWeights, setDraftWeights] = useState<Record<string, number>>({});
	const patchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const pendingWeights = useRef<Map<string, number>>(new Map());
	useEffect(
		() => () => {
			if (patchTimer.current) clearTimeout(patchTimer.current);
		},
		[],
	);

	const refresh = useCallback(async () => {
		await invalidate(backend.eventState.getEvents, [appId]);
		await invalidate(backend.eventState.getEvent, [appId, event.id]);
		if (backend.pageState.getPageBootstrap) {
			await invalidate(backend.pageState.getPageBootstrap, [
				appId,
				undefined,
				event.id,
			]);
		}
		onReload?.();
	}, [
		appId,
		event.id,
		backend.eventState,
		backend.pageState,
		invalidate,
		onReload,
	]);

	// Promote/abort move the event's primary target: the timeline gains a
	// version, the stats regroup and the per-variant setup rows change.
	const refreshAll = useCallback(async () => {
		if (backend.eventState.getEventTimeline) {
			await invalidate(backend.eventState.getEventTimeline, [appId, event.id]);
		}
		if (backend.eventState.getCanaryStats) {
			await invalidate(backend.eventState.getCanaryStats, [appId, event.id]);
		}
		if (backend.eventState.listEventSetups) {
			await invalidate(backend.eventState.listEventSetups, [appId, event.id]);
		}
		await refresh();
	}, [appId, event.id, backend.eventState, invalidate, refresh]);

	const handlePromote = useCallback(
		async (variant: IEventVariant) => {
			const promoteCanary = backend.eventState.promoteCanary;
			if (!promoteCanary) return;
			setBusy(true);
			try {
				const result = await promoteCanary.call(
					backend.eventState,
					appId,
					event.id,
					variant.name,
				);
				setPromoteTarget(null);
				toast.success(
					t(
						"canaryPromoted",
						'Variant "{{name}}" promoted — now v{{version}}',
						{
							name: variant.name,
							version: result.event.event_version?.join(".") ?? "",
						},
					),
				);
				// Forward-only semantics: the promote holds even when the stable
				// endpoint re-setup failed — surface that as a warning, not an error.
				if (result.setup_status && result.setup_status !== "ok") {
					toast.warning(
						t(
							"canaryPromoteSetupPending",
							"Promoted, but the endpoint re-setup reported: {{status}}. Inbound calls keep the previous registration until a setup succeeds.",
							{ status: result.setup_status },
						),
					);
				}
				await refreshAll();
			} catch (error) {
				toast.error(
					t("canaryPromoteFailed", "Could not promote the variant: {{val}}", {
						val: messageOf(error),
					}),
				);
			} finally {
				setBusy(false);
			}
		},
		[appId, event.id, backend.eventState, refreshAll, t],
	);

	const handleAbort = useCallback(
		async (variant: IEventVariant) => {
			const abortCanary = backend.eventState.abortCanary;
			if (!abortCanary) return;
			setBusy(true);
			try {
				await abortCanary.call(
					backend.eventState,
					appId,
					event.id,
					variant.name,
				);
				setAbortTarget(null);
				toast.success(
					t(
						"canaryAborted",
						'Variant "{{name}}" aborted — traffic is back on the primary',
						{ name: variant.name },
					),
				);
				await refreshAll();
			} catch (error) {
				toast.error(
					t("canaryAbortFailed", "Could not abort the variant: {{val}}", {
						val: messageOf(error),
					}),
				);
			} finally {
				setBusy(false);
			}
		},
		[appId, event.id, backend.eventState, refreshAll, t],
	);

	const handleRunSetup = useCallback(
		async (variantName: string | null) => {
			const setupEvent = backend.eventState.setupEvent;
			if (!setupEvent) return;
			const label = variantName ?? STABLE_SETUP_NAME;
			setSetupBusyFor(label);
			try {
				const response = await setupEvent.call(
					backend.eventState,
					appId,
					event.id,
					true,
					variantName ?? undefined,
				);
				if (response.status === "ok") {
					toast.success(
						t(
							"canarySetupFinished",
							'Setup "{{name}}" finished ({{registrations}} registrations)',
							{ name: label, registrations: response.registrations_written },
						),
					);
				} else {
					toast.warning(
						t(
							"canarySetupReported",
							'Setup "{{name}}" reported {{status}}: {{val}}',
							{
								name: label,
								status: response.status,
								val: response.error ?? "",
							},
						),
					);
				}
			} catch (error) {
				toast.error(
					t("canarySetupFailed", "Setup failed: {{val}}", {
						val: messageOf(error),
					}),
				);
			} finally {
				setSetupBusyFor(null);
				if (backend.eventState.listEventSetups) {
					await invalidate(backend.eventState.listEventSetups, [
						appId,
						event.id,
					]);
				}
				await invalidate(backend.eventState.getEvent, [appId, event.id]);
			}
		},
		[appId, event.id, backend.eventState, invalidate, t],
	);

	const flushWeightPatches = useCallback(async () => {
		const patchCanary = backend.eventState.patchCanary;
		if (!patchCanary) return;
		const entries = [...pendingWeights.current.entries()];
		pendingWeights.current.clear();
		if (entries.length === 0) return;
		try {
			for (const [name, weight] of entries) {
				await patchCanary.call(backend.eventState, appId, event.id, {
					name,
					weight,
				});
			}
			await refresh();
		} catch (error) {
			toast.error(
				t(
					"canaryWeightPatchFailed",
					"Could not update the traffic share: {{val}}",
					{ val: messageOf(error) },
				),
			);
		} finally {
			setDraftWeights({});
		}
	}, [appId, event.id, backend.eventState, refresh, t]);

	const commitWeight = useCallback(
		(name: string, weight: number) => {
			pendingWeights.current.set(name, weight);
			if (patchTimer.current) clearTimeout(patchTimer.current);
			patchTimer.current = setTimeout(() => {
				void flushWeightPatches();
			}, WEIGHT_PATCH_DEBOUNCE_MS);
		},
		[flushWeightPatches],
	);

	const putVariants = useCallback(
		async (next: IEventVariant[], successMessage: string) => {
			const putEventVariants = backend.eventState.putEventVariants;
			if (!putEventVariants) return;
			setBusy(true);
			try {
				await putEventVariants.call(backend.eventState, appId, event.id, next);
				await refresh();
				toast.success(successMessage);
			} catch (error) {
				toast.error(
					t(
						"canaryVariantSaveFailed",
						"Could not save the variant list: {{val}}",
						{ val: messageOf(error) },
					),
				);
			} finally {
				setBusy(false);
			}
		},
		[appId, event.id, backend.eventState, refresh, t],
	);

	const handleEditorSave = useCallback(
		async (variant: IEventVariant) => {
			const exists = variants.some((entry) => entry.name === variant.name);
			const next = exists
				? variants.map((entry) =>
						entry.name === variant.name ? variant : entry,
					)
				: [...variants, variant];
			await putVariants(
				next,
				t("canaryVariantSaved", 'Variant "{{name}}" saved', {
					name: variant.name,
				}),
			);
			setEditorTarget(null);
		},
		[variants, putVariants, t],
	);

	const handleRemove = useCallback(
		async (name: string) => {
			await putVariants(
				variants.filter((entry) => entry.name !== name),
				t("canaryVariantRemoved", 'Variant "{{name}}" removed', { name }),
			);
		},
		[variants, putVariants, t],
	);

	if (!variantsSupported) {
		return (
			<Card>
				<CardContent className="flex flex-col items-center gap-2 py-10 text-center">
					<CloudIcon className="size-5 text-muted-foreground/60" />
					<p className="text-sm font-medium">
						{t("canaryCloudOnly", "Canary releases are cloud-only")}
					</p>
					<p className="max-w-[52ch] text-xs text-muted-foreground">
						{t(
							"canaryCloudOnlyDetail",
							"Local runs have no traffic to split — they always execute the primary target. Open this event in a cloud-hosted app to configure a canary.",
						)}
					</p>
				</CardContent>
			</Card>
		);
	}

	return (
		<div className="space-y-6">
			{isInbound && setupsSupported && (
				<InboundSetupCard
					variants={variants}
					setupRows={setups.data}
					loading={setups.isLoading}
					error={setups.isError ? (setups.error?.message ?? "") : null}
					setupBusyFor={setupBusyFor}
					canRunSetup={setupRunSupported && !busy}
					onRunSetup={(variantName) => void handleRunSetup(variantName)}
				/>
			)}

			<Card>
				<CardHeader>
					<div className="flex items-center justify-between gap-2">
						<CardTitle className="flex items-center gap-2">
							<SplitIcon className="h-5 w-5" />
							{t("canaryVariants", "Variants")}
						</CardTitle>
						<Button
							size="sm"
							className="gap-2"
							disabled={busy || liveCount >= MAX_LIVE_VARIANTS}
							onClick={() => setEditorTarget("new")}
						>
							<PlusIcon className="h-4 w-4" />
							{t("addVariant", "Add variant")}
						</Button>
					</div>
					<CardDescription>
						{isPageEvent
							? t(
									"canaryPageVariantsDescription",
									"Each live variant replaces the primary page for its share of viewers. Weight changes apply to new sessions immediately and never cut an event version.",
								)
							: t(
									"canaryVariantsDescription",
									"Each live variant replaces the primary target for its share of triggers. Weight changes apply immediately and never cut an event version.",
								)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-3">
					{isPageEvent && (
						<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
							<LayoutIcon className="mt-0.5 h-3 w-3 shrink-0" />
							{t(
								"canaryPageBootstrapNote",
								"Page canaries resolve once, when a page bootstraps: each viewer is assigned by account and stays on that variant until they reload. Shadow mode is not available for pages.",
							)}
						</p>
					)}
					{isPageEvent && bootstrapSupported && variants.length > 0 && (
						<PageSessionServing
							loading={session.isLoading}
							error={session.isError ? (session.error?.message ?? "") : null}
							servedVariant={session.data?.servedVariant}
						/>
					)}
					{liveCount >= MAX_LIVE_VARIANTS && (
						<p className="text-xs text-muted-foreground">
							{t(
								"canaryLiveVariantCapReached",
								"At most {{max}} live variants per event — remove one to add another.",
								{ max: MAX_LIVE_VARIANTS },
							)}
						</p>
					)}
					{legacy && (
						<p className="text-xs text-muted-foreground">
							{t(
								"canaryLegacyNotice",
								"This canary predates named variants; the next change migrates it to the variant list.",
							)}
						</p>
					)}
					{variants.length === 0 ? (
						<div className="flex flex-col items-center gap-1 py-8 text-center">
							<SplitIcon className="size-5 text-muted-foreground/60" />
							<p className="text-sm font-medium">
								{t("noCanaryYet", "All traffic goes to the primary target")}
							</p>
							<p className="max-w-[52ch] text-xs text-muted-foreground">
								{t(
									"noCanaryYetDetail",
									"Add a variant to send a share of this event's triggers to another flow, version or node before promoting it.",
								)}
							</p>
						</div>
					) : (
						<div className="flex flex-col gap-2">
							{variants.map((variant) => (
								<VariantRow
									key={variant.name}
									variant={variant}
									boardsMap={boardsMap}
									pagesMap={pagesMap}
									pageEvent={isPageEvent}
									draftWeight={draftWeights[variant.name]}
									disabled={busy}
									sliderEnabled={patchSupported}
									onWeightDraft={(weight) =>
										setDraftWeights((previous) => ({
											...previous,
											[variant.name]: weight,
										}))
									}
									onWeightCommit={(weight) =>
										commitWeight(variant.name, weight)
									}
									onEdit={() => setEditorTarget(variant)}
									onPromote={
										promoteSupported
											? () => setPromoteTarget(variant)
											: undefined
									}
									removeLabel={
										abortSupported
											? t("abortVariant", "Abort variant")
											: t("removeVariant", "Remove variant")
									}
									onRemove={() =>
										abortSupported
											? setAbortTarget(variant)
											: void handleRemove(variant.name)
									}
								/>
							))}
						</div>
					)}
				</CardContent>
			</Card>

			{statsSupported && variants.length > 0 && (
				<Card>
					<CardHeader>
						<div className="flex flex-wrap items-center justify-between gap-2">
							<CardTitle className="flex items-center gap-2">
								<ActivityIcon className="h-5 w-5" />
								{t("canaryStats", "Traffic")}
							</CardTitle>
							<div className="flex items-center gap-2">
								{explainSupported && !isPageEvent && (
									<ExplainAssignmentPopover appId={appId} eventId={event.id} />
								)}
								<Select
									value={statsWindow}
									onValueChange={(value) =>
										setStatsWindow(value as IEventVariantStatsWindow)
									}
								>
									<SelectTrigger size="sm" className="w-28 text-xs">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="24h">
											{t("last24Hours", "Last 24 h")}
										</SelectItem>
										<SelectItem value="7d">
											{t("last7Days", "Last 7 days")}
										</SelectItem>
									</SelectContent>
								</Select>
							</div>
						</div>
						<CardDescription>
							{t(
								"canaryStatsDescription",
								"How each target has been running — watch errors and latency before raising the weight or promoting.",
							)}
						</CardDescription>
					</CardHeader>
					<CardContent>
						{stats.isLoading && (
							<div className="flex flex-col items-center justify-center gap-2 py-8">
								<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
								<p className="text-sm text-muted-foreground">
									{t("loadingCanaryStats", "Loading traffic stats…")}
								</p>
							</div>
						)}
						{stats.isError && (
							<div className="flex gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
								<AlertTriangleIcon className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
								<p>
									{t(
										"failedToLoadCanaryStats",
										"Could not load the per-variant stats: {{val}}",
										{ val: stats.error?.message ?? "" },
									)}
								</p>
							</div>
						)}
						{stats.data && (
							<div className="overflow-x-auto rounded-md border">
								<table className="w-full min-w-120 text-sm">
									<thead>
										<tr className="border-b bg-muted/40 text-left text-xs text-muted-foreground">
											<th className="px-3 py-2 font-medium">
												{t("variant", "Variant")}
											</th>
											<th className="px-3 py-2 text-right font-medium">
												{t("requests", "Requests")}
											</th>
											<th className="px-3 py-2 text-right font-medium">
												{t("errors", "Errors")}
											</th>
											<th className="px-3 py-2 text-right font-medium">p50</th>
											<th className="px-3 py-2 text-right font-medium">p95</th>
										</tr>
									</thead>
									<tbody>
										{stats.data.variants.map((row) => (
											<tr
												key={row.variant_name ?? "__primary__"}
												className="border-b last:border-b-0"
											>
												<td className="px-3 py-2">
													<span className="font-mono text-xs">
														{row.variant_name ?? t("primaryTarget", "Primary")}
													</span>
												</td>
												<td className="px-3 py-2 text-right tabular-nums">
													{row.requests}
												</td>
												<td
													className={
														row.errors > 0
															? "px-3 py-2 text-right tabular-nums text-destructive"
															: "px-3 py-2 text-right tabular-nums"
													}
												>
													{row.errors}
												</td>
												<td className="px-3 py-2 text-right tabular-nums">
													{formatDuration(row.p50_duration_us)}
												</td>
												<td className="px-3 py-2 text-right tabular-nums">
													{formatDuration(row.p95_duration_us)}
												</td>
											</tr>
										))}
									</tbody>
								</table>
							</div>
						)}
					</CardContent>
				</Card>
			)}

			{editorTarget && (
				<VariantEditorDialog
					appId={appId}
					event={event}
					existing={editorTarget === "new" ? null : editorTarget}
					takenNames={variants.map((entry) => entry.name)}
					busy={busy}
					onSave={handleEditorSave}
					onClose={() => setEditorTarget(null)}
				/>
			)}

			{promoteTarget && (
				<PromoteDialog
					variant={promoteTarget}
					boardsMap={boardsMap}
					pagesMap={pagesMap}
					stats={stats.data}
					busy={busy}
					onConfirm={() => void handlePromote(promoteTarget)}
					onClose={() => setPromoteTarget(null)}
				/>
			)}

			{abortTarget && (
				<AbortVariantDialog
					name={abortTarget.name}
					busy={busy}
					onConfirm={() => void handleAbort(abortTarget)}
					onClose={() => setAbortTarget(null)}
				/>
			)}
		</div>
	);
}

function VariantRow({
	variant,
	boardsMap,
	pagesMap,
	pageEvent,
	draftWeight,
	disabled,
	sliderEnabled,
	onWeightDraft,
	onWeightCommit,
	onEdit,
	onPromote,
	removeLabel,
	onRemove,
}: Readonly<{
	variant: IEventVariant;
	boardsMap: Map<string, string>;
	pagesMap: Map<string, PageListItem>;
	pageEvent: boolean;
	draftWeight?: number;
	disabled: boolean;
	sliderEnabled: boolean;
	onWeightDraft: (weight: number) => void;
	onWeightCommit: (weight: number) => void;
	onEdit: () => void;
	onPromote?: () => void;
	removeLabel: string;
	onRemove: () => void;
}>) {
	const { t } = useTranslation("settings");
	const storedWeight = liveWeight(variant);
	const sampleRate = shadowSampleRate(variant);
	const weight = draftWeight ?? storedWeight;
	const pinned = normalizeBoardVersion(variant.board_version);
	const overrideCount = Object.keys(variant.variables ?? {}).length;
	const boardName =
		boardsMap.get(variant.board_id) ?? t("boardNotFound", "BOARD NOT FOUND!");
	// A page-less Live variant on a page event is not a page target: the
	// bootstrap resolver hands anyone hashed onto it the primary instead.
	const pageMissing = pageEvent && !variant.default_page_id;

	return (
		<div className="rounded-md border p-3">
			<div className="flex flex-wrap items-center gap-2">
				<span className="font-mono text-sm font-semibold">{variant.name}</span>
				{storedWeight !== null ? (
					<Badge className="h-5 px-1.5 text-[10px]">
						{t("liveShare", "Live · {{share}}", {
							share: formatShare(weight ?? 0),
						})}
					</Badge>
				) : (
					<Badge variant="outline" className="h-5 px-1.5 text-[10px]">
						{t("shadowShare", "Shadow · {{share}}", {
							share: formatShare(sampleRate ?? 0),
						})}
					</Badge>
				)}
				<Badge
					variant="secondary"
					className="h-5 px-1.5 font-normal text-[10px]"
				>
					{pinned
						? t("pinnedVersionBadge", "pinned v{{version}}", {
								version: pinned.join("."),
							})
						: t("floatingLatestBadge", "floating latest")}
				</Badge>
				{overrideCount > 0 && (
					<Badge
						variant="outline"
						className="h-5 px-1.5 font-normal text-[10px]"
					>
						{t("variantOverrideCount", "{{count}} variable overrides", {
							count: overrideCount,
						})}
					</Badge>
				)}
				<span className="ml-auto flex items-center gap-1">
					{onPromote && (
						<Button
							variant="outline"
							size="sm"
							className="h-7 gap-1.5 px-2 text-xs"
							disabled={disabled}
							onClick={onPromote}
						>
							<RocketIcon className="h-3.5 w-3.5" />
							{t("promoteVariant", "Promote")}
						</Button>
					)}
					<Button
						variant="ghost"
						size="icon"
						className="size-7 text-muted-foreground hover:text-foreground"
						aria-label={t("editVariant", "Edit variant")}
						disabled={disabled}
						onClick={onEdit}
					>
						<PencilIcon className="h-3.5 w-3.5" />
					</Button>
					<Button
						variant="ghost"
						size="icon"
						className="size-7 text-destructive hover:text-destructive"
						aria-label={removeLabel}
						disabled={disabled}
						onClick={onRemove}
					>
						<Trash2Icon className="h-3.5 w-3.5" />
					</Button>
				</span>
			</div>
			{pageEvent ? (
				<p
					className={
						pageMissing
							? "mt-1 flex items-center gap-1.5 truncate text-xs text-destructive"
							: "mt-1 flex items-center gap-1.5 truncate text-xs text-muted-foreground"
					}
				>
					<LayoutIcon className="h-3 w-3 shrink-0" />
					<span className="truncate">
						{variant.default_page_id
							? (pagesMap.get(variant.default_page_id)?.name ??
								t("pageNotFound", "Page not found"))
							: t(
									"variantNoPageTarget",
									"No page — viewers hashed onto this variant stay on the primary",
								)}
						{" · "}
						{boardName}
					</span>
				</p>
			) : (
				<p className="mt-1 truncate text-xs text-muted-foreground">
					{boardName}
					{" · "}
					<span className="font-mono">{variant.node_id}</span>
				</p>
			)}
			{storedWeight !== null && (
				<div className="mt-3 flex items-center gap-3">
					<Slider
						className="max-w-72"
						value={[Math.round((weight ?? 0) * 100)]}
						min={0}
						max={100}
						step={1}
						disabled={disabled || !sliderEnabled}
						onValueChange={(value) => onWeightDraft((value[0] ?? 0) / 100)}
						onValueCommit={(value) => onWeightCommit((value[0] ?? 0) / 100)}
					/>
					<span className="w-12 text-right font-mono text-xs tabular-nums">
						{formatShare(weight ?? 0)}
					</span>
				</div>
			)}
			{storedWeight !== null && (weight ?? 0) === 0 && (
				<p className="mt-2 flex items-center gap-1.5 text-xs text-muted-foreground">
					<PinIcon className="h-3 w-3 shrink-0" />
					{t(
						"pinOnlyVariantHint",
						"Pin-only: weight 0 serves no weighted traffic, but callers can still force this variant with the x-flow-like-variant header or ?__variant=.",
					)}
				</p>
			)}
		</div>
	);
}

/**
 * The settings viewer's own bootstrap assignment for a page event. The server
 * reports it only once it resolves variants at bootstrap; older servers omit
 * the field and this says nothing rather than guessing.
 */
function PageSessionServing({
	loading,
	error,
	servedVariant,
}: Readonly<{
	loading: boolean;
	error: string | null;
	servedVariant: string | null | undefined;
}>) {
	const { t } = useTranslation("settings");

	if (loading) {
		return (
			<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
				<Loader2Icon className="h-3 w-3 animate-spin" />
				{t(
					"canaryPageSessionResolving",
					"Resolving which variant serves your account…",
				)}
			</p>
		);
	}
	if (error !== null) {
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"canaryPageSessionFailed",
					"Could not resolve which variant serves your account: {{val}}",
					{ val: error },
				)}
			</p>
		);
	}
	if (servedVariant === undefined) return null;
	return (
		<p className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
			<EyeIcon className="h-3 w-3 shrink-0" />
			{t("canaryPageSessionServedBy", "Your account bootstraps into:")}
			<Badge variant="outline" className="h-5 px-1.5 font-mono text-[10px]">
				{servedVariant ?? t("primaryTarget", "Primary")}
			</Badge>
		</p>
	);
}

/**
 * Per-variant REST/MCP setup health: one row per inbound registration bucket
 * (`stable` plus every live variant). A variant without a successful setup has
 * no inbound surface — routed calls keep hitting the stable registration.
 */
function InboundSetupCard({
	variants,
	setupRows,
	loading,
	error,
	setupBusyFor,
	canRunSetup,
	onRunSetup,
}: Readonly<{
	variants: IEventVariant[];
	setupRows?: IEventSetupInfo[];
	loading: boolean;
	error: string | null;
	setupBusyFor: string | null;
	canRunSetup: boolean;
	onRunSetup: (variant: string | null) => void;
}>) {
	const { t } = useTranslation("settings");

	// Shadow variants have no inbound surface (setup refuses them), so rows are
	// the stable target, live variants, and any leftover rows still on record.
	const rows = useMemo(() => {
		const byVariant = new Map<string, IEventSetupInfo>();
		for (const row of setupRows ?? []) byVariant.set(row.variant, row);
		const names = [
			STABLE_SETUP_NAME,
			...variants
				.filter((variant) => liveWeight(variant) !== null)
				.map((variant) => variant.name),
		];
		for (const row of setupRows ?? []) {
			if (!names.includes(row.variant)) names.push(row.variant);
		}
		return names.map((name) => ({ name, info: byVariant.get(name) }));
	}, [setupRows, variants]);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<RadioTowerIcon className="h-5 w-5" />
					{t("canaryInboundSetup", "Inbound setup")}
				</CardTitle>
				<CardDescription>
					{t(
						"canaryInboundSetupDescription",
						"REST and MCP calls are served per variant from its own registration bucket. A variant without a successful setup receives no inbound traffic — routed calls keep hitting the stable registration.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading && (
					<div className="flex flex-col items-center justify-center gap-2 py-6">
						<Loader2Icon className="h-5 w-5 animate-spin text-muted-foreground" />
						<p className="text-sm text-muted-foreground">
							{t("canaryLoadingSetups", "Loading setup health…")}
						</p>
					</div>
				)}
				{error !== null && (
					<div className="flex gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
						<AlertTriangleIcon className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
						<p>
							{t(
								"canaryFailedToLoadSetups",
								"Could not load the per-variant setup health: {{val}}",
								{ val: error },
							)}
						</p>
					</div>
				)}
				{!loading && error === null && (
					<TooltipProvider>
						<div className="flex flex-col gap-2">
							{rows.map(({ name, info }) => (
								<div
									key={name}
									className="flex flex-wrap items-center gap-2 rounded-md border p-2.5"
								>
									<span className="font-mono text-sm font-semibold">
										{name}
									</span>
									{name === STABLE_SETUP_NAME && (
										<Badge
											variant="secondary"
											className="h-5 px-1.5 font-normal text-[10px]"
										>
											{t("canaryStablePrimaryBadge", "primary")}
										</Badge>
									)}
									<SetupStatusChip info={info} />
									{info && (
										<span className="text-xs text-muted-foreground">
											{t("canaryServesVersion", "serves v{{version}}", {
												version: info.event_version,
											})}
										</span>
									)}
									<span className="ml-auto">
										<Button
											variant="outline"
											size="sm"
											className="h-7 gap-1.5 px-2 text-xs"
											disabled={!canRunSetup || setupBusyFor !== null}
											onClick={() =>
												onRunSetup(name === STABLE_SETUP_NAME ? null : name)
											}
										>
											{setupBusyFor === name ? (
												<Loader2Icon className="h-3.5 w-3.5 animate-spin" />
											) : (
												<WrenchIcon className="h-3.5 w-3.5" />
											)}
											{t("canaryRunSetup", "Run setup")}
										</Button>
									</span>
								</div>
							))}
						</div>
					</TooltipProvider>
				)}
			</CardContent>
		</Card>
	);
}

function SetupStatusChip({ info }: Readonly<{ info?: IEventSetupInfo }>) {
	const { t } = useTranslation("settings");
	const status = info?.setup_status ?? null;

	if (status === "ok") {
		return (
			<Badge className="h-5 px-1.5 text-[10px]">
				{t("canarySetupStatusOk", "ok")}
			</Badge>
		);
	}
	if (status === "running") {
		return (
			<Badge variant="outline" className="h-5 gap-1 px-1.5 text-[10px]">
				<Loader2Icon className="h-2.5 w-2.5 animate-spin" />
				{t("canarySetupStatusRunning", "running")}
			</Badge>
		);
	}
	if (status !== null) {
		const chip = (
			<Badge variant="destructive" className="h-5 px-1.5 text-[10px]">
				{status === "error" ? t("canarySetupStatusError", "error") : status}
			</Badge>
		);
		if (!info?.last_setup_error) return chip;
		return (
			<Tooltip>
				<TooltipTrigger asChild>{chip}</TooltipTrigger>
				<TooltipContent className="max-w-80 break-words">
					{info.last_setup_error}
				</TooltipContent>
			</Tooltip>
		);
	}
	return (
		<Badge
			variant="outline"
			className="h-5 px-1.5 font-normal text-[10px] text-muted-foreground"
		>
			{t("canarySetupStatusNone", "not set up")}
		</Badge>
	);
}

function PromoteDialog({
	variant,
	boardsMap,
	pagesMap,
	stats,
	busy,
	onConfirm,
	onClose,
}: Readonly<{
	variant: IEventVariant;
	boardsMap: Map<string, string>;
	pagesMap: Map<string, PageListItem>;
	stats?: IEventVariantStatsResult;
	busy: boolean;
	onConfirm: () => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("settings");
	const pinned = normalizeBoardVersion(variant.board_version);
	const isShadow = shadowSampleRate(variant) !== null;

	const rates = useMemo(() => {
		const entries = stats?.variants;
		if (!entries) return null;
		const variantRow = entries.find((row) => row.variant_name === variant.name);
		const primaryRow = entries.find((row) => row.variant_name === null);
		if (!variantRow || variantRow.requests === 0) {
			return { missing: true as const };
		}
		const variantRate = variantRow.errors / variantRow.requests;
		const primaryRate =
			primaryRow && primaryRow.requests > 0
				? primaryRow.errors / primaryRow.requests
				: null;
		return { missing: false as const, variantRate, primaryRate };
	}, [stats, variant.name]);

	const worseThanPrimary =
		rates !== null &&
		!rates.missing &&
		rates.primaryRate !== null &&
		rates.variantRate > rates.primaryRate;

	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<RocketIcon className="h-4 w-4" />
						{t("promoteVariantTitle", 'Promote "{{name}}"', {
							name: variant.name,
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"promoteVariantDescription",
							"Its target becomes this event's primary for all traffic, the variant is removed, and a new event version is cut.",
						)}
					</DialogDescription>
				</DialogHeader>
				<DialogBody className="space-y-3">
					<div className="rounded-md border bg-muted/40 p-3 text-sm">
						<dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
							<dt className="text-muted-foreground">{t("flow", "Flow")}</dt>
							<dd className="truncate">
								{boardsMap.get(variant.board_id) ?? variant.board_id}
							</dd>
							<dt className="text-muted-foreground">
								{t("flowVersion", "Flow Version")}
							</dt>
							<dd>{pinned ? `v${pinned.join(".")}` : t("latest", "Latest")}</dd>
							{variant.default_page_id ? (
								<>
									<dt className="text-muted-foreground">{t("page", "Page")}</dt>
									<dd className="truncate">
										{pagesMap.get(variant.default_page_id)?.name ??
											variant.default_page_id}
									</dd>
								</>
							) : (
								<>
									<dt className="text-muted-foreground">{t("node", "Node")}</dt>
									<dd className="truncate font-mono text-xs leading-5">
										{variant.node_id}
									</dd>
								</>
							)}
						</dl>
					</div>
					{isShadow && (
						<p className="text-xs text-muted-foreground">
							{t(
								"promoteShadowHint",
								"This is a shadow variant — promoting makes its mirrored target the live primary.",
							)}
						</p>
					)}
					{rates &&
						(rates.missing ? (
							<p className="text-xs text-muted-foreground">
								{t(
									"canaryNoTrafficInWindow",
									"No traffic recorded for this variant in the stats window.",
								)}
							</p>
						) : (
							<p
								className={
									worseThanPrimary
										? "text-xs text-destructive"
										: "text-xs text-muted-foreground"
								}
							>
								{rates.primaryRate === null
									? t(
											"canaryErrorRateNoPrimary",
											"Error rate {{variant}} — no primary traffic in this window to compare.",
											{ variant: formatRatePct(rates.variantRate) },
										)
									: t(
											"canaryErrorRateDelta",
											"Error rate {{variant}} vs {{primary}} on the primary ({{delta}} pts).",
											{
												variant: formatRatePct(rates.variantRate),
												primary: formatRatePct(rates.primaryRate),
												delta: formatRateDelta(
													rates.variantRate - rates.primaryRate,
												),
											},
										)}
							</p>
						))}
				</DialogBody>
				<DialogFooter>
					<Button variant="outline" onClick={onClose} disabled={busy}>
						{t("cancel", "Cancel")}
					</Button>
					<Button className="gap-2" onClick={onConfirm} disabled={busy}>
						{busy ? (
							<Loader2Icon className="h-4 w-4 animate-spin" />
						) : (
							<RocketIcon className="h-4 w-4" />
						)}
						{t("promoteVariant", "Promote")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function AbortVariantDialog({
	name,
	busy,
	onConfirm,
	onClose,
}: Readonly<{
	name: string;
	busy: boolean;
	onConfirm: () => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<AlertDialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("abortVariantTitle", 'Abort "{{name}}"?', { name })}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"abortVariantDescription",
							"The variant is removed and its traffic returns to the primary immediately. Its inbound registrations are dropped.",
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel disabled={busy}>
						{t("cancel", "Cancel")}
					</AlertDialogCancel>
					<AlertDialogAction disabled={busy} onClick={onConfirm}>
						{t("abortVariant", "Abort variant")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function VariantEditorDialog({
	appId,
	event,
	existing,
	takenNames,
	busy,
	onSave,
	onClose,
}: Readonly<{
	appId: string;
	event: IEvent;
	existing: IEventVariant | null;
	takenNames: string[];
	busy: boolean;
	onSave: (variant: IEventVariant) => Promise<void>;
	onClose: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const isPageEvent = !!event.default_page_id;

	const [name, setName] = useState(
		existing?.name ?? (takenNames.includes("canary") ? "" : "canary"),
	);
	const [boardId, setBoardId] = useState(existing?.board_id ?? event.board_id);
	const [version, setVersion] = useState<BoardVersion | undefined>(
		normalizeBoardVersion(existing?.board_version),
	);
	const [nodeId, setNodeId] = useState(existing?.node_id ?? "");
	// A page variant defaults to the primary's page, so the common case — canary
	// a new version of the same page — is one version pick away.
	const [pageId, setPageId] = useState(
		existing?.default_page_id ?? event.default_page_id ?? "",
	);
	const [weightPct, setWeightPct] = useState(() => {
		const stored = existing ? liveWeight(existing) : null;
		return Math.round((stored ?? 0.1) * 100);
	});

	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId],
		!!appId,
	);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		Boolean(appId && isPageEvent),
	);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, boardId],
		Boolean(appId && boardId),
	);
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, boardId, version],
		Boolean(appId && boardId && !isPageEvent),
	);
	const startNodes = useMemo(
		() => Object.values(board.data?.nodes ?? {}).filter((node) => node.start),
		[board.data?.nodes],
	);
	const boardName = useMemo(
		() =>
			boards.data?.find((summary) => summary.id === boardId)?.name ?? boardId,
		[boards.data, boardId],
	);

	// The page decides the board, exactly like the event form's page picker.
	const handleSelectPage = useCallback(
		(value: string) => {
			setPageId(value);
			const page = pages.data?.find((entry) => entry.pageId === value);
			setBoardId(page?.boardId ?? "");
			setVersion(undefined);
		},
		[pages.data],
	);

	const nameError = useMemo(() => {
		if (!VARIANT_NAME_PATTERN.test(name)) {
			return t(
				"variantNameInvalid",
				"Only lowercase letters, digits and '-' are allowed.",
			);
		}
		if (name !== existing?.name && takenNames.includes(name)) {
			return t(
				"variantNameTaken",
				"This event already has a variant named that.",
			);
		}
		return null;
	}, [name, existing?.name, takenNames, t]);

	// Mirrors the upsert refusal: a variant equal to the primary target is a
	// no-op. Page variants inherit the primary's node, so their triple is
	// board, version and page.
	const primaryError = useMemo(
		() =>
			boardId === event.board_id &&
			sameBoardVersion(version, event.board_version) &&
			(isPageEvent
				? pageId === (event.default_page_id ?? "")
				: nodeId === event.node_id)
				? t(
						"variantEqualsPrimary",
						"This is exactly the primary target — a variant must differ somewhere.",
					)
				: null,
		[
			isPageEvent,
			boardId,
			version,
			nodeId,
			pageId,
			event.board_id,
			event.board_version,
			event.node_id,
			event.default_page_id,
			t,
		],
	);

	const canSave =
		!busy &&
		!nameError &&
		!primaryError &&
		!!boardId &&
		(isPageEvent ? !!pageId : !!nodeId);

	const handleSave = useCallback(async () => {
		// Page events take Live variants only: shadowing a page would double
		// every state-mutating page action, so core refuses it.
		const mode =
			!isPageEvent && existing && !("Live" in existing.mode)
				? existing.mode
				: { Live: { weight: weightPct / 100 } };
		await onSave({
			name,
			board_id: boardId,
			board_version: version ?? null,
			node_id: isPageEvent ? event.node_id : nodeId,
			variables: existing?.variables ?? {},
			default_page_id: isPageEvent
				? pageId
				: (existing?.default_page_id ?? null),
			mode,
			created_at: existing?.created_at ?? nowSystemTime(),
			updated_at: nowSystemTime(),
		});
	}, [
		isPageEvent,
		existing,
		name,
		boardId,
		version,
		nodeId,
		pageId,
		weightPct,
		event.node_id,
		onSave,
	]);

	const showWeight = isPageEvent || !existing || "Live" in existing.mode;

	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
		>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<SplitIcon className="h-4 w-4" />
						{existing
							? t("editVariantTitle", 'Edit variant "{{name}}"', {
									name: existing.name,
								})
							: t("addVariantTitle", "Add variant")}
					</DialogTitle>
					<DialogDescription>
						{isPageEvent
							? t(
									"pageVariantEditorDescription",
									"The variant replaces the primary page for its share of viewers. Variables stay inherited from the event unless overridden.",
								)
							: t(
									"variantEditorDescription",
									"The variant replaces the primary target for its share of triggers. Variables stay inherited from the event unless overridden.",
								)}
					</DialogDescription>
				</DialogHeader>
				<DialogBody className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="variant-name">
							{t("variantName", "Variant name")}
						</Label>
						<Input
							id="variant-name"
							value={name}
							disabled={!!existing}
							placeholder="canary"
							className="font-mono"
							onChange={(e) => setName(e.target.value)}
						/>
						{nameError && (
							<p className="text-xs text-destructive">{nameError}</p>
						)}
					</div>
					{isPageEvent ? (
						<div className="space-y-2">
							<Label>{t("page", "Page")}</Label>
							<Select value={pageId} onValueChange={handleSelectPage}>
								<SelectTrigger>
									<SelectValue
										placeholder={t("selectAPage", "Select a page")}
									/>
								</SelectTrigger>
								<SelectContent>
									{pages.data?.map((page: PageListItem) => (
										<SelectItem key={page.pageId} value={page.pageId}>
											{page.name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							{boardId && (
								<p className="text-xs text-muted-foreground">
									{t(
										"pageVariantFlowHint",
										"The page decides the flow: {{name}}. Pick its version below.",
										{ name: boardName },
									)}
								</p>
							)}
						</div>
					) : (
						<div className="space-y-2">
							<Label>{t("flow", "Flow")}</Label>
							<Select
								value={boardId}
								onValueChange={(value) => {
									setBoardId(value);
									setVersion(undefined);
									setNodeId("");
								}}
							>
								<SelectTrigger>
									<SelectValue
										placeholder={t("selectABoard", "Select a board")}
									/>
								</SelectTrigger>
								<SelectContent>
									{boards.data?.map((summary: IBoardSummary) => (
										<SelectItem key={summary.id} value={summary.id}>
											{summary.name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
					)}
					<div className="space-y-2">
						<Label>{t("flowVersion", "Flow Version")}</Label>
						<Select
							value={version?.join(".") ?? "latest"}
							onValueChange={(value) => {
								setVersion(
									value === "latest"
										? undefined
										: normalizeBoardVersion(value.split(".").map(Number)),
								);
								setNodeId("");
							}}
						>
							<SelectTrigger>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="latest">{t("latest", "Latest")}</SelectItem>
								{versions.data?.map((entry) => (
									<SelectItem key={entry.join(".")} value={entry.join(".")}>
										v{entry.join(".")}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<p className="text-xs text-muted-foreground">
							{t(
								"variantVersionHint",
								"Latest floats with new versions of the flow; a pinned version stays put until you promote.",
							)}
						</p>
					</div>
					{!isPageEvent && (
						<div className="space-y-2">
							<Label>{t("node", "Node")}</Label>
							<Select value={nodeId} onValueChange={setNodeId}>
								<SelectTrigger>
									<SelectValue
										placeholder={t("selectANode", "Select a node")}
									/>
								</SelectTrigger>
								<SelectContent>
									{startNodes.map((node) => (
										<SelectItem key={node.id} value={node.id}>
											{node.friendly_name || node.name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
					)}
					{showWeight && (
						<div className="space-y-2">
							<Label>
								{t("initialTrafficShare", "Traffic share ({{share}})", {
									share: `${weightPct}%`,
								})}
							</Label>
							<Slider
								value={[weightPct]}
								min={0}
								max={100}
								step={1}
								onValueChange={(value) => setWeightPct(value[0] ?? 0)}
							/>
							<p className="text-xs text-muted-foreground">
								{t(
									"pinOnlyWeightZeroHint",
									"Weight 0 is pin-only: nothing is routed by weight, but the variant can be forced with an explicit pin.",
								)}
							</p>
						</div>
					)}
					<p className="text-xs text-muted-foreground">
						{isPageEvent
							? t(
									"pageVariantBootstrapHint",
									"Resolved once when the page bootstraps, by the viewer's account — an open session keeps its variant until reload. Shadow mode is not available for pages.",
								)
							: t(
									"variantSinkIdentityHint",
									"Variants run under the primary event's sink identity and credentials — an event has exactly one sink.",
								)}
					</p>
					{primaryError && (
						<p className="text-xs text-destructive">{primaryError}</p>
					)}
				</DialogBody>
				<DialogFooter>
					<Button variant="outline" onClick={onClose} disabled={busy}>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={() => void handleSave()} disabled={!canSave}>
						{busy && <Loader2Icon className="h-4 w-4 animate-spin" />}
						{existing ? t("save", "Save") : t("addVariant", "Add variant")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function ExplainAssignmentPopover({
	appId,
	eventId,
}: Readonly<{ appId: string; eventId: string }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [key, setKey] = useState("");
	const [source, setSource] = useState("subject");
	const [result, setResult] = useState<ICanaryExplainResult | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [isExplaining, setIsExplaining] = useState(false);

	const handleExplain = useCallback(async () => {
		const explainCanary = backend.eventState.explainCanary;
		if (!explainCanary || !key.trim()) return;
		setIsExplaining(true);
		setError(null);
		try {
			const assignment = await explainCanary.call(
				backend.eventState,
				appId,
				eventId,
				key.trim(),
				source,
			);
			setResult(assignment);
		} catch (cause) {
			setResult(null);
			setError(messageOf(cause));
		} finally {
			setIsExplaining(false);
		}
	}, [appId, eventId, backend.eventState, key, source]);

	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button variant="outline" size="sm" className="gap-2">
					<SplitIcon className="h-3.5 w-3.5" />
					{t("explainAssignment", "Explain assignment")}
				</Button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-80 space-y-3">
				<div>
					<p className="text-sm font-medium">
						{t("explainAssignmentTitle", "Which variant serves a key?")}
					</p>
					<p className="mt-0.5 text-xs text-muted-foreground">
						{t(
							"explainAssignmentDescription",
							"Assignments are a pure hash of the event and the split key, so any past or hypothetical key can be recomputed here.",
						)}
					</p>
				</div>
				<div className="space-y-2">
					<Input
						value={key}
						placeholder={t(
							"explainKeyPlaceholder",
							"Idempotency key, user id, run id…",
						)}
						className="font-mono text-xs"
						onChange={(e) => setKey(e.target.value)}
					/>
					<Select value={source} onValueChange={setSource}>
						<SelectTrigger size="sm" className="w-full text-xs">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="idempotency-key">
								{t("splitSourceIdempotencyKey", "Idempotency key")}
							</SelectItem>
							<SelectItem value="trace">
								{t("splitSourceTrace", "Trace id")}
							</SelectItem>
							<SelectItem value="subject">
								{t("splitSourceSubject", "Caller subject")}
							</SelectItem>
							<SelectItem value="run-id">
								{t("splitSourceRunId", "Run id")}
							</SelectItem>
							<SelectItem value="pin">
								{t("splitSourcePin", "Explicit pin (variant name)")}
							</SelectItem>
						</SelectContent>
					</Select>
					<Button
						size="sm"
						className="w-full"
						disabled={isExplaining || !key.trim()}
						onClick={() => void handleExplain()}
					>
						{isExplaining && (
							<Loader2Icon className="h-3.5 w-3.5 animate-spin" />
						)}
						{t("explain", "Explain")}
					</Button>
				</div>
				{error && <p className="text-xs text-destructive">{error}</p>}
				{result && (
					<div className="rounded-md border bg-muted/40 p-2.5 text-xs">
						<p>
							{t("explainServedBy", "Served by:")}{" "}
							<span className="font-mono font-semibold">
								{result.variant_name ?? t("primaryTarget", "Primary")}
							</span>
						</p>
						<p className="mt-1 text-muted-foreground">
							{t(
								"explainShareBounds",
								"Owns [{{lo}}, {{hi}}) of the unit interval.",
								{
									lo: formatShare(result.share_bounds[0]),
									hi: formatShare(result.share_bounds[1]),
								},
							)}
						</p>
					</div>
				)}
			</PopoverContent>
		</Popover>
	);
}
