"use client";

import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";

import { BOTS_ROUTE } from "@flow-like/flow-like-ui/components/bots-tab/dock-selection";
import { useInvoke } from "@flow-like/flow-like-ui/hooks/use-invoke";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import {
	type NativeActionRequest,
	assertNativeActionCurrent,
	cancelNativeAction,
	completeNativeAction,
	nativeActionErrorOutcome,
	nativeActionRunIds,
	subscribeNativeActionLifecycle,
} from "@flow-like/flow-like-ui/lib/native-action-result";
import type { NativeCustomWidget } from "@flow-like/flow-like-ui/lib/native-widget";
import {
	RECENT_APPS_CHANGED,
	readRecentApps,
} from "@flow-like/flow-like-ui/lib/recent-apps";
import {
	useBackend,
	useBackendReady,
} from "@flow-like/flow-like-ui/state/backend-state";
import type { IBotTurn } from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import { useExecutionEngine } from "@flow-like/flow-like-ui/state/execution-engine-context";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import { cancelGlobalChatRun } from "@flow-like/flow-like-ui/state/global-chat/global-chat-stream";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { usePathname, useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useNativeComposerCapture } from "../hooks/use-native-composer-capture";
import { createNativeAppIconPublisher } from "../lib/native-app-icons";
import {
	isNativeDeliveryRetryable,
	nativeBotUrl,
	withNativeBotTurns,
} from "../lib/native-bots";
import { createNativeCustomWidgetPublisher } from "../lib/native-custom-widgets";
import {
	NativeEventInteractionRequired,
	executeNativeEventWithResult,
} from "../lib/native-event-execution";
import {
	type NativeEventCatalog,
	type NativePendingAction,
	type NativeSnapshot,
	dispatchNativeAction,
	loadNativeSnapshot,
	nativeNavigationRequest,
	nativeWebOrigin,
	withNativeActivePage,
	withNativeActiveRuns,
} from "../lib/native-integration";
import {
	type NativeNotificationIconSource,
	createNativeNotificationIconResolver,
} from "../lib/native-notification-icons";
import { readNativeSharedFiles } from "../lib/native-shared-files";
import {
	type NativeBotInput,
	NativeBotInputDialog,
	type NativeBotInputOperation,
} from "./native-bot-input-dialog";
import {
	NativeEventResultDialog,
	type NativeEventRunView,
} from "./native-event-result-dialog";
import {
	type NativeMcpRequest,
	NativeMcpRunDialog,
} from "./native-mcp-run-dialog";

const FOCUS_REFRESH_MS = 60_000;
const PERIODIC_REFRESH_MS = 10 * 60_000;

