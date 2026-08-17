"use client";
import {
	Button,
	ChallengeRunner,
	type IEvent,
	type IOAuthProvider,
	type IStoredOAuthToken,
	LessonActionButton,
	LessonContent,
	LessonWorkspace,
	type PaneTarget,
	buildLessonAction,
	paneModeForSubpath,
	routeLabelForLessonSubpath,
	useBackend,
	useHub,
	useInvoke,
	useIsWideScreen,
	useLessonWorkspaceLayout,
} from "@flow-like/flow-like-ui";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";
import type {
	BoardSnapshot,
	Challenge,
	ChallengeAttempt,
	LessonAction,
	LessonAppRef,
} from "@flow-like/flow-like-ui";
import { BOARD_BRIDGE_NATIVE_EVENT } from "@flow-like/flow-like-ui/lib/learn/board-bridge";
import {
	type UserLessonProgress,
	translateId,
} from "@flow-like/flow-like-ui/lib/learn/types";
import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import "@xyflow/react/dist/style.css";
import { ArrowLeft } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { learnApi } from "../../../lib/learn-api";
import { oauthConsentStore, oauthTokenStore } from "../../../lib/oauth-db";
import { oauthService } from "../../../lib/oauth-service";

export default function LessonPage() {
	return (
		<Suspense fallback={null}>
			<LessonContentPage />
		</Suspense>
	);
}

