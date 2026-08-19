"use client";
import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { isEqual } from "lodash-es";
import {
	ArrowLeft,
	BookOpen,
	CheckCircle2,
	Columns2,
	FileText,
	LayoutTemplate,
	type LucideIcon,
	Play,
	Sparkles,
	Workflow,
	Wrench,
	Zap,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	ImperativePanelGroupHandle,
	ImperativePanelHandle,
} from "react-resizable-panels";
import { toast } from "sonner";
import type {
	IApp,
	IEvent,
	IMetadata,
	IOAuthProvider,
	IStoredOAuthToken,
} from "../../lib";
import { EVENT_CONFIG } from "../../lib/event-config";
import { cn } from "../../lib/utils";
import { useInvoke } from "../../hooks/use-invoke";
import type { useHub } from "../../hooks/use-hub";
import { useBackend } from "../../state/backend-state";
import { useFlowBoardParentState } from "../../state/flow-board-parent-state";
import { IExecutionStage, ILogLevel } from "../../lib/schema/flow/board";
import {
	type FlowLibraryBoardCreationState,
	FlowLibraryBoardsSection,
	FlowLibraryHeader,
} from "../flow/flow-library";
import { FlowWrapper } from "../flow/flow-wrapper";
import { UsePageContent } from "../interfaces/use-page-content";
import { AppGeneralSettings } from "../settings/app-general/app-general-settings";
import EventsPage from "../settings/events/events-page";
import { type PageData, PagesSection } from "../settings/routes/pages-section";
import { Button } from "../ui/button";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
} from "../ui/resizable";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";

export type PaneMode = "use" | "flow" | "flows" | "events" | "pages" | "config";

export interface PaneTarget {
	readonly mode: PaneMode;
	readonly appId: string;
	readonly boardId?: string;
	readonly nodeId?: string;
	readonly version?: [number, number, number];
	readonly routePath?: string;
	readonly eventId?: string | null;
	readonly newEventTemplate?: Partial<IEvent>;
	readonly label?: string;
}

export type LessonMode = "read" | "split" | "build";

export const LESSON_MODE_LAYOUTS: Record<LessonMode, [number, number]> = {
	read: [100, 0],
	split: [50, 50],
	build: [30, 70],
};

export function routeLabelForLessonSubpath(subpath: string) {
	if (subpath === "config") return "Config";
	if (subpath === "events") return "Events";
	if (subpath === "pages") return "Pages";
	if (subpath === "flow") return "Board";
	if (subpath === "use") return "App";
	return "App";
}

export function paneModeForSubpath(subpath: string): PaneMode {
	if (subpath === "events") return "events";
	if (subpath === "pages") return "pages";
	if (subpath === "flow") return "flows";
	if (subpath === "use") return "use";
	return "config";
}

export function useIsWideScreen() {
	const [isWide, setIsWide] = useState(false);
	useEffect(() => {
		if (typeof window === "undefined") return;
		const mql = window.matchMedia("(min-width: 1024px)");
		const update = () => setIsWide(mql.matches);
		update();
		mql.addEventListener("change", update);
		return () => mql.removeEventListener("change", update);
	}, []);
	return isWide;
}

interface OAuthBridge {
	readonly tokenStore: unknown;
	readonly consentStore: unknown;
	readonly onStartOAuth: (provider: IOAuthProvider) => Promise<void>;
	readonly onRefreshToken: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
}