export function NativeIntegrationProvider() {
	const queryClient = useQueryClient();
	const backend = useBackend();
	const ready = useBackendReady();
	const auth = useAuth();
	const engine = useExecutionEngine();
	const router = useClientRouter();
	const pathname = usePathname();
	const query = useSearchParams().toString();
	// Observe the same query as ProfileSyncer. Its pushProfile mutates the backend
	// without notifying React, so backend readiness alone cannot identify this workspace.
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		ready && !auth.isLoading,
		[],
		0,
	);
	const identityReady =
		ready &&
		!auth.isLoading &&
		profile.isSuccess &&
		profile.isFetchedAfterMount &&
		Boolean(profile.data?.id) &&
		(!auth.isAuthenticated || Boolean(auth.user?.profile.sub));
	const scope = JSON.stringify([
		getApiOrigin(profile.data),
		profile.data?.id ?? "",
		(auth.isAuthenticated ? auth.user?.profile.sub : undefined) ?? "local",
	]);
	const currentScope = useRef<string | undefined>(undefined);
	currentScope.current = identityReady ? scope : undefined;
	useNativeComposerCapture(identityReady ? scope : undefined);
	const writes = useRef(Promise.resolve());
	const automationRequests = useRef(new Map<string, NativePendingAction>());
	const previousScope = useRef<string | undefined>(undefined);
	const [mcpRequests, setMcpRequests] = useState<NativeMcpRequest[]>([]);
	const [eventRuns, setEventRuns] = useState<NativeEventRunView[]>([]);
	const [botInput, setBotInput] = useState<NativeBotInput>();
	const webOrigin = useQuery({
		queryKey: ["native-web-origin", scope],
		queryFn: async () => {
			const origin = getApiOrigin(profile.data);
			if (origin === "https://api.flow-like.com")
				return "https://app.flow-like.com";
			const response = await fetch(`${origin}/api/v1`);
			if (!response.ok) throw new Error("Hub app address is unavailable.");
			return nativeWebOrigin((await response.json()).app) ?? null;
		},
		enabled:
			identityReady &&
			isTauri() &&
			typeof navigator !== "undefined" &&
			/Mac|iPhone|iPad|iPod|Android/.test(navigator.userAgent),
		staleTime: 300_000,
	});
	const navigation = useRef({
		router,
		pathname,
		query,
		webOrigin: webOrigin.data,
	});
	navigation.current = { router, pathname, query, webOrigin: webOrigin.data };
	const publishCurrentPage = useRef<(() => Promise<void>) | undefined>(
		undefined,
	);

	// biome-ignore lint/correctness/useExhaustiveDependencies: Route publication must not restart in-flight native action delivery.
	useEffect(() => {
		void publishCurrentPage.current?.();
	}, [pathname, query, webOrigin.data]);

	useEffect(() => {
		if (!isTauri() || !/Mac|iPhone|iPad|iPod|Android/.test(navigator.userAgent))
			return;
		let stopped = false;
		let refreshPromise: Promise<void> | undefined;
		let draining = false;
		let drainRequested = false;
		let baseSnapshot: NativeSnapshot | undefined;
		const botTurns = new Map<string, IBotTurn>();
		const botTurnRevisions = new Map<string, number>();
		let botStatusRevision = 0;
		let lastBotStatusRead = 0;
		let botStatusKnown = false;
		let customWidgets: NativeCustomWidget[] = [];
		let activeSignature = "";
		const handling = new Set<string>();
		const handled = new Set<string>();
		const wasHandled = (key: string) => {
			if (handled.has(key)) return true;
			try {
				return sessionStorage.getItem(key) !== null;
			} catch {
				return false;
			}
		};
		const markHandled = (key: string) => {
			handled.add(key);
			try {
				sessionStorage.setItem(key, "handled");
			} catch {
				// In-memory deduplication also works when webview storage is unavailable.
			}
		};
		const current = () => !stopped && currentScope.current === scope;
		const acceptBotStatus = (turns: readonly IBotTurn[], revision: number) => {
			if (!current() || revision < lastBotStatusRead) return;
			lastBotStatusRead = revision;
			botStatusKnown = true;
			// A response cannot undo turn events received while its request was in flight.
			for (const [id, observed] of botTurnRevisions) {
				if (observed > revision) continue;
				botTurns.delete(id);
				botTurnRevisions.delete(id);
			}
			for (const turn of turns) {
				if ((botTurnRevisions.get(turn.id) ?? 0) > revision) continue;
				botTurns.set(turn.id, turn);
				botTurnRevisions.set(turn.id, revision);
			}
		};
		const automationKey = (request: NativeActionRequest) =>
			JSON.stringify([request.scope, request.id]);
		const stopAutomation = (request: NativeActionRequest) => {
			for (const runId of nativeActionRunIds(request))
				void cancelGlobalChatRun(runId).catch(() => {});
			void engine.cancelTask(`native-${request.id}`).catch(() => {});
			const matches = (item?: NativeActionRequest) =>
				item?.id === request.id && item?.scope === request.scope;
			useGlobalChatStore.setState((state) => {
				const drafts = [state.draft, ...state.nativeDrafts].filter(
					(draft) => draft && !matches(draft.nativeRequest),
				);
				return {
					draft: drafts[0] ?? null,
					nativeDrafts: drafts.slice(1) as typeof state.nativeDrafts,
					queue: state.queue.filter((item) => !matches(item.nativeRequest)),
				};
			});
			setEventRuns((runs) => runs.filter((run) => !matches(run)));
			setBotInput((input) => (input && matches(input) ? undefined : input));
		};
		const enqueueWrite = (fn: () => Promise<unknown>) => {
			writes.current = writes.current
				.catch(() => {})
				.then(async () => {
					if (current()) await fn();
				})
				.catch((error) => console.warn("Native integration cache:", error));
			return writes.current;
		};
		if (!identityReady)
			return () => {
				stopped = true;
			};
		if (previousScope.current && previousScope.current !== scope) {
			// Invalidation must survive effect cleanup and precede the next account's publication.
			writes.current = writes.current
				.catch(() => {})
				.then(() => invoke("native_clear_snapshot"))
				.then(() => {})
				.catch((error) => console.warn("Native cache invalidation:", error));
			setMcpRequests([]);
			setEventRuns([]);
			setBotInput(undefined);
			previousScope.current = scope;
		}
		previousScope.current = scope;
		const offLifecycle = subscribeNativeActionLifecycle((request, state) => {
			if (request.scope !== scope || !request.automation) return;
			automationRequests.current.delete(automationKey(request));
			if (state === "cancelled") stopAutomation(request);
		});
		const onAutomationCancel = (event: Event) => {
			const detail = (event as CustomEvent).detail;
			if (detail?.scope !== scope || typeof detail?.id !== "string") return;
			const request = automationRequests.current.get(automationKey(detail));
			if (request) cancelNativeAction(request);
		};
		let checkingAutomations = false;
		const checkAutomations = async () => {
			if (checkingAutomations) return;
			checkingAutomations = true;
			try {
				for (const request of automationRequests.current.values()) {
					if (request.scope !== scope) continue;
					try {
						assertNativeActionCurrent(request, currentScope.current);
						if (
							!(await invoke<boolean>("native_automation_current", {
								id: request.id,
								scope: request.scope,
							}))
						)
							cancelNativeAction(request);
					} catch {
						cancelNativeAction(request);
					}
				}
			} finally {
				checkingAutomations = false;
			}
		};
		window.addEventListener(
			"flow-like-native-action-cancel",
			onAutomationCancel,
		);
		const automationTimer = window.setInterval(() => {
			void checkAutomations();
		}, 1000);
		const icons = createNativeAppIconPublisher({
			publish: async (payload) => {
				if (current()) await invoke("native_publish_app_icons", { ...payload });
			},
		});
		const notificationIcons = createNativeNotificationIconResolver();
		const publish = () => {
			if (!baseSnapshot || !current()) return Promise.resolve();
			const runs = engine.getActiveExecutions(scope);
			activeSignature = JSON.stringify(runs);
			const snapshot = withNativeActivePage(
				withNativeBotTurns(
					withNativeActiveRuns({ ...baseSnapshot, customWidgets }, runs),
					[...botTurns.values()],
					botStatusKnown,
				),
				navigation.current.pathname,
				navigation.current.query,
				navigation.current.webOrigin || undefined,
			);
			return enqueueWrite(() =>
				invoke("native_publish_snapshot", { snapshot }),
			);
		};
		const customWidgetPublisher = createNativeCustomWidgetPublisher({
			backend,
			scope,
			viewerId: auth.isAuthenticated ? auth.user?.profile.sub : undefined,
			isCurrent: current,
			publish: async (widgets) => {
				customWidgets = widgets;
				await publish();
			},
		});
		customWidgetPublisher.start();
		publishCurrentPage.current = publish;
		let lastRefreshAt = 0;
		const eventCatalog: NativeEventCatalog = new Map();
		const refresh = (maxAgeMs = 0): Promise<void> => {
			if (refreshPromise) return refreshPromise;
			if (
				!current() ||
				document.visibilityState !== "visible" ||
				Date.now() - lastRefreshAt < maxAgeMs
			)
				return Promise.resolve();
			lastRefreshAt = Date.now();
			const statusRevision = ++botStatusRevision;
			refreshPromise = (async () => {
				try {
					let notificationSources: NativeNotificationIconSource[] = [];
					const snapshot = await loadNativeSnapshot(
						backend,
						scope,
						readRecentApps(JSON.parse(scope)),
						auth.isAuthenticated,
						new Date(),
						(sources) => {
							notificationSources = sources;
						},
						eventCatalog,
						(turns) => acceptBotStatus(turns, statusRevision),
					);
					if (current()) {
						baseSnapshot = snapshot;
						await publish();
						if (current() && notificationSources.length)
							void notificationIcons
								.resolve(scope, notificationSources)
								.then(async (resolved) => {
									if (!current() || baseSnapshot !== snapshot) return;
									baseSnapshot = {
										...snapshot,
										sections: snapshot.sections.map((section) =>
											section.kind === "inbox"
												? {
														...section,
														items: section.items.map((item) => ({
															...item,
															icon: resolved[item.id],
														})),
													}
												: section,
										),
									};
									await publish();
								})
								.catch(() => {});
						if (current())
							void icons
								.sync({
									backend,
									scope,
									appIds: snapshot.apps.map((app) => app.id),
								})
								.catch((error) => console.warn("Native app icons:", error));
					}
				} catch (error) {
					console.warn("Native snapshot refresh:", error);
				} finally {
					refreshPromise = undefined;
				}
			})();
			return refreshPromise;
		};
		const reportError = (error: unknown) =>
			toast.error(
				error instanceof Error
					? error.message
					: "This native action could not complete.",
			);
		const handle = async (request: NativePendingAction): Promise<boolean> => {
			if (!current()) return false;
			const key = `flow-like:native-action:${scope}:${request.id}`;
			const acknowledge = async () => {
				if (!current()) return;
				try {
					await invoke("native_ack_action", {
						id: request.id,
						scope: request.scope,
					});
				} catch (error) {
					console.warn("Native action acknowledgement:", error);
				}
			};
			if (handling.has(key)) return false;
			if (wasHandled(key)) {
				await acknowledge();
				return true;
			}
			handling.add(key);
			try {
				if (request.responseMode && request.automation) {
					assertNativeActionCurrent(request, scope);
					if (
						!(await invoke<boolean>("native_automation_current", {
							id: request.id,
							scope,
						}))
					)
						throw new Error("This automation was cancelled or expired.");
					automationRequests.current.set(automationKey(request), request);
				}
				const requestCurrent = () => {
					try {
						assertNativeActionCurrent(request, currentScope.current);
						return current();
					} catch {
						return false;
					}
				};
				await dispatchNativeAction(request, {
					scope,
					backend,
					getWebOrigin: async () =>
						navigation.current.webOrigin ||
						(await webOrigin.refetch()).data ||
						undefined,
					navigate: (url) => navigation.current.router.push(url),
					isCurrent: requestCurrent,
					complete: async (outcome) => {
						void queryClient.invalidateQueries({ queryKey: ["inbox"] });
						await completeNativeAction(request, outcome, {
							getCurrentScope: () => currentScope.current,
							invoke,
						});
					},
					bot: async (action) => {
						if (!action.botId || !requestCurrent()) return;
						const operation: NativeBotInputOperation | undefined =
							action.kind === "private_bot_clipboard"
								? "private_clipboard"
								: action.kind === "capture_bot_voice" || action.voice
									? "voice"
									: action.kind === "capture_bot_document"
										? action.operation === "photo"
											? "photo"
											: "document"
										: action.kind === "local_bot_summary"
											? action.operation === "extract"
												? "extract"
												: "summarize"
											: undefined;
						navigation.current.router.push(nativeBotUrl(action));
						if (operation) {
							setBotInput({
								id: request.id,
								scope,
								botId: action.botId,
								operation,
								text: action.prompt || action.text,
							});
							return;
						}
						const files = await readNativeSharedFiles(action.files ?? []);
						if (!requestCurrent()) return;
						const prompt = action.prompt || action.text || "";
						if (!prompt && !files.length) return;
						markHandled(key);
						useGlobalChatStore.getState().setDraft({
							prompt,
							files,
							botId: action.botId,
							nativeScope: scope,
							nativeRequest: request,
						});
					},
					flowpilot: async (prompt, voice, paths) => {
						// Opening FlowPilot must not wait for files to cross the native bridge.
						navigation.current.router.push(
							voice ? `${BOTS_ROUTE}?voice=1` : BOTS_ROUTE,
						);
						const files = await Promise.all(
							(paths ?? []).map(async (path) => {
								const file = await invoke<{
									bytes: number[];
									name: string;
									mimeType?: string;
								}>("native_read_shared_file", { path });
								return new File([new Uint8Array(file.bytes)], file.name, {
									type: file.mimeType || "application/octet-stream",
								});
							}),
						);
						if (!requestCurrent()) return;
						if (prompt || files.length) {
							markHandled(key);
							useGlobalChatStore.getState().setDraft({
								prompt: prompt || "",
								files,
								nativeScope: scope,
								nativeRequest: request,
							});
						} else if (request.responseMode)
							throw new Error("Provide a question for FlowPilot.");
					},
					callEvent: (appId, event, input) => {
						if (!requestCurrent()) return;
						markHandled(key);
						const isCurrent = () => {
							try {
								if (request.automation)
									assertNativeActionCurrent(request, currentScope.current);
								return currentScope.current === scope;
							} catch {
								return false;
							}
						};
						const completion = {
							getCurrentScope: () => currentScope.current,
							invoke,
						};
						const update = (patch: Partial<NativeEventRunView>) => {
							if (isCurrent())
								setEventRuns((runs) =>
									runs.map((run) =>
										run.id === request.id && run.scope === scope
											? { ...run, ...patch }
											: run,
									),
								);
						};
						setEventRuns((runs) => [
							...runs.filter((run) => run.scope === scope),
							{
								id: request.id,
								scope,
								appId,
								eventId: event.id,
								title: event.name,
								input,
								status: "running",
								output: { text: "" },
								interactions: [],
							},
						]);
						// Accept delivery now; execution completes the separate native response request.
						void (async () => {
							try {
								const output = await executeNativeEventWithResult({
									request,
									appId,
									event,
									input,
									backend,
									engine,
									isCurrent,
									onUpdate: (output, interactions) => {
										if (!isCurrent()) return;
										setEventRuns((runs) =>
											runs.map((run) =>
												run.id === request.id && run.scope === scope
													? {
															...run,
															output,
															interactions: [
																...new Map(
																	[...run.interactions, ...interactions].map(
																		(item) => [item.id, item],
																	),
																).values(),
															],
														}
													: run,
											),
										);
									},
								});
								update({ status: "complete", output, interactions: [] });
								await completeNativeAction(
									request,
									{ status: "success", ...output },
									completion,
								);
							} catch (error) {
								const outcome = nativeActionErrorOutcome(error);
								if (error instanceof NativeEventInteractionRequired)
									outcome.status = "interaction_required";
								update({
									status: "error",
									error: outcome.error,
									interactions: [],
								});
								await completeNativeAction(request, outcome, completion);
							}
						})();
					},
					execute: async (appId, event, payload, requestId) => {
						markHandled(key);
						void engine
							.executeEvent(`native-${requestId}`, {
								appId,
								eventId: event.id,
								payload: { id: event.node_id, payload },
								title: event.name,
								path: `/use?id=${encodeURIComponent(appId)}&eventId=${encodeURIComponent(event.id)}`,
								interfaceType: "generic",
							})
							.catch(reportError);
					},
					mcp: (appId, event, tool, id, input) => {
						if (current())
							setMcpRequests((requests) => [
								...requests.filter((request) => request.scope === scope),
								{
									id,
									scope,
									appId,
									eventId: event.id,
									title: event.name,
									tool,
									input,
								},
							]);
					},
				});
				if (!current()) return false;
				markHandled(key);
				await acknowledge();
				return true;
			} catch (error) {
				if (!current()) return false;
				if (!request.backgroundError && isNativeDeliveryRetryable(error))
					return false;
				reportError(error);
				await completeNativeAction(request, nativeActionErrorOutcome(error), {
					getCurrentScope: () => currentScope.current,
					invoke,
				});
				markHandled(key);
				await acknowledge();
				return true;
			} finally {
				handling.delete(key);
			}
		};
		const drain = async () => {
			if (!current() || document.visibilityState !== "visible") return;
			drainRequested = true;
			if (draining) return;
			draining = true;
			try {
				do {
					drainRequested = false;
					// Finish account invalidation before consuming its successor's queue.
					await writes.current;
					if (!current() || document.visibilityState !== "visible") return;
					const actions = await invoke<NativePendingAction[]>(
						"native_pending_actions",
					);
					for (const request of actions) await handle(request);
				} while (
					current() &&
					drainRequested &&
					document.visibilityState === "visible"
				);
			} catch (error) {
				console.warn("Native action delivery:", error);
			} finally {
				draining = false;
			}
		};
		const onFocus = () => {
			// Native entry points stay usable while hub/widget refreshes are slow or offline.
			void drain();
			void refresh(FOCUS_REFRESH_MS);
		};
		const onRecentApps = () => {
			void refresh();
		};
		const onBotInput = (event: Event) => {
			const value = (event as CustomEvent).detail;
			if (
				!value ||
				typeof value.botId !== "string" ||
				![
					"voice",
					"document",
					"photo",
					"summarize",
					"extract",
					"private_clipboard",
				].includes(value.operation)
			)
				return;
			void handle({
				id: crypto.randomUUID(),
				scope,
				action: {
					kind:
						value.operation === "private_clipboard"
							? "private_bot_clipboard"
							: value.operation === "voice"
								? "capture_bot_voice"
								: ["summarize", "extract"].includes(value.operation)
									? "local_bot_summary"
									: "capture_bot_document",
					botId: value.botId,
					operation: value.operation,
				},
			});
		};
		const onTick = () => {
			void drain();
			void refresh(PERIODIC_REFRESH_MS);
		};
		let statusLoading = false;
		const refreshStatus = async () => {
			if (
				!current() ||
				statusLoading ||
				document.visibilityState !== "visible" ||
				!backend.botState?.getMobileStatus
			)
				return;
			statusLoading = true;
			const revision = ++botStatusRevision;
			try {
				const turns = await backend.botState.getMobileStatus();
				if (!current()) return;
				acceptBotStatus(turns, revision);
				await publish();
			} catch {
				/* Keep the last observed status until the next successful sync. */
			} finally {
				statusLoading = false;
			}
		};
		const unlisten = listen<{ url?: string; replayed?: boolean }>(
			"native-action",
			(event) => {
				const url = event.payload?.url;
				if (url) {
					const replayKey = `flow-like:native-link:${scope}:${url}`;
					if (!event.payload.replayed || !wasHandled(replayKey)) {
						const request = nativeNavigationRequest(url, scope);
						if (request) {
							void handle(request).then((accepted) => {
								if (accepted) markHandled(replayKey);
							});
						}
					}
				}
				onFocus();
			},
		);
		void unlisten
			.then(() => invoke("deeplink_replay_pending"))
			.catch(console.warn);
		const unsubscribeRuns = engine.subscribeToGlobalUpdates(() => {
			if (
				current() &&
				JSON.stringify(engine.getActiveExecutions(scope)) !== activeSignature
			)
				void publish();
		});
		const turnSubscription = backend.botState?.turns?.subscribe((turn) => {
			if (!current()) return;
			botTurns.set(turn.id, turn);
			botTurnRevisions.set(turn.id, ++botStatusRevision);
			void publish();
		});
		void turnSubscription?.catch((error) =>
			console.warn("Native bot status:", error),
		);
		const initialStatusRevision = ++botStatusRevision;
		void backend.botState?.turns
			?.list()
			.then((turns) => {
				if (!current() || lastBotStatusRead > initialStatusRevision) return;
				for (const turn of turns) {
					if (botTurns.has(turn.id)) continue;
					botTurns.set(turn.id, turn);
					botTurnRevisions.set(turn.id, initialStatusRevision);
				}
				void publish();
			})
			.catch(() => {});
		let botRefreshTimer: ReturnType<typeof setTimeout> | undefined;
		const unsubscribeQueries = queryClient
			.getQueryCache()
			.subscribe((event) => {
				if (
					event.type !== "updated" ||
					!["bots", "inbox"].includes(String(event.query.queryKey[0]))
				)
					return;
				if (botRefreshTimer) return;
				botRefreshTimer = setTimeout(() => {
					botRefreshTimer = undefined;
					void refresh();
				}, 1000);
			});
		window.addEventListener("native-bot-input", onBotInput);
		window.addEventListener("flow-like-native-action", onFocus);
		window.addEventListener("focus", onFocus);
		window.addEventListener(RECENT_APPS_CHANGED, onRecentApps);
		document.addEventListener("visibilitychange", onFocus);
		const timer = window.setInterval(onTick, 60_000);
		const statusTimer = window.setInterval(() => {
			void refreshStatus();
		}, 15_000);
		void refreshStatus();
		onFocus();
		return () => {
			stopped = true;
			clearInterval(automationTimer);
			window.removeEventListener(
				"flow-like-native-action-cancel",
				onAutomationCancel,
			);
			if (currentScope.current !== scope)
				for (const request of automationRequests.current.values())
					if (request.scope === scope) cancelNativeAction(request);
			offLifecycle();
			icons.dispose();
			notificationIcons.dispose();
			customWidgetPublisher.dispose();
			if (publishCurrentPage.current === publish)
				publishCurrentPage.current = undefined;
			window.clearInterval(timer);
			window.clearInterval(statusTimer);
			window.removeEventListener("focus", onFocus);
			window.removeEventListener(RECENT_APPS_CHANGED, onRecentApps);
			document.removeEventListener("visibilitychange", onFocus);
			unsubscribeRuns();
			unsubscribeQueries();
			if (botRefreshTimer) clearTimeout(botRefreshTimer);
			window.removeEventListener("native-bot-input", onBotInput);
			window.removeEventListener("flow-like-native-action", onFocus);
			void turnSubscription?.then((off) => off()).catch(() => {});
			void unlisten.then((off) => off());
		};
	}, [
		identityReady,
		auth.isAuthenticated,
		auth.user?.profile.sub,
		scope,
		backend,
		engine,
		queryClient,
		webOrigin.refetch,
	]);
	if (identityReady && botInput?.scope === scope)
		return (
			<NativeBotInputDialog
				key={botInput.id}
				input={botInput}
				isCurrent={() => currentScope.current === botInput.scope}
				onClose={() => setBotInput(undefined)}
				onSend={async (result) => {
					const files = [
						...(await readNativeSharedFiles(result.files ?? [])),
						...(result.attachments ?? []),
					];
					if (currentScope.current !== botInput.scope) return;
					router.push(nativeBotUrl({ botId: botInput.botId }));
					useGlobalChatStore.getState().setDraft({
						prompt: result.text || "",
						files,
						botId: botInput.botId,
						nativeScope: botInput.scope,
					});
				}}
			/>
		);
	const request = identityReady
		? mcpRequests.find((request) => request.scope === scope)
		: undefined;
	const run = identityReady
		? eventRuns.find((run) => run.scope === scope)
		: undefined;
	if (run)
		return (
			<NativeEventResultDialog
				key={run.id}
				run={run}
				isCurrent={() => currentScope.current === run.scope}
				onClose={() =>
					setEventRuns((runs) =>
						runs.filter((item) => item.id !== run.id && item.scope === scope),
					)
				}
				onOpen={() => {
					router.push(
						`/use?id=${encodeURIComponent(run.appId)}&eventId=${encodeURIComponent(run.eventId)}`,
					);
					setEventRuns((runs) =>
						runs.filter((item) => item.id !== run.id && item.scope === scope),
					);
				}}
			/>
		);
	return request ? (
		<NativeMcpRunDialog
			key={request.id}
			request={request}
			isCurrent={() => currentScope.current === request.scope}
			onClose={() =>
				setMcpRequests((requests) =>
					requests.filter(
						(item) => item.id !== request.id && item.scope === scope,
					),
				)
			}
		/>
	) : null;
}