function LessonContentPage() {
	const { t } = useTranslation("common");
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

	/** Same key as the course page, so arriving from there costs no request. */
	const structureQuery = useQuery({
		queryKey: ["learn", "structure", courseId, profileId],
		enabled: Boolean(profile && courseId),
		queryFn: () => learnApi.getCourseStructure(getProfile(), auth, courseId),
	});

	const courseLessons = useMemo(
		() =>
			(structureQuery.data?.modules ?? []).flatMap((m) =>
				m.lessons.map((l) => ({
					id: l.id,
					title: l.title,
					moduleId: m.id,
				})),
			),
		[structureQuery.data],
	);
	const lessonIndex = useMemo(
		() => courseLessons.findIndex((l) => l.id === lessonId),
		[courseLessons, lessonId],
	);
	const previousLesson =
		lessonIndex > 0 ? courseLessons[lessonIndex - 1] : null;
	const nextLesson =
		lessonIndex >= 0 && lessonIndex < courseLessons.length - 1
			? courseLessons[lessonIndex + 1]
			: null;
	const lessonHref = useCallback(
		(targetModuleId: string, targetLessonId: string) =>
			`/learn/lesson?learnCourseId=${encodeURIComponent(courseId)}&learnModuleId=${encodeURIComponent(targetModuleId)}&learnLessonId=${encodeURIComponent(targetLessonId)}`,
		[courseId],
	);

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
								label: t("flows", "Flows"),
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
							label: t("board", "Board"),
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
						label: t("board", "Board"),
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
						label: t("board", "Board"),
					};
				}
				case "CREATE_EVENT": {
					const appId = await resolvePaneAppId(action.appId, action.appAlias);
					if (!appId) return null;
					return {
						mode: "events",
						appId,
						newEventTemplate: action.template as Partial<IEvent>,
						label: t("events", "Events"),
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
						label: t("app", "App"),
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
					label: t("board", "Board"),
				};
			}
		}
		return null;
	}, [challenges, resolveAppId, enrollment]);

	const hasAppPane = appRefs.length > 0 || boardDefaultTarget !== null;
	const showSplitView = hasAppPane && isWideScreen;
	const { mode, applyMode, revealWorkspace, panelGroupRef, appPanelRef } =
		useLessonWorkspaceLayout(showSplitView);

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

	const requestBoardSnapshot = useCallback(
		(timeoutMs: number) =>
			new Promise<BoardSnapshot>((resolve, reject) => {
				const timer = window.setTimeout(() => {
					reject(new Error("Timed out waiting for board state."));
				}, timeoutMs);
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
			}),
		[],
	);

	/**
	 * A board challenge reads live board state, so the board has to be open. That
	 * is something the UI can do on the learner's behalf — opening the pane here
	 * beats failing the check and telling them to go open it themselves.
	 */
	const buildBoardSubmission = useCallback(async (): Promise<BoardSnapshot> => {
		if (paneTarget?.mode === "flow" && paneTarget.boardId) {
			return requestBoardSnapshot(5000);
		}

		if (!boardDefaultTarget) {
			throw new Error(
				t(
					"thisChallengeNeedsABoardThatIsNotLinkedToTheLesson",
					"This challenge needs a board, but none is linked to the lesson.",
				),
			);
		}

		setPaneTarget(boardDefaultTarget);
		setPaneTouched(true);
		revealWorkspace();

		for (let attempt = 0; attempt < 5; attempt++) {
			try {
				return await requestBoardSnapshot(1500);
			} catch {
				// The board is still mounting — the bridge answers once it is ready.
			}
		}
		throw new Error(
			t(
				"couldNotReadTheBoardOpenItInTheWorkspacePaneAndTryAgain",
				"Could not read the board. Open it in the workspace pane and try again.",
			),
		);
	}, [
		paneTarget,
		boardDefaultTarget,
		requestBoardSnapshot,
		revealWorkspace,
		t,
	]);

	if (!courseId || !moduleId || !lessonId) {
		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-3xl p-6 md:p-10">
					<div className="rounded-xl border border-border/70 p-6">
						<h1 className="text-lg font-semibold">
							{t("lessonMissing", "Lesson missing")}
						</h1>
						<p className="mt-1 text-sm text-muted-foreground">
							{t(
								"openALessonFromACourseInTheUniversityCatalog",
								"Open a lesson from a course in the university catalog.",
							)}
						</p>
						<Button asChild variant="outline" className="mt-4">
							<Link href="/learn">
								<ArrowLeft className="mr-2 h-4 w-4" />
								{t("allCourses", "All courses")}
							</Link>
						</Button>
					</div>
				</div>
			</div>
		);
	}

	const lessonBody = (
		<div className="mx-auto w-full max-w-5xl space-y-6 p-6 md:p-8 lg:p-10">
			{lesson ? (
				<LessonContent lesson={lesson} assets={assets} />
			) : (
				<p className="text-sm text-muted-foreground">
					{t("loading", "Loading…")}
				</p>
			)}

			{appRefs.length > 0 && (
				<section className="rounded-lg border border-border/70 border-l-2 border-l-primary bg-card p-4">
					<p className="font-mono text-[10px] uppercase tracking-wider text-primary">
						{t("doItInTheWorkspace", "Do it in the workspace")}
					</p>
					<p className="mt-1.5 text-sm text-muted-foreground">
						{showSplitView
							? t(
									"theseOpenInThePaneBesideTheLesson",
									"These open in the pane beside the lesson — nothing leaves this screen.",
								)
							: t(
									"theseOpenTheWorkspaceBelowTheLesson",
									"These open the workspace below the lesson.",
								)}
					</p>
					<div className="mt-3 flex flex-wrap gap-2">
						{appRefs.map((r: LessonAppRef) => (
							<LessonActionButton
								key={r.id}
								appRef={r}
								resolveAppId={resolveAppId}
								dispatch={dispatch}
							/>
						))}
					</div>
				</section>
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

			{(previousLesson || nextLesson) && (
				<nav className="flex gap-3 border-t pt-5">
					{previousLesson && (
						<Link
							href={lessonHref(previousLesson.moduleId, previousLesson.id)}
							className="flex-1 rounded-lg border border-border/70 p-3 transition-colors hover:bg-muted/50"
						>
							<span className="block font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
								{t("previous", "Previous")}
							</span>
							<span className="mt-0.5 block truncate text-sm font-semibold">
								{previousLesson.title}
							</span>
						</Link>
					)}
					{nextLesson && (
						<Link
							href={lessonHref(nextLesson.moduleId, nextLesson.id)}
							className="flex-1 rounded-lg border border-border/70 p-3 text-right transition-colors hover:bg-muted/50"
						>
							<span className="block font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
								{t("next", "Next")}
							</span>
							<span className="mt-0.5 block truncate text-sm font-semibold">
								{nextLesson.title}
							</span>
						</Link>
					)}
				</nav>
			)}
		</div>
	);

	return (
		<LessonWorkspace
			courseHref={`/learn/course?courseId=${encodeURIComponent(courseId)}`}
			BackLink={({ href, className, children }) => (
				<Link href={href} className={className}>
					{children}
				</Link>
			)}
			lessonIndex={lessonIndex}
			lessonCount={courseLessons.length}
			estimatedMinutes={lesson?.estimated_minutes}
			lessonComplete={lessonComplete}
			completePending={completeMutation.isPending}
			canComplete={Boolean(lesson)}
			onMarkComplete={() => completeMutation.mutate()}
			showSplitView={showSplitView}
			mode={mode}
			onModeChange={applyMode}
			panelGroupRef={panelGroupRef}
			appPanelRef={appPanelRef}
			paneTarget={paneTarget}
			onPaneTargetChange={setPaneTarget}
			authSub={auth.user?.profile?.sub}
			hub={hub}
			uiEventTypes={uiEventTypes}
			oauth={{
				tokenStore: oauthTokenStore,
				consentStore: oauthConsentStore,
				onStartOAuth: handleStartOAuth,
				onRefreshToken: handleRefreshToken,
			}}
		>
			{lessonBody}
		</LessonWorkspace>
	);
}
