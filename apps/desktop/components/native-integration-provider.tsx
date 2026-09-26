"use client";

import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";

import { useInvoke } from "@flow-like/flow-like-ui/hooks/use-invoke";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import {
	completeNativeAction,
	nativeActionErrorOutcome,
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
import { useExecutionEngine } from "@flow-like/flow-like-ui/state/execution-engine-context";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import { useQuery } from "@tanstack/react-query";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { usePathname, useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { createNativeAppIconPublisher } from "../lib/native-app-icons";
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
	const writes = useRef(Promise.resolve());
	const previousScope = useRef<string | undefined>(undefined);
	const [mcpRequests, setMcpRequests] = useState<NativeMcpRequest[]>([]);
	const [eventRuns, setEventRuns] = useState<NativeEventRunView[]>([]);
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
			/Mac|iPhone|iPad|iPod/.test(navigator.userAgent),
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
		if (!isTauri() || !/Mac|iPhone|iPad|iPod/.test(navigator.userAgent)) return;
		let stopped = false;
		let refreshPromise: Promise<void> | undefined;
		let draining = false;
		let drainRequested = false;
		let baseSnapshot: NativeSnapshot | undefined;
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
			previousScope.current = scope;
		}
		previousScope.current = scope;
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
				withNativeActiveRuns({ ...baseSnapshot, customWidgets }, runs),
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
				await dispatchNativeAction(request, {
					scope,
					backend,
					navigate: (url) => navigation.current.router.push(url),
					isCurrent: current,
					flowpilot: async (prompt, voice, paths) => {
						// Opening FlowPilot must not wait for files to cross the native bridge.
						navigation.current.router.push(voice ? "/chat?voice=1" : "/chat");
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
						if (!current()) return;
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
						if (!current()) return;
						markHandled(key);
						const isCurrent = () => currentScope.current === scope;
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
		const onTick = () => {
			void drain();
			void refresh(PERIODIC_REFRESH_MS);
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
		window.addEventListener("focus", onFocus);
		window.addEventListener(RECENT_APPS_CHANGED, onRecentApps);
		document.addEventListener("visibilitychange", onFocus);
		const timer = window.setInterval(onTick, 60_000);
		onFocus();
		return () => {
			stopped = true;
			icons.dispose();
			notificationIcons.dispose();
			customWidgetPublisher.dispose();
			if (publishCurrentPage.current === publish)
				publishCurrentPage.current = undefined;
			window.clearInterval(timer);
			window.removeEventListener("focus", onFocus);
			window.removeEventListener(RECENT_APPS_CHANGED, onRecentApps);
			document.removeEventListener("visibilitychange", onFocus);
			unsubscribeRuns();
			void unlisten.then((off) => off());
		};
	}, [
		identityReady,
		auth.isAuthenticated,
		auth.user?.profile.sub,
		scope,
		backend,
		engine,
	]);
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