export interface LessonWorkspaceProps {
	readonly courseHref: string;
	readonly onBack?: () => void;
	/** Zero-based position of this lesson within the whole course. */
	readonly lessonIndex: number;
	readonly lessonCount: number;
	readonly estimatedMinutes?: number;
	readonly lessonComplete: boolean;
	readonly completePending: boolean;
	readonly canComplete: boolean;
	readonly onMarkComplete: () => void;
	readonly showSplitView: boolean;
	readonly mode: LessonMode;
	readonly onModeChange: (mode: LessonMode) => void;
	readonly panelGroupRef: React.RefObject<ImperativePanelGroupHandle | null>;
	readonly appPanelRef: React.RefObject<ImperativePanelHandle | null>;
	readonly children: ReactNode;
	readonly paneTarget: PaneTarget | null;
	readonly onPaneTargetChange: (target: PaneTarget) => void;
	readonly authSub?: string;
	readonly hub: ReturnType<typeof useHub>["hub"];
	readonly uiEventTypes: string[];
	readonly oauth: OAuthBridge;
	readonly BackLink?: (props: {
		readonly href: string;
		readonly className?: string;
		readonly children: ReactNode;
	}) => ReactNode;
}

/**
 * The lesson chrome shared by every shell: reading column, workspace pane, and
 * the Read / Split / Build split between them. Both apps used to own a copy of
 * this and drifted; data wiring stays in the page, the layout lives here.
 */
export function LessonWorkspace({
	courseHref,
	onBack,
	lessonIndex,
	lessonCount,
	estimatedMinutes,
	lessonComplete,
	completePending,
	canComplete,
	onMarkComplete,
	showSplitView,
	mode,
	onModeChange,
	panelGroupRef,
	appPanelRef,
	children,
	paneTarget,
	onPaneTargetChange,
	authSub,
	hub,
	uiEventTypes,
	oauth,
	BackLink,
}: LessonWorkspaceProps) {
	const { t } = useTranslation();

	const back = BackLink ? (
		<BackLink
			href={courseHref}
			className="inline-flex shrink-0 items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
		>
			<ArrowLeft className="h-3.5 w-3.5" />
			{t("backToCourse", "Back to course")}
		</BackLink>
	) : (
		<button
			type="button"
			onClick={onBack}
			className="inline-flex shrink-0 items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
		>
			<ArrowLeft className="h-3.5 w-3.5" />
			{t("backToCourse", "Back to course")}
		</button>
	);

	return (
		<div className="flex h-full flex-1 flex-col overflow-hidden">
			<header className="flex items-center gap-4 border-b px-4 py-2.5 backdrop-blur supports-backdrop-filter:bg-background/70">
				{back}

				{lessonIndex >= 0 && lessonCount > 1 && (
					<div className="hidden min-w-0 items-center gap-3 md:flex">
						<span className="h-3 w-px shrink-0 bg-border" />
						<span
							className="flex items-center gap-1"
							aria-label={`Lesson ${lessonIndex + 1} of ${lessonCount}`}
						>
							{Array.from({ length: Math.min(lessonCount, 14) }).map((_, i) => (
								<span
									key={`step-${i + 1}`}
									className={cn(
										"block h-1 w-4 rounded-full",
										i === lessonIndex
											? "bg-primary"
											: i < lessonIndex
												? "bg-emerald-500/70 dark:bg-emerald-400/70"
												: "bg-border",
									)}
								/>
							))}
						</span>
						<span className="shrink-0 font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
							{t("valOfVal", "{{current}} / {{total}}", {
								current: lessonIndex + 1,
								total: lessonCount,
							})}
						</span>
						{estimatedMinutes ? (
							<span className="shrink-0 font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
								{t("valMin", "{{val}} min", { val: estimatedMinutes })}
							</span>
						) : null}
					</div>
				)}

				<div className="ml-auto flex shrink-0 items-center gap-3">
					{showSplitView && (
						<LessonModeToggle mode={mode} onChange={onModeChange} />
					)}
					<Button
						variant={lessonComplete ? "secondary" : "default"}
						size="sm"
						disabled={completePending || !canComplete || lessonComplete}
						onClick={onMarkComplete}
						className="gap-1.5"
					>
						<CheckCircle2 className="h-3.5 w-3.5" />
						{lessonComplete
							? t("completed", "Completed")
							: t("markComplete", "Mark complete")}
					</Button>
				</div>
			</header>

			{showSplitView ? (
				<ResizablePanelGroup
					ref={panelGroupRef}
					direction="horizontal"
					autoSaveId="lesson-workspace-layout"
					className="min-h-0 flex-1"
				>
					<ResizablePanel defaultSize={50} minSize={28} className="min-h-0">
						<section className="h-full overflow-auto">{children}</section>
					</ResizablePanel>
					<ResizableHandle withHandle className="bg-border/60" />
					<ResizablePanel
						ref={appPanelRef}
						defaultSize={50}
						minSize={25}
						collapsible
						collapsedSize={0}
						onCollapse={() => onModeChange("read")}
						onExpand={() => onModeChange(mode === "read" ? "split" : mode)}
						className="min-h-0"
					>
						<AppPane
							target={paneTarget}
							onTargetChange={onPaneTargetChange}
							authSub={authSub}
							hub={hub}
							uiEventTypes={uiEventTypes}
							oauth={oauth}
						/>
					</ResizablePanel>
				</ResizablePanelGroup>
			) : (
				<section className="flex-1 overflow-auto">{children}</section>
			)}
		</div>
	);
}

