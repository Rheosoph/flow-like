"use client";

import {
	type CollisionDetection,
	DndContext,
	type DragEndEvent,
	DragOverlay,
	type DragStartEvent,
	KeyboardSensor,
	PointerSensor,
	closestCenter,
	pointerWithin,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import { arrayMove, sortableKeyboardCoordinates } from "@dnd-kit/sortable";
import { LANGUAGES, useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	AlertTriangle,
	Code2,
	ExternalLink,
	Loader2,
	MousePointerClick,
	Send,
	X,
} from "lucide-react";
import Link from "next/link";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useState,
	useSyncExternalStore,
} from "react";
import { toast } from "sonner";
import { apiErrorMessage } from "../../../../lib/api-error";
import { isTauri } from "../../../../lib/platform";
import { cn } from "../../../../lib/utils";
import { EXPLORE_PATH } from "../../../store/explore/explore-href";
import { isExploreUnsupportedError } from "../../../store/explore/explore-model";
import {
	EXPLORE_LIMITS,
	type ExploreEditorState,
	type ExplorePlacementInput,
	type ExploreSlotKey,
	type ExploreViewer,
} from "../../../store/explore/explore-types";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../../ui/alert-dialog";
import { Button } from "../../../ui/button";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
} from "../../../ui/sheet";
import { Skeleton } from "../../../ui/skeleton";
import type { AddHandler } from "./add-menu";
import {
	type AddTarget,
	type AdminT,
	addedPlacementId,
	changeLabel,
	collectionPlacements,
	defaultPlacementName,
	duplicateInput,
	errorMessage,
	findPlacement,
	issueMessage,
	layoutRows,
	newPlacementTemplate,
	newRowKey,
	orderAfterMove,
	orderWithNewRow,
	orderWithPriority,
	orderWithRows,
	orderWithoutRow,
	placementInput,
	refsByKey,
	slotAccepts,
	slotLabel,
	traceMarkers,
	validatePlacement,
} from "./explore-admin-model";
import { ExploreCanvas, NEW_ROW_DROP } from "./explore-canvas";
import { useItemNames } from "./inspector-items";
import {
	NewPlacementDialog,
	type PendingPlacement,
} from "./new-placement-dialog";
import { PlacementInspector } from "./placement-inspector";
import { PlacementLibrary } from "./placement-library";
import {
	type AdminExploreApi,
	type AdminExploreMediaUpload,
	browserMediaUpload,
	isDraftConflict,
	useAdminExplore,
	useAdminExplorePreview,
	useArtworkUpload,
} from "./use-admin-explore";
import { type DraftFailure, usePlacementDraft } from "./use-placement-draft";

const LARGE_QUERY = "(min-width: 1024px)";

function useMediaQuery(query: string): boolean {
	return useSyncExternalStore(
		(notify) => {
			const media = window.matchMedia(query);
			media.addEventListener("change", notify);
			return () => media.removeEventListener("change", notify);
		},
		() => window.matchMedia(query).matches,
		() => false,
	);
}

function previewLanguage(language: string | undefined): string {
	const code = (language ?? "en").toLowerCase();
	return (
		LANGUAGES.find((entry) => entry.toLowerCase() === code) ??
		LANGUAGES.find((entry) => entry.toLowerCase() === code.split("-")[0]) ??
		"en"
	);
}

function accountError(api: AdminExploreApi, t: AdminT): Error | null {
	if (!api.info.data && api.info.isError) return api.info.error;
	if (api.profile.data) return null;
	if (api.profile.isError) return api.profile.error;
	return api.profile.isSuccess
		? new Error(
				t(
					"exploreProfileUnavailable",
					"Your admin profile is not available. Reload and try again.",
				),
			)
		: null;
}

export function AdminExplorePage({
	uploadMedia = browserMediaUpload,
}: {
	uploadMedia?: AdminExploreMediaUpload;
}) {
	const { t } = useTranslation("admin");
	const api = useAdminExplore();
	if (!api.ready || api.profile.isLoading || api.info.isLoading) {
		return <EditorSkeleton />;
	}
	const account = accountError(api, t);
	if (account) {
		return (
			<EditorError
				error={account}
				onRetry={() => {
					void api.profile.refetch();
					void api.info.refetch();
				}}
			/>
		);
	}
	if (!api.allowed) return <PermissionGate />;
	if (api.state.isError && !api.state.data) {
		return (
			<EditorError
				error={api.state.error}
				onRetry={() => api.state.refetch()}
			/>
		);
	}
	if (!api.state.data) return <EditorSkeleton />;
	return (
		<AdminExploreEditor
			api={api}
			state={api.state.data}
			uploadMedia={uploadMedia}
		/>
	);
}

