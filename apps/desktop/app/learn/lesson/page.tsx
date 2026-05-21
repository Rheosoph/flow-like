"use client";
import { createId } from "@paralleldrive/cuid2";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	ChallengeRunner,
	AppGeneralSettings,
	type FlowLibraryBoardCreationState,
	FlowLibraryBoardsSection,
	FlowLibraryHeader,
	IExecutionStage,
	ILogLevel,
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	UsePageContent,
	LessonActionButton,
	LessonContent,
	buildLessonAction,
	type IApp,
	type IEvent,
	type IMetadata,
	type IOAuthProvider,
	type IStoredOAuthToken,
	isEqual,
	useBackend,
	useFlowBoardParentState,
	useHub,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { FlowWrapper } from "@flow-like/flow-like-ui/components/flow/flow-wrapper";
import type {
	BoardSnapshot,
	Challenge,
	ChallengeAttempt,
	LessonAction,
	LessonAppRef,
} from "@flow-like/flow-like-ui";
import {
	type UserLessonProgress,
	translateId,
} from "@flow-like/flow-like-ui/lib/learn/types";
import { BOARD_BRIDGE_NATIVE_EVENT } from "@flow-like/flow-like-ui/lib/learn/board-bridge";
import EventsPage from "@flow-like/flow-like-ui/components/settings/events/events-page";
import {
	PagesSection,
	type PageData,
} from "@flow-like/flow-like-ui/components/settings/routes";
import "@xyflow/react/dist/style.css";
import {
	ArrowLeft,
	BookOpen,
	CheckCircle2,
	Columns2,
	FileText,
	type LucideIcon,
	Sparkles,
	Wrench,
} from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import type {
	ImperativePanelGroupHandle,
	ImperativePanelHandle,
} from "react-resizable-panels";
import {
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { EVENT_CONFIG } from "../../../lib/event-config";
import { learnApi } from "../../../lib/learn-api";
import { oauthConsentStore, oauthTokenStore } from "../../../lib/oauth-db";
import { oauthService } from "../../../lib/oauth-service";

type PaneMode = "use" | "flow" | "flows" | "events" | "pages" | "config";

interface PaneTarget {
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

function routeLabelForLessonSubpath(subpath: string) {
	if (subpath === "config") return "Config";
	if (subpath === "events") return "Events";
	if (subpath === "pages") return "Pages";
	if (subpath === "flow") return "Board";
	if (subpath === "use") return "App";
	return "App";
}

function paneModeForSubpath(subpath: string): PaneMode {
	if (subpath === "events") return "events";
	if (subpath === "pages") return "pages";
	if (subpath === "flow") return "flows";
	if (subpath === "use") return "use";
	return "config";
}

type LessonMode = "read" | "split" | "build";

const LESSON_MODE_LAYOUTS: Record<LessonMode, [number, number]> = {
	read: [100, 0],
	split: [50, 50],
	build: [30, 70],
};

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

function useIsWideScreen() {
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

function LessonModeToggle({
	mode,
	onChange,
}: {
	readonly mode: LessonMode;
	readonly onChange: (mode: LessonMode) => void;
}) {
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
								className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-colors ${
									active
										? "bg-background text-foreground shadow-sm"
										: "text-muted-foreground hover:text-foreground"
								}`}
								aria-pressed={active}
								aria-label={`${m.label} layout`}
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

export default function LessonPage() {
	return (
		<Suspense fallback={null}>
			<LessonContentPage />
		</Suspense>
	);
}

function LessonContentPage() {
	const auth = useAuth();
	const { hub } = useHub();
	const searchParams = useSearchParams();
	const courseId =
		searchParams.get("learnCourseId") ?? searchParams.get("courseId") ?? "";
	const moduleId =
		searchParams.get("learnModuleId") ?? searchParams.get("moduleId") ?? "";
	const lessonId =
		searchParams.get("learnLessonId") ?? searchParams.get("lessonId") ?? "";
	const queryClient = useQueryClient();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const profileId = profile?.id ?? "no-profile";
	const getProfile = useCallback(() => {
		if (!profile) {
			throw new Error("Profile is required for learning API calls.");
		}
		return profile;
	}, [profile]);

	const lessonQuery = useQuery({
		queryKey: ["learn", "lesson", courseId, moduleId, lessonId, profileId],
		enabled: Boolean(profile && courseId && moduleId && lessonId),
		queryFn: () =>
			learnApi.getLesson(getProfile(), auth, courseId, moduleId, lessonId),
	});

	const enrollmentQuery = useQuery({
		queryKey: ["learn", "enrollments", "me", profileId, auth.user?.profile.sub],
		enabled: Boolean(profile && auth.user),
		queryFn: () => learnApi.myEnrollments(getProfile(), auth),
	});

	const enrollment = useMemo(
		() => (enrollmentQuery.data ?? []).find((e) => e.course_id === courseId),
		[enrollmentQuery.data, courseId],
	);
	const linkedAppIds = (enrollment?.linked_app_ids ?? {}) as Record<
		string,
		string
	>;

	const progressQuery = useQuery({
		queryKey: ["learn", "progress", "me", courseId, profileId],
		enabled: Boolean(profile && auth.user && courseId),
		queryFn: () => learnApi.myCourseProgress(getProfile(), auth, courseId),
	});

	const completeMutation = useMutation({
		mutationFn: () => learnApi.markLessonComplete(getProfile(), auth, lessonId),
		onSuccess: (progress) => {
			queryClient.setQueryData<UserLessonProgress[]>(
				["learn", "progress", "me", courseId, profileId],
				(rows = []) => [
					...rows.filter((row) => row.lesson_id !== progress.lesson_id),
					progress,
				],
			);
			queryClient.invalidateQueries({
				queryKey: ["learn", "progress", "me", courseId],
			});
			queryClient.invalidateQueries({
				queryKey: ["learn", "enrollments", "me"],
			});
			queryClient.invalidateQueries({
				queryKey: ["learn", "courses"],
			});
			toast.success("Lesson marked complete");
		},
		onError: (error) => {
			console.error(error);
			toast.error("Could not mark lesson complete");
		},
	});

	const submitAttempt = useCallback(
		async (challengeId: string, submission: unknown) => {
			const result = await learnApi.submitAttempt(
				getProfile(),
				auth,
				challengeId,
				submission,
			);
			queryClient.invalidateQueries({
				queryKey: ["learn", "lesson", courseId, moduleId, lessonId, profileId],
			});
			return result;
		},
		[getProfile, auth, queryClient, courseId, moduleId, lessonId, profileId],
	);

	const resolveAppId = useCallback(
		(alias: string | null, fallback: string | null): string | null => {
			if (!alias) return fallback;
			return linkedAppIds[alias] ?? fallback;
		},
		[linkedAppIds],
	);

	const aliasForApp = useCallback(
		(appId: string): string | null => {
			for (const [alias, mappedAppId] of Object.entries(linkedAppIds)) {
				if (mappedAppId === appId) return alias;
			}
			return null;
		},
		[linkedAppIds],
	);
	const [paneTarget, setPaneTarget] = useState<PaneTarget | null>(null);
	const [paneTouched, setPaneTouched] = useState(false);
	const [lessonMode, setLessonMode] = useState<LessonMode>("split");
	const panelGroupRef = useRef<ImperativePanelGroupHandle>(null);
	const appPanelRef = useRef<ImperativePanelHandle>(null);
	const isWideScreen = useIsWideScreen();
	const uiEventTypes = useMemo(() => {
		const set = new Set<string>();
		Object.values(EVENT_CONFIG).forEach((cfg: any) => {
			Object.keys(cfg?.useInterfaces ?? {}).forEach((type) => set.add(type));
		});
		return Array.from(set);
	}, []);
	const handleStartOAuth = useCallback(async (provider: IOAuthProvider) => {
		await oauthService.startAuthorization(provider);
	}, []);
	const handleRefreshToken = useCallback(
		async (provider: IOAuthProvider, token: IStoredOAuthToken) => {
			return oauthService.refreshToken(provider, token);
		},
		[],
	);

	const resolvePaneAppId = useCallback(
		async (appId: string | null | undefined, appAlias?: string) => {
			if (appId) return appId;
			if (!appAlias || !profile || !courseId) return null;
			const linked = await learnApi.openSharedApp(
				getProfile(),
				auth,
				courseId,
				appAlias,
			);
			queryClient.invalidateQueries({
				queryKey: ["learn", "enrollments", "me"],
			});
			return linked.app_id;
		},
		[profile, courseId, getProfile, auth, queryClient],
	);

	const buildPaneTarget = useCallback(
		async (action: LessonAction): Promise<PaneTarget | null> => {
			switch (action.kind) {
				case "NAVIGATE": {
					const appId = await resolvePaneAppId(action.appId, action.appAlias);
					if (!appId) return null;
					const mode = paneModeForSubpath(action.subpath);
					const params = action.params ?? {};
					if (mode === "flows") {
						const boardId = params.id ?? params.boardId ?? params.board ?? "";
						if (!boardId) {
							return {
								mode: "flows",
								appId,
								label: "Flows",
							};
						}
						const version = params.version
							? (params.version.split("_").map(Number) as [
									number,
									number,
									number,
								])
							: undefined;
						return {
							mode: "flow",
							appId,
							boardId,
							nodeId: params.node ?? params.focus,
							version,
							label: "Board",
						};
					}
					return {
						mode,
						appId,
						routePath: params.route ?? "/",
						eventId: params.eventId ?? null,
						label: routeLabelForLessonSubpath(action.subpath),
					};
				}
				case "FOCUS_NODE": {
					const appId = await resolvePaneAppId(action.appId, action.appAlias);
					if (!appId) return null;
					const alias = action.appAlias ?? aliasForApp(appId);
					const boardId =
						translateId(enrollment ?? null, alias, "boards", action.boardId) ??
						action.boardId;
					const nodeId =
						translateId(enrollment ?? null, alias, "nodes", action.nodeId) ??
						action.nodeId;
					return {
						mode: "flow",
						appId,
						boardId,
						nodeId,
						label: "Board",
					};
				}
				case "ADD_NODE": {
					const appId = await resolvePaneAppId(action.appId, action.appAlias);
					if (!appId) return null;
					const alias = action.appAlias ?? aliasForApp(appId);
					const boardId =
						translateId(enrollment ?? null, alias, "boards", action.boardId) ??
						action.boardId;
					return {
						mode: "flow",
						appId,
						boardId,
						label: "Board",
					};
				}
				case "CREATE_EVENT": {
					const appId = await resolvePaneAppId(action.appId, action.appAlias);
					if (!appId) return null;
					return {
						mode: "events",
						appId,
						newEventTemplate: action.template as Partial<IEvent>,
						label: "Events",
					};
				}
				case "OPEN_OR_CLONE_APP": {
					const appId = await resolvePaneAppId(
						action.sharedAppId,
						action.alias,
					);
					if (!appId) return null;
					return {
						mode: "use",
						appId,
						routePath: "/",
						eventId: null,
						label: "App",
					};
				}
			}
		},
		[resolvePaneAppId, aliasForApp, enrollment],
	);

	const dispatch = useCallback(
		async (action: LessonAction) => {
			const target = await buildPaneTarget(action);
			if (!target) {
				toast.error("Could not open the app for this lesson action");
				return;
			}
			setPaneTarget(target);
			setPaneTouched(true);
		},
		[buildPaneTarget],
	);

	const lesson = lessonQuery.data?.lesson;
	const lessonComplete = useMemo(
		() =>
			(progressQuery.data ?? []).some(
				(p) => p.lesson_id === lessonId && p.status === "COMPLETED",
			),
		[progressQuery.data, lessonId],
	);
	const challenges = useMemo(
		() => lessonQuery.data?.challenges ?? [],
		[lessonQuery.data],
	);
	const attemptsByChallenge = useMemo(() => {
		const byChallenge = new Map<string, ReadonlyArray<ChallengeAttempt>>();
		for (const attempt of lessonQuery.data?.attempts ?? []) {
			const current = byChallenge.get(attempt.challenge_id) ?? [];
			byChallenge.set(attempt.challenge_id, [...current, attempt]);
		}
		return byChallenge;
	}, [lessonQuery.data?.attempts]);
	const appRefs = lessonQuery.data?.app_refs ?? [];
	const assets = lessonQuery.data?.assets ?? [];

	const boardDefaultTarget = useMemo<PaneTarget | null>(() => {
		const boardChallenge = challenges.find(
			(c: Challenge) => c.kind === "BOARD_RIDDLE" || c.kind === "EXECUTE_NODE",
		);
		if (boardChallenge) {
			const payload = boardChallenge.payload as {
				boardId?: string;
				appAlias?: string;
				appId?: string;
			};
			const alias = payload.appAlias ?? null;
			const appId = resolveAppId(alias, payload.appId ?? null);
			const translatedBoard =
				translateId(
					enrollment ?? null,
					alias,
					"boards",
					payload.boardId ?? null,
				) ??
				payload.boardId ??
				undefined;
			if (appId && translatedBoard) {
				return {
					mode: "flow",
					appId,
					boardId: translatedBoard,
					label: "Board",
				};
			}
		}
		return null;
	}, [challenges, resolveAppId, enrollment]);

	const hasAppPane = appRefs.length > 0 || boardDefaultTarget !== null;

	useEffect(() => {
		setPaneTarget(null);
		setPaneTouched(false);
	}, [lessonId]);

	useEffect(() => {
		if (!hasAppPane) {
			setPaneTarget(null);
			return;
		}
		if (paneTouched || paneTarget) return;

		let cancelled = false;
		async function openDefaultPane() {
			const firstAction = appRefs[0]
				? buildLessonAction(appRefs[0], resolveAppId)
				: null;
			const target = firstAction
				? await buildPaneTarget(firstAction)
				: boardDefaultTarget;
			if (!cancelled && target) {
				setPaneTarget(target);
			}
		}

		void openDefaultPane();
		return () => {
			cancelled = true;
		};
	}, [
		hasAppPane,
		paneTouched,
		paneTarget,
		appRefs,
		resolveAppId,
		buildPaneTarget,
		boardDefaultTarget,
	]);

	const buildBoardSubmission = useCallback(async (): Promise<BoardSnapshot> => {
		if (!paneTarget || paneTarget.mode !== "flow") {
			throw new Error(
				"Open the board in the side-by-side pane (Edit board) before checking.",
			);
		}
		return new Promise<BoardSnapshot>((resolve, reject) => {
			const timer = window.setTimeout(() => {
				reject(new Error("Timed out waiting for board state."));
			}, 5000);
			window.dispatchEvent(
				new CustomEvent(BOARD_BRIDGE_NATIVE_EVENT, {
					detail: {
						resolve: (snapshot: BoardSnapshot) => {
							window.clearTimeout(timer);
							resolve(snapshot);
						},
						reject: (error: Error) => {
							window.clearTimeout(timer);
							reject(error);
						},
					},
				}),
			);
		});
	}, [paneTarget]);

	if (!courseId || !moduleId || !lessonId) {
		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-3xl p-6 md:p-10">
					<Card>
						<CardHeader>
							<CardTitle>Lesson missing</CardTitle>
							<CardDescription>
								Open a lesson from a course in the university catalog.
							</CardDescription>
						</CardHeader>
						<CardContent>
							<Button asChild variant="outline">
								<Link href="/learn">
									<ArrowLeft className="mr-2 h-4 w-4" />
									All courses
								</Link>
							</Button>
						</CardContent>
					</Card>
				</div>
			</div>
		);
	}

	const showSplitView = hasAppPane && isWideScreen;

	const applyLessonMode = useCallback(
		(next: LessonMode) => {
			setLessonMode(next);
			if (!showSplitView) return;
			const appPanel = appPanelRef.current;
			const group = panelGroupRef.current;
			if (next === "read") {
				appPanel?.collapse();
				return;
			}
			if (appPanel?.isCollapsed()) {
				appPanel.expand();
			}
			group?.setLayout([...LESSON_MODE_LAYOUTS[next]]);
		},
		[showSplitView],
	);

	const lessonBody = (
		<div className="mx-auto w-full max-w-3xl space-y-6 p-6 md:p-8 lg:p-10">
			{lesson ? (
				<LessonContent lesson={lesson} assets={assets} />
			) : (
				<p className="text-sm text-muted-foreground">Loading…</p>
			)}

			{appRefs.length > 0 && (
				<Card className="overflow-hidden border-primary/20 bg-linear-to-br from-primary/6 via-card to-card shadow-sm">
					<CardHeader className="space-y-1.5 pb-3">
						<div className="flex items-start gap-3">
							<div className="flex h-9 w-9 items-center justify-center rounded-full bg-primary/10 text-primary ring-1 ring-primary/20">
								<Sparkles className="h-4 w-4" />
							</div>
							<div className="space-y-0.5">
								<CardTitle className="text-base">
									Try it in the workspace
								</CardTitle>
								<CardDescription className="text-xs">
									Open the side workspace to apply what you've just learned.
								</CardDescription>
							</div>
						</div>
					</CardHeader>
					<CardContent className="flex flex-wrap gap-2">
						{appRefs.map((r: LessonAppRef) => (
							<LessonActionButton
								key={r.id}
								appRef={r}
								resolveAppId={resolveAppId}
								dispatch={dispatch}
							/>
						))}
					</CardContent>
				</Card>
			)}

			{challenges.length > 0 && (
				<div className="space-y-4">
					{challenges.map((c: Challenge) => {
						const usesBoard =
							c.kind === "BOARD_RIDDLE" || c.kind === "EXECUTE_NODE";
						return (
							<ChallengeRunner
								key={c.id}
								challenge={c}
								onSubmit={(submission) => submitAttempt(c.id, submission)}
								attempts={attemptsByChallenge.get(c.id) ?? []}
								buildBoardSubmission={
									usesBoard ? buildBoardSubmission : undefined
								}
							/>
						);
					})}
				</div>
			)}
		</div>
	);

	return (
		<div className="flex-1 flex flex-col h-full overflow-hidden bg-linear-to-b from-muted/20 via-background to-background">
			<header className="border-b backdrop-blur supports-backdrop-filter:bg-background/70 px-4 py-2.5 flex items-center gap-3">
				<Link
					href={`/learn/course?courseId=${encodeURIComponent(courseId)}`}
					className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
				>
					<ArrowLeft className="h-3.5 w-3.5" />
					Back to course
				</Link>
				<div className="ml-auto flex items-center gap-3">
					{showSplitView && (
						<LessonModeToggle mode={lessonMode} onChange={applyLessonMode} />
					)}
					<Button
						variant={lessonComplete ? "secondary" : "default"}
						size="sm"
						disabled={completeMutation.isPending || !lesson || lessonComplete}
						onClick={() => completeMutation.mutate()}
						className="gap-1.5"
					>
						<CheckCircle2 className="h-3.5 w-3.5" />
						{lessonComplete ? "Completed" : "Mark complete"}
					</Button>
				</div>
			</header>

			{showSplitView ? (
				<ResizablePanelGroup
					ref={panelGroupRef}
					direction="horizontal"
					autoSaveId="lesson-workspace-layout"
					className="flex-1 min-h-0"
				>
					<ResizablePanel
						defaultSize={50}
						minSize={28}
						className="min-h-0"
					>
						<section className="h-full overflow-auto">{lessonBody}</section>
					</ResizablePanel>
					<ResizableHandle withHandle className="bg-border/60" />
					<ResizablePanel
						ref={appPanelRef}
						defaultSize={50}
						minSize={25}
						collapsible
						collapsedSize={0}
						onCollapse={() => setLessonMode("read")}
						onExpand={() =>
							setLessonMode((m) => (m === "read" ? "split" : m))
						}
						className="min-h-0"
					>
						<AppPane
							target={paneTarget}
							onTargetChange={setPaneTarget}
							authSub={auth.user?.profile?.sub}
							hub={hub}
							uiEventTypes={uiEventTypes}
							onStartOAuth={handleStartOAuth}
							onRefreshToken={handleRefreshToken}
						/>
					</ResizablePanel>
				</ResizablePanelGroup>
			) : (
				<section className="flex-1 overflow-auto">{lessonBody}</section>
			)}
		</div>
	);
}

interface AppPaneProps {
	readonly target: PaneTarget | null;
	readonly onTargetChange: (target: PaneTarget) => void;
	readonly authSub?: string;
	readonly hub: ReturnType<typeof useHub>["hub"];
	readonly uiEventTypes: string[];
	readonly onStartOAuth: (provider: IOAuthProvider) => Promise<void>;
	readonly onRefreshToken: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
}

function AppPane({
	target,
	onTargetChange,
	authSub,
	hub,
	uiEventTypes,
	onStartOAuth,
	onRefreshToken,
}: AppPaneProps) {
	if (!target) {
		return (
			<aside className="flex h-full flex-col border-l bg-muted/10 items-center justify-center p-8 text-center">
				<div className="max-w-sm space-y-3">
					<div className="rounded-2xl bg-linear-to-br from-primary/10 via-background to-background border p-6 shadow-sm">
						<Sparkles className="h-8 w-8 mx-auto text-primary" />
					</div>
					<h3 className="font-medium">Preparing your workspace</h3>
					<p className="text-sm text-muted-foreground">
						The app for this lesson is loading.
					</p>
				</div>
			</aside>
		);
	}

	return (
		<aside className="flex h-full flex-col border-l bg-background overflow-hidden">
			<div className="flex items-center gap-2 border-b bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
				<Badge variant="outline" className="bg-background">
					{target.label ?? (target.mode === "flow" ? "Board" : "App")}
				</Badge>
				<code className="truncate">
					{target.mode === "flow" ? target.boardId : target.appId}
				</code>
				{target.mode === "flow" && (
					<Button
						variant="outline"
						size="sm"
						className="ml-auto h-6"
						onClick={() =>
							onTargetChange({
								mode: "use",
								appId: target.appId,
								routePath: "/",
								eventId: null,
								label: "App",
							})
						}
					>
						Open app
					</Button>
				)}
			</div>
			<div className="flex-1 min-h-0 overflow-hidden">
				<AppPaneContent
					target={target}
					onTargetChange={onTargetChange}
					authSub={authSub}
					hub={hub}
					uiEventTypes={uiEventTypes}
					onStartOAuth={onStartOAuth}
					onRefreshToken={onRefreshToken}
				/>
			</div>
		</aside>
	);
}

interface AppPaneContentProps {
	readonly target: PaneTarget;
	readonly onTargetChange: (target: PaneTarget) => void;
	readonly authSub?: string;
	readonly hub: ReturnType<typeof useHub>["hub"];
	readonly uiEventTypes: string[];
	readonly onStartOAuth: (provider: IOAuthProvider) => Promise<void>;
	readonly onRefreshToken: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
}

function AppPaneContent({
	target,
	onTargetChange,
	authSub,
	hub,
	uiEventTypes,
	onStartOAuth,
	onRefreshToken,
}: AppPaneContentProps) {
	if (target.mode === "use") {
		return (
			<UsePageContent
				eventConfig={EVENT_CONFIG}
				notFound={<PaneEmpty title="App not found" />}
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
					tokenStore={oauthTokenStore}
					consentStore={oauthConsentStore}
					onStartOAuth={onStartOAuth}
					onRefreshToken={onRefreshToken}
					hub={hub}
					appId={target.appId}
					eventId={target.eventId ?? null}
					embedded
					onEventIdChange={(eventId) =>
						onTargetChange({ ...target, eventId })
					}
					onNavigateToFlow={(flow) =>
						onTargetChange({
							mode: "flow",
							appId: flow.appId,
							boardId: flow.boardId,
							nodeId: flow.nodeId,
							version: flow.version,
							label: "Board",
						})
					}
					newEventTemplate={target.newEventTemplate}
				/>
			</div>
		);
	}

	if (target.mode === "pages") {
		return (
			<AppPagesPane appId={target.appId} onTargetChange={onTargetChange} />
		);
	}

	return <AppConfigPane appId={target.appId} />;
}

function AppConfigPane({ appId }: Readonly<{ appId: string }>) {
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
		return <PaneEmpty title="Loading app config..." />;
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
	const backend = useBackend();
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		Boolean(appId),
		[appId],
	);
	const pageData = useMemo<PageData[]>(
		() => {
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
		},
		[appId, pages.data],
	);

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
						const params = new URLSearchParams({
							id: pageId,
							app: appId,
						});
						if (boardId) params.set("board", boardId);
						window.location.href = `/page-builder?${params.toString()}`;
					}}
					onOpenBoard={async (boardId) =>
						onTargetChange({
							mode: "flow",
							appId,
							boardId,
							label: "Board",
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
	const backend = useBackend();
	const parentRegister = useFlowBoardParentState();
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId],
		Boolean(appId),
	);
	const boards = useInvoke(
		backend.boardState.getBoards,
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
		boards.data.forEach((board) => {
			parentRegister?.addBoardParent(
				board.id,
				`/learn/lesson?learnPane=flows&learnPaneAppId=${appId}`,
			);
		});
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
							label: "Board",
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