const LESSON_MODES: ReadonlyArray<{
	readonly id: LessonMode;
	readonly label: string;
	readonly description: string;
	readonly Icon: LucideIcon;
}> = [
	{
		id: "read",
		label: "Read",
		description: "Focus on the lesson — hide the workspace.",
		Icon: BookOpen,
	},
	{
		id: "split",
		label: "Split",
		description: "Read alongside the workspace, side by side.",
		Icon: Columns2,
	},
	{
		id: "build",
		label: "Build",
		description: "Give the workspace the spotlight.",
		Icon: Wrench,
	},
];

export function LessonModeToggle({
	mode,
	onChange,
}: {
	readonly mode: LessonMode;
	readonly onChange: (mode: LessonMode) => void;
}) {
	const { t } = useTranslation();
	return (
		<div className="inline-flex items-center gap-0.5 rounded-full border bg-muted/40 p-0.5">
			{LESSON_MODES.map((m) => {
				const active = mode === m.id;
				return (
					<Tooltip key={m.id}>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={() => onChange(m.id)}
								className={cn(
									"inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-colors",
									active
										? "bg-background text-foreground shadow-sm"
										: "text-muted-foreground hover:text-foreground",
								)}
								aria-pressed={active}
								aria-label={t("labelLayout", "{{label}} layout", {
									label: m.label,
								})}
							>
								<m.Icon className="h-3.5 w-3.5" />
								<span className="hidden sm:inline">{m.label}</span>
							</button>
						</TooltipTrigger>
						<TooltipContent side="bottom">{m.description}</TooltipContent>
					</Tooltip>
				);
			})}
		</div>
	);
}

interface AppPaneProps {
	readonly target: PaneTarget | null;
	readonly onTargetChange: (target: PaneTarget) => void;
	readonly authSub?: string;
	readonly hub: ReturnType<typeof useHub>["hub"];
	readonly uiEventTypes: string[];
	readonly oauth: OAuthBridge;
}