function CenteredMessage({
	title,
	description,
	action,
}: {
	title: string;
	description: string;
	action?: ReactNode;
}) {
	const { t } = useTranslation("admin");
	return (
		<main className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
			<AlertCircle className="size-7 text-muted-foreground" />
			<h1 className="font-semibold">{title}</h1>
			<p className="max-w-md text-sm text-muted-foreground">{description}</p>
			<div className="flex gap-2">
				{action}
				<Button variant="outline" asChild>
					<Link href="/admin">{t("exploreBackToAdmin", "Back to admin")}</Link>
				</Button>
			</div>
		</main>
	);
}

function PermissionGate() {
	const { t } = useTranslation("admin");
	return (
		<CenteredMessage
			title={t("explorePermissionTitle", "Landing page permission required")}
			description={t(
				"explorePermissionDescription",
				"An administrator with permission to edit landing pages can curate the Explore page here.",
			)}
		/>
	);
}

function EditorError({
	error,
	onRetry,
}: {
	error: Error;
	onRetry: () => void;
}) {
	const { t } = useTranslation("admin");
	if (isExploreUnsupportedError(error)) {
		return (
			<CenteredMessage
				title={t(
					"exploreUnsupportedTitle",
					"This hub has no Explore editor yet",
				)}
				description={t(
					"exploreUnsupportedDescription",
					"Update the hub to curate the Explore page. Until then, Explore shows its classic lists.",
				)}
			/>
		);
	}
	return (
		<CenteredMessage
			title={t("exploreLoadFailed", "The Explore draft could not be loaded")}
			description={errorMessage(error, t("exploreTryAgain", "Try again."))}
			action={
				<Button onClick={onRetry}>{t("exploreRetry", "Try again")}</Button>
			}
		/>
	);
}

function EditorSkeleton() {
	return (
		<main className="flex min-h-0 flex-1 flex-col">
			<div className="flex h-16 items-center gap-4 border-b px-6">
				<Skeleton className="h-8 w-48" />
				<Skeleton className="ml-auto h-8 w-72" />
			</div>
			<div className="grid flex-1 gap-4 p-4 lg:grid-cols-[280px_minmax(0,1fr)_340px]">
				<Skeleton className="h-96" />
				<Skeleton className="h-96" />
				<Skeleton className="hidden h-96 lg:block" />
			</div>
		</main>
	);
}

function PreviewToggle({
	label,
	prefix,
	value,
	options,
	onChange,
}: {
	label: string;
	prefix?: ReactNode;
	value: string;
	options: readonly { value: string; label: string }[];
	onChange: (value: string) => void;
}) {
	return (
		<fieldset
			aria-label={label}
			className="flex h-8 min-w-0 items-center gap-0.5 rounded-[7px] border bg-card p-0.5"
		>
			{prefix}
			{options.map((option) => {
				const pressed = option.value === value;
				return (
					<button
						key={option.value}
						type="button"
						aria-pressed={pressed}
						onClick={() => onChange(option.value)}
						className={cn(
							"h-6.5 rounded-[5px] px-2.5 text-xs whitespace-nowrap transition-colors focus-visible:outline-2 focus-visible:outline-ring",
							pressed
								? "bg-muted font-semibold text-foreground shadow-xs"
								: "font-medium text-muted-foreground hover:text-foreground",
						)}
					>
						{option.label}
					</button>
				);
			})}
		</fieldset>
	);
}