function AppPane({
	target,
	onTargetChange,
	authSub,
	hub,
	uiEventTypes,
	oauth,
}: AppPaneProps) {
	const { t } = useTranslation();
	const [lastBoardId, setLastBoardId] = useState<string | null>(null);

	useEffect(() => {
		if (target?.mode === "flow" && target.boardId) {
			setLastBoardId(target.boardId);
		}
	}, [target]);

	if (!target) {
		return (
			<aside className="flex h-full flex-col items-center justify-center border-l bg-muted/10 p-8 text-center">
				<div className="max-w-sm space-y-3">
					<Sparkles className="mx-auto h-6 w-6 text-primary" />
					<h3 className="font-medium">
						{t("preparingYourWorkspace", "Preparing your workspace")}
					</h3>
					<p className="text-sm text-muted-foreground">
						{t(
							"theAppForThisLessonIsLoading",
							"The app for this lesson is loading.",
						)}
					</p>
				</div>
			</aside>
		);
	}

	const appId = target.appId;
	const paneTabs: ReadonlyArray<{
		readonly id: string;
		readonly label: string;
		readonly Icon: LucideIcon;
		readonly active: boolean;
		readonly next: PaneTarget;
	}> = [
		{
			id: "board",
			label: t("board", "Board"),
			Icon: Workflow,
			active: target.mode === "flow" || target.mode === "flows",
			next: {
				mode: lastBoardId ? "flow" : "flows",
				appId,
				boardId: lastBoardId ?? undefined,
				label: t("board", "Board"),
			},
		},
		{
			id: "pages",
			label: t("pages", "Pages"),
			Icon: LayoutTemplate,
			active: target.mode === "pages",
			next: { mode: "pages", appId, label: t("pages", "Pages") },
		},
		{
			id: "events",
			label: t("events", "Events"),
			Icon: Zap,
			active: target.mode === "events",
			next: { mode: "events", appId, label: t("events", "Events") },
		},
		{
			id: "use",
			label: t("app", "App"),
			Icon: Play,
			active: target.mode === "use",
			next: {
				mode: "use",
				appId,
				routePath: "/",
				eventId: null,
				label: t("app", "App"),
			},
		},
	];

	return (
		<aside className="flex h-full flex-col overflow-hidden border-l bg-background">
			<div
				className="flex items-center gap-1 border-b bg-muted/20 px-2 py-1.5"
				role="tablist"
			>
				{paneTabs.map((tab) => (
					<button
						key={tab.id}
						type="button"
						role="tab"
						aria-selected={tab.active}
						onClick={() => onTargetChange(tab.next)}
						className={cn(
							"inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs transition-colors",
							tab.active
								? "bg-background font-semibold text-foreground ring-1 ring-border"
								: "text-muted-foreground hover:bg-background/60 hover:text-foreground",
						)}
					>
						<tab.Icon className="size-3.5" />
						{tab.label}
					</button>
				))}
			</div>
			<div className="min-h-0 flex-1 overflow-hidden">
				<AppPaneContent
					target={target}
					onTargetChange={onTargetChange}
					authSub={authSub}
					hub={hub}
					uiEventTypes={uiEventTypes}
					oauth={oauth}
				/>
			</div>
		</aside>
	);
}

function AppPaneContent({
	target,
	onTargetChange,
	authSub,
	hub,
	uiEventTypes,
	oauth,
}: Omit<AppPaneProps, "target"> & { readonly target: PaneTarget }) {
	const { t } = useTranslation();

	if (target.mode === "use") {
		return (
			<UsePageContent
				eventConfig={EVENT_CONFIG}
				notFound={<PaneEmpty title={t("appNotFound", "App not found")} />}
				appId={target.appId}
				routePath={target.routePath ?? "/"}
				eventId={target.eventId ?? null}
				embedded
				onNavigate={(next) =>
					onTargetChange({
						...target,
						routePath: next.routePath ?? target.routePath ?? "/",
						eventId:
							next.eventId === undefined
								? (target.eventId ?? null)
								: next.eventId,
					})
				}
			/>
		);
	}

	if (target.mode === "flow") {
		if (!target.boardId) {
			return (
				<AppFlowsPane appId={target.appId} onTargetChange={onTargetChange} />
			);
		}
		return (
			<div className="h-full min-h-0">
				<FlowWrapper
					boardId={target.boardId}
					appId={target.appId}
					nodeId={target.nodeId}
					version={target.version}
					sub={authSub}
					externalAssistant
				/>
			</div>
		);
	}

	if (target.mode === "flows") {
		return <AppFlowsPane appId={target.appId} onTargetChange={onTargetChange} />;
	}

	if (target.mode === "events") {
		return (
			<div className="h-full min-h-0 overflow-auto p-4">
				<EventsPage
					eventMapping={EVENT_CONFIG}
					uiEventTypes={uiEventTypes}
					tokenStore={oauth.tokenStore as never}
					consentStore={oauth.consentStore as never}
					onStartOAuth={oauth.onStartOAuth}
					onRefreshToken={oauth.onRefreshToken}
					hub={hub}
					appId={target.appId}
					eventId={target.eventId ?? null}
					embedded
					onEventIdChange={(eventId) => onTargetChange({ ...target, eventId })}
					onNavigateToFlow={(flow) =>
						onTargetChange({
							mode: "flow",
							appId: flow.appId,
							boardId: flow.boardId,
							nodeId: flow.nodeId,
							version: flow.version,
							label: t("board", "Board"),
						})
					}
					newEventTemplate={target.newEventTemplate}
				/>
			</div>
		);
	}

	if (target.mode === "pages") {
		return <AppPagesPane appId={target.appId} onTargetChange={onTargetChange} />;
	}

	return <AppConfigPane appId={target.appId} />;
}

function AppConfigPane({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation();
	const backend = useBackend();
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId],
		Boolean(appId),
	);
	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		Boolean(appId),
	);
	const [localApp, setLocalApp] = useState<IApp | undefined>();
	const [localMetadata, setLocalMetadata] = useState<IMetadata | undefined>();

	useEffect(() => {
		setLocalApp(app.data);
	}, [app.data]);

	useEffect(() => {
		setLocalMetadata(metadata.data);
	}, [metadata.data]);

	const hasChanges = useMemo(
		() =>
			Boolean(
				app.data &&
					metadata.data &&
					localApp &&
					localMetadata &&
					(!isEqual(localApp, app.data) ||
						!isEqual(localMetadata, metadata.data)),
			),
		[app.data, localApp, localMetadata, metadata.data],
	);

	const saveChanges = useCallback(async () => {
		if (!localApp || !localMetadata) return;
		await backend.appState.pushAppMeta(appId, localMetadata);
		await backend.appState.updateApp(localApp);
		await Promise.all([app.refetch(), metadata.refetch()]);
		toast.success("App config saved");
	}, [app, appId, backend.appState, localApp, localMetadata, metadata]);

	if (!localApp || !localMetadata) {
		return <PaneEmpty title={t("loadingAppConfig", "Loading app config...")} />;
	}

	return (
		<div className="h-full overflow-auto p-4">
			<AppGeneralSettings
				app={localApp}
				metadata={localMetadata}
				canEdit
				hasChanges={hasChanges}
				onAppChange={setLocalApp}
				onMetadataChange={setLocalMetadata}
				onSave={saveChanges}
				onReset={() => {
					setLocalApp(app.data);
					setLocalMetadata(metadata.data);
				}}
			/>
		</div>
	);
}

function AppPagesPane({
	appId,
	onTargetChange,
}: Readonly<{
	appId: string;
	onTargetChange: (target: PaneTarget) => void;
}>) {
	const { t } = useTranslation();
	const backend = useBackend();
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		Boolean(appId),
		[appId],
	);
	const pageData = useMemo<PageData[]>(() => {
		const timestamp = {
			secs_since_epoch: Math.floor(Date.now() / 1000),
			nanos_since_epoch: 0,
		};
		return (pages.data ?? []).map((page) => ({
			appId,
			pageId: page.pageId,
			boardId: page.boardId ?? null,
			metadata: {
				name: page.name,
				description: page.description ?? "",
				preview_media: [],
				tags: [],
				created_at: timestamp,
				updated_at: timestamp,
			},
		}));
	}, [appId, pages.data]);

	const handleDeletePage = useCallback(
		async (pageId: string, boardId: string | null) => {
			if (!boardId) return;
			await backend.pageState.deletePage(appId, pageId, boardId);
			await pages.refetch();
		},
		[appId, backend.pageState, pages],
	);

	return (
		<TooltipProvider>
			<div className="h-full overflow-auto p-6">
				<PagesSection
					pages={pageData}
					onOpenPage={(pageId, boardId) => {
						const params = new URLSearchParams({ id: pageId, app: appId });
						if (boardId) params.set("board", boardId);
						window.location.href = `/page-builder?${params.toString()}`;
					}}
					onOpenBoard={async (boardId) =>
						onTargetChange({
							mode: "flow",
							appId,
							boardId,
							label: t("board", "Board"),
						})
					}
					onDelete={handleDeletePage}
				/>
			</div>
		</TooltipProvider>
	);
}