function EditorHeader({
	state,
	viewer,
	onViewer,
	publishing,
	publishBlocked,
	onDiscard,
	onPublish,
}: {
	state: ExploreEditorState;
	viewer: ExploreViewer;
	onViewer: (viewer: ExploreViewer) => void;
	publishing: boolean;
	publishBlocked?: string;
	onDiscard: () => void;
	onPublish: () => void;
}) {
	const { t } = useTranslation("admin");
	const changes = state.changes.length;
	const canPublish = changes > 0 || state.liveRevision === null;
	return (
		<header className="z-20 flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2.5 border-b bg-background/95 px-4 py-3 backdrop-blur md:sticky md:top-0 lg:min-h-16 lg:pr-5 lg:pl-6">
			<div className="flex min-w-0 flex-col">
				<nav
					aria-label={t("exploreBreadcrumb", "Breadcrumb")}
					className="text-xs leading-4 text-muted-foreground"
				>
					<Link href="/admin" className="hover:text-foreground">
						{t("exploreBreadcrumbAdmin", "Admin")}
					</Link>
					<span aria-hidden="true" className="mx-1.5">
						/
					</span>
					<span>{t("exploreBreadcrumbExplore", "Explore")}</span>
				</nav>
				<div className="flex min-w-0 flex-wrap items-center gap-2.5">
					<h1 className="text-[19px] leading-6.5 font-semibold tracking-[-0.015em] whitespace-nowrap">
						{t("exploreEditorTitle", "Explore layout")}
					</h1>
					<output
						aria-live="polite"
						className={cn(
							"inline-flex h-5.5 items-center gap-1.5 rounded-full px-2.25 text-[11.5px] font-medium whitespace-nowrap",
							changes
								? "bg-primary/12 text-primary"
								: "bg-green-500/12 text-green-700 dark:text-green-400",
						)}
					>
						<span
							aria-hidden="true"
							className={cn(
								"size-1.5 rounded-full",
								changes ? "bg-primary" : "bg-green-500",
							)}
						/>
						{changeLabel(state.changes, t)}
					</output>
				</div>
			</div>
			<div className="flex flex-wrap items-center gap-2 lg:ml-auto">
				<span className="text-xs whitespace-nowrap text-muted-foreground">
					{t("explorePreviewAs", "Preview as")}
				</span>
				<PreviewToggle
					label={t("explorePreviewDevMode", "Developer mode")}
					prefix={
						<span className="inline-flex items-center gap-1.5 pr-2 pl-1.5 text-xs whitespace-nowrap text-muted-foreground">
							<Code2 aria-hidden="true" className="size-3.5" />
							{t("explorePreviewDevMode", "Developer mode")}
						</span>
					}
					value={viewer.dev ? "on" : "off"}
					options={[
						{ value: "on", label: t("explorePreviewOn", "On") },
						{ value: "off", label: t("explorePreviewOff", "Off") },
					]}
					onChange={(value) => onViewer({ ...viewer, dev: value === "on" })}
				/>
				<PreviewToggle
					label={t("explorePreviewAuth", "Sign-in state")}
					value={viewer.signedIn ? "in" : "out"}
					options={[
						{ value: "in", label: t("exploreAudienceSignedIn", "Signed in") },
						{
							value: "out",
							label: t("exploreAudienceSignedOut", "Signed out"),
						},
					]}
					onChange={(value) =>
						onViewer({ ...viewer, signedIn: value === "in" })
					}
				/>
				<PreviewToggle
					label={t("explorePreviewPlatform", "Platform")}
					value={viewer.platform}
					options={[
						{ value: "desktop", label: t("exploreAudienceDesktop", "Desktop") },
						{ value: "web", label: t("exploreAudienceWeb", "Web") },
					]}
					onChange={(value) =>
						onViewer({
							...viewer,
							platform: value === "web" ? "web" : "desktop",
						})
					}
				/>
				<Select
					value={viewer.language}
					onValueChange={(language) => onViewer({ ...viewer, language })}
				>
					<SelectTrigger
						size="sm"
						className="h-8 w-20 text-xs"
						aria-label={t("explorePreviewLanguage", "Preview language")}
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{LANGUAGES.map((language) => (
							<SelectItem key={language} value={language}>
								{language}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<span
					aria-hidden="true"
					className="mx-1.5 hidden h-6 w-px bg-border sm:block"
				/>
				<Button variant="outline" size="icon" asChild>
					<Link
						href={EXPLORE_PATH}
						aria-label={t("exploreOpenLive", "Open the live Explore page")}
						title={t("exploreOpenLive", "Open the live Explore page")}
					>
						<ExternalLink className="size-4" />
					</Link>
				</Button>
				<Button
					variant="secondary"
					onClick={onDiscard}
					disabled={!changes || publishing}
				>
					{t("exploreDiscard", "Discard")}
				</Button>
				<Button
					onClick={onPublish}
					disabled={!canPublish || publishing || !!publishBlocked}
					title={publishBlocked}
				>
					{publishing ? (
						<Loader2 className="size-4 animate-spin" />
					) : (
						<Send className="size-4" />
					)}
					{t("explorePublish", "Publish changes")}
				</Button>
			</div>
		</header>
	);
}

function Banner({
	tone,
	title,
	children,
	actions,
}: {
	tone: "warning" | "info";
	title: string;
	children: ReactNode;
	actions: ReactNode;
}) {
	return (
		<div
			role={tone === "warning" ? "alert" : "status"}
			className={cn(
				"flex shrink-0 flex-wrap items-start gap-x-4 gap-y-2 border-b px-5 py-3 text-sm",
				tone === "warning"
					? "border-amber-500/30 bg-amber-500/10"
					: "border-blue-500/25 bg-blue-500/8",
			)}
		>
			<AlertTriangle
				aria-hidden="true"
				className={cn(
					"mt-0.5 size-4 shrink-0",
					tone === "warning"
						? "text-amber-600 dark:text-amber-400"
						: "text-blue-600 dark:text-blue-400",
				)}
			/>
			<div className="min-w-0 flex-1">
				<p className="font-medium">{title}</p>
				<div className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
					{children}
				</div>
			</div>
			<div className="flex shrink-0 flex-wrap gap-2">{actions}</div>
		</div>
	);
}

function AdminExploreEditor({
	api,
	state,
	uploadMedia,
}: {
	api: AdminExploreApi;
	state: ExploreEditorState;
	uploadMedia: AdminExploreMediaUpload;
}) {
	const { t, i18n } = useTranslation("admin");
	const [viewer, setViewer] = useState<ExploreViewer>(() => ({
		dev: true,
		signedIn: true,
		platform: isTauri() ? "desktop" : "web",
		language: previewLanguage(i18n.resolvedLanguage ?? i18n.language),
	}));
	const preview = useAdminExplorePreview(api, viewer);
	const layout = state.layout;
	const previewPage = preview.isError ? undefined : preview.data;
	const markers = useMemo(
		() => (previewPage ? traceMarkers(previewPage.trace, layout) : undefined),
		[previewPage, layout],
	);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const location = selectedId ? findPlacement(layout, selectedId) : undefined;
	const collectionIds = useMemo(
		() =>
			new Set(collectionPlacements(layout).map((placement) => placement.id)),
		[layout],
	);
	const savePlacement = useCallback(
		async (id: string, input: ExplorePlacementInput) => {
			const next = await api.update(id, input);
			const saved = findPlacement(next.layout, id);
			return saved ? placementInput(saved.placement) : undefined;
		},
		[api.update],
	);
	const onBackgroundFailure = useCallback(
		(name: string, failure: DraftFailure) => {
			const description =
				failure.reason === "error"
					? failure.message
					: failure.reason === "conflict"
						? api.conflictMessage()
						: t("exploreEditsLostInvalid", "Some fields were not valid.");
			toast.error(
				t("exploreEditsLost", "Your edits to “{{name}}” were not saved", {
					name,
				}),
				{ description },
			);
		},
		[api.conflictMessage, t],
	);
	const draft = usePlacementDraft({
		placement: location?.placement,
		collectionIds,
		save: savePlacement,
		fallbackError: t("exploreSaveFailed", "The placement could not be saved."),
		onBackgroundFailure,
	});
	const refs = useMemo(() => refsByKey(state.refs), [state.refs]);
	const [picked, setPicked] = useState<ReadonlyMap<string, string>>(
		() => new Map(),
	);
	const onPicked = useCallback((key: string, name: string) => {
		setPicked((current) =>
			current.get(key) === name ? current : new Map(current).set(key, name),
		);
	}, []);
	const nameOf = useItemNames(layout, refs, picked);
	const artwork = useArtworkUpload(api, uploadMedia, !!location);
	const large = useMediaQuery(LARGE_QUERY);
	const formatDate = useMemo(() => {
		const format = new Intl.DateTimeFormat(i18n.language, {
			dateStyle: "medium",
			timeStyle: "short",
		});
		return (iso: string) => {
			const date = new Date(iso);
			return Number.isNaN(date.getTime()) ? iso : format.format(date);
		};
	}, [i18n.language]);
	const [showGrid, setShowGrid] = useState(true);
	const [pending, setPending] = useState<PendingPlacement | null>(null);
	const [discardOpen, setDiscardOpen] = useState(false);
	const [publishing, setPublishing] = useState(false);
	const [warnings, setWarnings] = useState<string[]>([]);
	const [dragLabel, setDragLabel] = useState<string | null>(null);
	const [leaving, setLeaving] = useState<{ to: string | null } | null>(null);

	useEffect(() => {
		if (selectedId && !findPlacement(layout, selectedId)) setSelectedId(null);
	}, [layout, selectedId]);

	const select = (id: string | null) => {
		if (id === selectedId) return;
		if (location && draft.cannotSave) {
			setLeaving({ to: id });
			return;
		}
		setSelectedId(id);
	};

	const leaveWithoutSaving = () => {
		if (!leaving) return;
		if (draft.saveState === "conflict") api.clearConflict();
		draft.reset(undefined);
		setSelectedId(leaving.to);
		setLeaving(null);
	};

	const guard = api.saving || draft.unsaved;
	useEffect(() => {
		if (!guard) return;
		const prevent = (event: BeforeUnloadEvent) => {
			event.preventDefault();
			event.returnValue = "";
		};
		window.addEventListener("beforeunload", prevent);
		return () => window.removeEventListener("beforeunload", prevent);
	}, [guard]);

	const fail = (title: string) => (error: unknown) => {
		if (isDraftConflict(error)) return;
		const description = apiErrorMessage(error, "");
		toast.error(title, description ? { description } : undefined);
	};

	const issueFor = useCallback(
		(field: string) => {
			const issue = draft.issues.find(
				(entry) =>
					entry.field === field ||
					(field === "items" && /^items\.\d+$/.test(entry.field)),
			);
			return issue ? issueMessage(issue, t) : undefined;
		},
		[draft.issues, t],
	);

	const createAt = async (target: AddTarget, input: ExplorePlacementInput) => {
		const before = layout;
		if (target.newRow) {
			await api.order((current) =>
				orderWithNewRow(current, target.slot as `row:${string}`),
			);
		}
		const next = await api.create(target.slot, input);
		const id = addedPlacementId(before, next.layout, target.slot);
		if (id) select(id);
	};

	const onAdd: AddHandler = (target, choice) => {
		const input = newPlacementTemplate(
			choice.kind,
			target.slot,
			defaultPlacementName(choice, t),
			choice.rail,
		);
		if (validatePlacement(input).length) {
			setPending({ target, choice, input });
			return;
		}
		createAt(target, input).catch(
			fail(t("exploreCreateFailed", "The placement could not be added.")),
		);
	};

	const reorder = (
		build: Parameters<AdminExploreApi["order"]>[0],
		title = t("exploreMoveFailed", "The layout could not be changed."),
	) => api.order(build).then(() => undefined, fail(title));

	const duplicate = async () => {
		if (!location || draft.draft?.id !== location.placement.id) return;
		const source = { ...location.placement, ...draft.draft.input };
		const suffix = ` ${t("exploreCopySuffix", "(copy)")}`;
		try {
			const next = await api.create(
				location.slot.key,
				duplicateInput(source, suffix),
				location.index + 1,
			);
			const id = addedPlacementId(layout, next.layout, location.slot.key);
			if (id) select(id);
		} catch (error) {
			fail(
				t("exploreDuplicateFailed", "The placement could not be duplicated."),
			)(error);
		}
	};

	const move = async (target: ExploreSlotKey) => {
		if (!location) return;
		const id = location.placement.id;
		await reorder((current) => orderAfterMove(current, id, target));
	};

	const remove = async () => {
		if (!location) return;
		void draft.flush();
		await api.remove(location.placement.id);
		setSelectedId(null);
	};

	const publish = async () => {
		setPublishing(true);
		try {
			if (!(await draft.flush())) {
				toast.error(t("explorePublishFailed", "Publishing failed"), {
					description: t(
						"explorePublishBlocked",
						"Fix or drop the unsaved edits in the inspector before publishing.",
					),
				});
				return;
			}
			const next = await api.publish();
			setWarnings(next.warnings);
			if (next.warnings.length) {
				toast.warning(
					t("explorePublishedWarnings", "Published with {{count}} warnings", {
						count: next.warnings.length,
					}),
					{ description: next.warnings.slice(0, 3).join("\n") },
				);
			} else {
				toast.success(t("explorePublished", "The Explore page is live"));
			}
		} catch (error) {
			fail(t("explorePublishFailed", "Publishing failed"))(error);
		} finally {
			setPublishing(false);
		}
	};

	const discard = async () => {
		draft.reset(location?.placement);
		try {
			const next = await api.discard();
			const kept = selectedId
				? findPlacement(next.layout, selectedId)
				: undefined;
			draft.reset(kept?.placement);
			if (!kept) setSelectedId(null);
			setWarnings([]);
			toast.success(
				t("exploreDiscarded", "The draft matches the live page again"),
			);
		} catch (error) {
			fail(t("exploreDiscardFailed", "The draft could not be discarded"))(
				error,
			);
		} finally {
			setDiscardOpen(false);
		}
	};

	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
		useSensor(KeyboardSensor, {
			coordinateGetter: sortableKeyboardCoordinates,
		}),
	);
	const collision = useCallback<CollisionDetection>((args) => {
		const active = String(args.active.id);
		const scope = (prefixes: string[]) => ({
			...args,
			droppableContainers: args.droppableContainers.filter((container) =>
				prefixes.some((prefix) => String(container.id).startsWith(prefix)),
			),
		});
		if (active.startsWith("r:")) return closestCenter(scope(["r:"]));
		const within = pointerWithin(scope(["p:", "s:", "r:"]));
		if (within.length) return within;
		return closestCenter(
			scope(active.startsWith("p:") ? ["p:"] : ["s:", "r:"]),
		);
	}, []);

	const dragStart = ({ active }: DragStartEvent) => {
		const id = String(active.id);
		if (id.startsWith("r:")) {
			setDragLabel(slotLabel(layout, id.slice(2), t));
			return;
		}
		const placementId = (active.data.current as { placementId?: string })
			?.placementId;
		const found = placementId ? findPlacement(layout, placementId) : undefined;
		setDragLabel(found?.placement.name ?? null);
	};

	const dragEnd = ({ active, over }: DragEndEvent) => {
		setDragLabel(null);
		if (!over) return;
		const activeId = String(active.id);
		const overId = String(over.id);
		if (activeId === overId) return;
		if (activeId.startsWith("r:")) {
			if (!overId.startsWith("r:")) return;
			const keys = layoutRows(layout).map((row) => row.key);
			const from = keys.indexOf(activeId.slice(2) as ExploreSlotKey);
			const to = keys.indexOf(overId.slice(2) as ExploreSlotKey);
			if (from < 0 || to < 0) return;
			const rows = arrayMove(keys, from, to);
			void reorder((current) => orderWithRows(current, rows));
			return;
		}
		const placementId = (active.data.current as { placementId?: string })
			?.placementId;
		const found = placementId ? findPlacement(layout, placementId) : undefined;
		if (!placementId || !found) return;
		let target: ExploreSlotKey | undefined;
		let index: number | undefined;
		if (overId.startsWith("p:")) {
			const destination = findPlacement(layout, overId.slice(2));
			if (!destination) return;
			target = destination.slot.key;
			index = destination.index;
		} else if (overId === NEW_ROW_DROP) {
			if (layoutRows(layout).length >= EXPLORE_LIMITS.rows) return;
			target = newRowKey(layout);
		} else if (overId.startsWith("s:") || overId.startsWith("r:")) {
			target = overId.slice(2) as ExploreSlotKey;
		}
		if (!target) return;
		if (target === found.slot.key) {
			if (index === undefined || index === found.index) return;
			const ids = arrayMove(
				found.slot.placements.map((placement) => placement.id),
				found.index,
				index,
			);
			const slotKey = target;
			void reorder((current) => orderWithPriority(current, slotKey, ids));
			return;
		}
		if (!slotAccepts(target, found.placement.content)) {
			toast.error(
				t("exploreDropRejected", "{{slot}} does not take this placement", {
					slot: slotLabel(layout, target, t),
				}),
			);
			return;
		}
		const destination = target;
		void reorder((current) =>
			orderAfterMove(current, placementId, destination, index),
		);
	};

	const inspector =
		location && draft.draft?.id === location.placement.id ? (
			<PlacementInspector
				key={location.placement.id}
				layout={layout}
				location={location}
				input={draft.draft.input}
				issueFor={issueFor}
				saveState={draft.saveState}
				serverError={draft.serverError}
				refs={refs}
				nameOf={nameOf}
				markers={markers}
				preview={previewPage?.page}
				viewer={viewer}
				artwork={artwork}
				formatDate={formatDate}
				onEdit={draft.edit}
				onPicked={onPicked}
				onSelect={select}
				onDuplicate={duplicate}
				onMove={move}
				onDelete={remove}
			/>
		) : null;

	const publishBlocked = draft.cannotSave
		? t(
				"explorePublishBlocked",
				"Fix or drop the unsaved edits in the inspector before publishing.",
			)
		: undefined;

	const conflictName = location?.placement.name ?? "";
	const draftConflict = draft.saveState === "conflict";

	return (
		<main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto lg:overflow-hidden">
			<EditorHeader
				state={state}
				viewer={viewer}
				onViewer={setViewer}
				publishing={publishing}
				publishBlocked={publishBlocked}
				onDiscard={() => setDiscardOpen(true)}
				onPublish={() => void publish()}
			/>
			{(api.conflict || draftConflict) && (
				<Banner
					tone="warning"
					title={t("exploreConflictTitle", "The Explore draft changed")}
					actions={
						draftConflict ? (
							<>
								<Button
									size="sm"
									onClick={() => {
										api.clearConflict();
										draft.reapply();
									}}
								>
									{t("exploreConflictReapply", "Save my edits again")}
								</Button>
								<Button
									size="sm"
									variant="outline"
									onClick={() => {
										api.clearConflict();
										draft.reset(location?.placement);
									}}
								>
									{t("exploreConflictDrop", "Drop my edits")}
								</Button>
							</>
						) : (
							<Button size="sm" variant="outline" onClick={api.clearConflict}>
								{t("exploreDismiss", "Dismiss")}
							</Button>
						)
					}
				>
					{api.conflict ?? api.conflictMessage()}{" "}
					{draftConflict
						? t(
								"exploreConflictKept",
								"The editor now shows their version. Your unsaved edits to “{{name}}” are still in the inspector.",
								{ name: conflictName },
							)
						: t(
								"exploreConflictReloaded",
								"The editor now shows their version. Check it, then repeat your last step.",
							)}
				</Banner>
			)}
			{warnings.length > 0 && (
				<Banner
					tone="info"
					title={t("explorePublishWarningsTitle", "Published with warnings")}
					actions={
						<Button
							size="icon"
							variant="ghost"
							className="size-7"
							aria-label={t("exploreDismiss", "Dismiss")}
							onClick={() => setWarnings([])}
						>
							<X className="size-4" />
						</Button>
					}
				>
					<ul className="list-disc pl-4">
						{warnings.map((warning) => (
							<li key={warning}>{warning}</li>
						))}
					</ul>
				</Banner>
			)}
			{api.state.isError && (
				<Banner
					tone="warning"
					title={t(
						"exploreLoadFailed",
						"The Explore draft could not be loaded",
					)}
					actions={
						<Button
							size="sm"
							variant="outline"
							disabled={api.state.isFetching}
							onClick={() => void api.state.refetch()}
						>
							{t("exploreRetry", "Try again")}
						</Button>
					}
				>
					{errorMessage(api.state.error, t("exploreTryAgain", "Try again."))}
				</Banner>
			)}
			<DndContext
				sensors={sensors}
				collisionDetection={collision}
				onDragStart={dragStart}
				onDragCancel={() => setDragLabel(null)}
				onDragEnd={dragEnd}
			>
				<div className="grid grid-cols-1 md:grid-cols-[260px_minmax(0,1fr)] lg:min-h-0 lg:flex-1 lg:grid-cols-[280px_minmax(0,1fr)_340px]">
					<aside
						aria-label={t("explorePlacements", "Placements")}
						className="border-b md:border-r md:border-b-0 lg:min-h-0 lg:overflow-hidden"
					>
						<PlacementLibrary
							layout={layout}
							markers={markers}
							selectedId={selectedId}
							now={state.now}
							formatDate={formatDate}
							onSelect={select}
							onAdd={onAdd}
						/>
					</aside>
					<section
						aria-label={t("exploreCanvas", "Layout canvas")}
						className="min-w-0 lg:min-h-0 lg:overflow-hidden"
					>
						<ExploreCanvas
							layout={layout}
							markers={markers}
							preview={previewPage?.page}
							previewPending={preview.isFetching}
							previewFailed={preview.isError}
							onRetryPreview={() => void preview.refetch()}
							viewer={viewer}
							selectedId={selectedId}
							refs={refs}
							nameOf={nameOf}
							showGrid={showGrid}
							onToggleGrid={() => setShowGrid((value) => !value)}
							onSelect={select}
							onAdd={onAdd}
							onAddRow={() =>
								void reorder((current) => orderWithNewRow(current))
							}
							onRemoveRow={(key) =>
								void reorder((current) => orderWithoutRow(current, key))
							}
						/>
					</section>
					{large && (
						<aside
							aria-label={t("exploreInspector", "Inspector")}
							className="min-h-0 overflow-y-auto border-l bg-card/40"
						>
							{inspector ?? (
								<div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center text-sm text-muted-foreground">
									<MousePointerClick aria-hidden="true" className="size-5" />
									{t(
										"exploreInspectorEmpty",
										"Select a placement on the canvas or in the list to edit it.",
									)}
								</div>
							)}
						</aside>
					)}
				</div>
				<DragOverlay dropAnimation={null}>
					{dragLabel ? (
						<div className="rounded-md border bg-popover px-2.5 py-1.5 text-xs font-medium shadow-lg">
							{dragLabel}
						</div>
					) : null}
				</DragOverlay>
			</DndContext>
			{!large && (
				<Sheet
					open={!!inspector}
					onOpenChange={(open) => {
						if (!open) select(null);
					}}
				>
					<SheetContent
						side="right"
						className="w-full gap-0 overflow-y-auto p-0 sm:max-w-100"
					>
						<SheetHeader className="sr-only">
							<SheetTitle>{location?.placement.name}</SheetTitle>
							<SheetDescription>
								{t("exploreInspector", "Inspector")}
							</SheetDescription>
						</SheetHeader>
						<div className="pt-7">{inspector}</div>
					</SheetContent>
				</Sheet>
			)}
			{pending && (
				<NewPlacementDialog
					pending={pending}
					layout={layout}
					refs={refs}
					nameOf={nameOf}
					onPicked={onPicked}
					onCancel={() => setPending(null)}
					onCreate={async (target, input) => {
						await createAt(target, input);
						setPending(null);
					}}
				/>
			)}
			<AlertDialog open={discardOpen} onOpenChange={setDiscardOpen}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("exploreDiscardTitle", "Discard all unpublished changes?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"exploreDiscardDescription",
								"The draft goes back to the live layout for every admin. This cannot be undone.",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("exploreCancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-white hover:bg-destructive/90"
							onClick={(event) => {
								event.preventDefault();
								void discard();
							}}
						>
							{t("exploreDiscardConfirm", "Discard changes")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
			<AlertDialog
				open={!!leaving}
				onOpenChange={(open) => !open && setLeaving(null)}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("exploreLeaveTitle", "Leave “{{name}}” without saving?", {
								name:
									draft.draft?.input.name.trim() ||
									location?.placement.name ||
									"",
							})}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"exploreLeaveDescription",
								"Your edits to it are not saved yet. If you switch now, they are dropped.",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("exploreKeepEditing", "Keep editing")}
						</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-white hover:bg-destructive/90"
							onClick={leaveWithoutSaving}
						>
							{t("exploreConflictDrop", "Drop my edits")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</main>
	);
}