function AppFlowsPane({
	appId,
	onTargetChange,
}: Readonly<{
	appId: string;
	onTargetChange: (target: PaneTarget) => void;
}>) {
	const { t } = useTranslation();
	const backend = useBackend();
	const parentRegister = useFlowBoardParentState();
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId],
		Boolean(appId),
	);
	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId],
		Boolean(appId),
	);
	const [boardCreation, setBoardCreation] =
		useState<FlowLibraryBoardCreationState>({
			open: false,
			name: "",
			description: "",
		});

	useEffect(() => {
		if (!boards.data) return;
		for (const board of boards.data) {
			parentRegister?.addBoardParent(
				board.id,
				`/learn/lesson?learnPane=flows&learnPaneAppId=${appId}`,
			);
		}
	}, [appId, boards.data, parentRegister]);

	const handleCreateBoard = useCallback(async () => {
		await backend.boardState.upsertBoard(
			appId,
			createId(),
			boardCreation.name,
			boardCreation.description,
			ILogLevel.Debug,
			IExecutionStage.Dev,
		);
		await Promise.allSettled([boards.refetch(), app.refetch()]);
		setBoardCreation({ open: false, name: "", description: "" });
	}, [appId, app, backend.boardState, boardCreation, boards]);

	return (
		<div className="h-full overflow-auto p-6">
			<div className="flex flex-col gap-4">
				<FlowLibraryHeader
					boardCreation={boardCreation}
					setBoardCreation={setBoardCreation}
					onCreateBoard={handleCreateBoard}
				/>
				<FlowLibraryBoardsSection
					boards={boards}
					app={app.data}
					boardCreation={boardCreation}
					setBoardCreation={setBoardCreation}
					onOpenBoard={async (boardId) =>
						onTargetChange({
							mode: "flow",
							appId,
							boardId,
							label: t("board", "Board"),
						})
					}
					onDeleteBoard={async (boardId) => {
						await backend.boardState.deleteBoard(appId, boardId);
						await boards.refetch();
					}}
				/>
			</div>
		</div>
	);
}

function PaneEmpty({ title }: Readonly<{ title: string }>) {
	return (
		<div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
			<div>
				<FileText className="mx-auto mb-3 h-8 w-8" />
				{title}
			</div>
		</div>
	);
}

/** Panel plumbing every lesson page needs: mode state plus the imperative refs. */
export function useLessonWorkspaceLayout(showSplitView: boolean) {
	const [mode, setMode] = useState<LessonMode>("split");
	const panelGroupRef = useRef<ImperativePanelGroupHandle | null>(null);
	const appPanelRef = useRef<ImperativePanelHandle | null>(null);

	const applyMode = useCallback(
		(next: LessonMode) => {
			setMode(next);
			if (!showSplitView) return;
			const appPanel = appPanelRef.current;
			if (next === "read") {
				appPanel?.collapse();
				return;
			}
			if (appPanel?.isCollapsed()) appPanel.expand();
			panelGroupRef.current?.setLayout([...LESSON_MODE_LAYOUTS[next]]);
		},
		[showSplitView],
	);

	/** Force the workspace open — used when a challenge needs the live board. */
	const revealWorkspace = useCallback(() => {
		const appPanel = appPanelRef.current;
		if (appPanel?.isCollapsed()) {
			appPanel.expand();
			setMode((current) => (current === "read" ? "split" : current));
			panelGroupRef.current?.setLayout([...LESSON_MODE_LAYOUTS.split]);
		}
	}, []);

	return {
		mode,
		setMode,
		applyMode,
		revealWorkspace,
		panelGroupRef,
		appPanelRef,
	};
}
