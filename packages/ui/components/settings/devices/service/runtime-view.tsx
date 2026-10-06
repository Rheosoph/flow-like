"use client";

import { useTranslation } from "@flow-like/locales";
import { QueryClientProvider } from "@tanstack/react-query";
import {
	Suspense,
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { AssetSourceResolverContext } from "../../../../hooks/use-asset-source";
import { readAppQuery } from "../../../../lib/app-route-url";
import {
	type ClientNavigation,
	ClientNavigationContext,
} from "../../../../lib/client-navigation";
import {
	type PageBootstrap,
	isPersonStarted,
} from "../../../../lib/service-runtime/backend";
import {
	appRouteHref,
	initialServiceEvent,
	resolveServiceNavigation,
	serviceEventHref,
} from "../../../../lib/service-runtime/navigation";
import type { RuntimeSession } from "../../../../lib/service-runtime/session";
import { setRuntimeNavigation } from "../../../../lib/service-runtime/session-scope";
import { QueryParamScopeContext } from "../../../../lib/set-query-params";
import { parseUint8ArrayToJson } from "../../../../lib/uint8";
import { BackendContext } from "../../../../state/backend-state";
import { ExecutionEngineProviderComponent } from "../../../../state/execution-engine-context";
import { ExecutionServiceContext } from "../../../../state/execution-service-context-value";
import { ChatInterface } from "../../../interfaces/chat-default";
import { ChatFeedbackEnabledContext } from "../../../interfaces/chat-default/message";
import { FormWorkbenchInterface } from "../../../interfaces/form-workbench";
import { PageInterface } from "../../../interfaces/page-interface";
import { DvSelect } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";

export function RuntimeView({ session }: { session: RuntimeSession }) {
	const { t } = useTranslation("devices");
	const { appId, inventory, backend, bootstrap, execution, client } = session;
	const events = useMemo(
		() =>
			inventory.events.filter(
				(event) =>
					event.default_page_id ||
					event.event_type === "simple_chat" ||
					isPersonStarted(event),
			),
		[inventory],
	);
	const initial = initialServiceEvent("", events);
	const [selected, setSelected] = useState(initial?.id);
	const [href, setHref] = useState(
		initial ? serviceEventHref(initial) : "/ui/",
	);
	const currentHref = useRef(href);
	currentHref.current = href;
	const [page, setPage] = useState<PageBootstrap>();
	const [error, setError] = useState("");
	const event = events.find((event) => event.id === selected);
	const search = new URL(href, "https://service.invalid").search;
	const queryParams = useMemo(
		() => Object.fromEntries(new URLSearchParams(search)),
		[search],
	);
	useLayoutEffect(() => {
		setRuntimeNavigation(appId, {
			route: event?.route ?? "/",
			queryParams: Object.fromEntries(readAppQuery(search)),
		});
	}, [appId, event?.route, search]);
	const setQuery = useCallback((key: string, value: string | undefined) => {
		setHref((href) => {
			const url = new URL(href, "https://service.invalid");
			if (value === undefined) url.searchParams.delete(key);
			else url.searchParams.set(key, value);
			return `${url.pathname}${url.search}${url.hash}`;
		});
	}, []);
	const queryScope = useMemo(
		() => ({ search, set: setQuery }),
		[search, setQuery],
	);
	useEffect(() => {
		let current = true;
		setPage(undefined);
		setError("");
		if (event?.default_page_id)
			void bootstrap(appId, event.id).then(
				(page) => {
					if (current && !session.signal.aborted) setPage(page);
				},
				(cause) => {
					if (current && !session.signal.aborted)
						setError(cause instanceof Error ? cause.message : String(cause));
				},
			);
		return () => {
			current = false;
		};
	}, [appId, bootstrap, event?.id, event?.default_page_id, session.signal]);
	const navigation = useMemo<ClientNavigation>(() => {
		const resolve = (href: string) => {
			// Renderers address their isolated app identity; service links use the deployed ID.
			let url: URL;
			try {
				url = new URL(
					href,
					new URL(currentHref.current, "https://service.invalid"),
				);
			} catch {
				return { kind: "unsupported" } as const;
			}
			if (url.searchParams.get("id") === appId)
				url.searchParams.set("id", inventory.project_id);
			return resolveServiceNavigation(
				url.href,
				inventory.project_id,
				events,
				new URL(currentHref.current, "https://service.invalid"),
			);
		};
		return {
			href: (href) => {
				const target = resolve(href);
				return target.kind === "unsupported"
					? "/ui/?route=__unavailable"
					: target.href;
			},
			navigate: (href) => {
				const target = resolve(href);
				if (target.kind === "unsupported") {
					setError(
						t(
							"serviceConfig.runtime.routeUnavailable",
							"That route is not deployed on this service.",
						),
					);
					return;
				}
				if (target.kind === "external") {
					window.open(target.href, "_blank", "noopener,noreferrer");
					return;
				}
				if (target.kind === "event") setSelected(target.eventId);
				setError("");
				setHref(target.href);
			},
		};
	}, [appId, inventory.project_id, events, t]);
	const navigate = useCallback(
		(route: string, replace: boolean, params?: Record<string, string>) =>
			navigation.navigate(appRouteHref(route, params), replace),
		[navigation],
	);
	const config = useMemo(() => {
		const routes =
			event && parseUint8ArrayToJson(event.config)?.navigate_to_routes;
		return Array.isArray(routes) ? { navigate_to_routes: routes } : {};
	}, [event]);
	return (
		<BackendContext.Provider value={backend}>
			<AssetSourceResolverContext.Provider value={session.resolveResource}>
				<QueryClientProvider client={client}>
					<ExecutionServiceContext.Provider value={execution}>
						<ClientNavigationContext.Provider value={navigation}>
							<QueryParamScopeContext.Provider value={queryScope}>
								<div className="flex min-h-0 flex-1 flex-col">
									<div className="border-b px-4 py-3">
										<DvSelect
											aria-label={t(
												"serviceConfig.runtime.interface",
												"Deployed interface",
											)}
											value={selected ?? ""}
											options={events.map((event) => ({
												value: event.id,
												label: event.name,
											}))}
											onValueChange={(id) => {
												const event = events.find((event) => event.id === id);
												if (event) {
													setSelected(id);
													setHref(serviceEventHref(event));
												}
											}}
										/>
									</div>
									{error && (
										<InlineResult tone="critical">{error}</InlineResult>
									)}
									{!event ? (
										<p className="p-6 text-muted-foreground">
											{t(
												"serviceConfig.runtime.empty",
												"This deployment has no Page, chat, form or quick action. Use its API endpoints to call it.",
											)}
										</p>
									) : (
										<Suspense
											fallback={
												<p className="p-6">
													{t(
														"serviceConfig.runtime.loading",
														"Loading interface…",
													)}
												</p>
											}
										>
											<ExecutionEngineProviderComponent
												executionScope={appId}
												showRunningTasks={false}
											>
												<div
													key={event.id}
													className="flex min-h-0 flex-1 flex-col overflow-hidden"
												>
													{event.default_page_id ? (
														page && page.event_id === event.id ? (
															<PageInterface
																appId={appId}
																event={page.event}
																page={{ ...page.page, noCache: true }}
																pageRevision={page.execution_revision}
																pageExecutionRevision={page.execution_revision}
																pageElementDemand={page.element_demand}
																route={page.route}
																config={{}}
																queryParams={queryParams}
																onNavigationMessage={(message) => {
																	if (message.type === "navigateTo")
																		navigate(
																			message.route,
																			message.replace ?? false,
																			message.queryParams,
																		);
																	else
																		setQuery(
																			message.key,
																			message.value ?? undefined,
																		);
																}}
															/>
														) : (
															<p className="p-6">
																{t(
																	"serviceConfig.runtime.loading",
																	"Loading interface…",
																)}
															</p>
														)
													) : isPersonStarted(event) ? (
														<FormWorkbenchInterface
															appId={appId}
															event={event}
															config={config}
															onNavigate={navigate}
														/>
													) : (
														<ChatFeedbackEnabledContext.Provider value={false}>
															<ChatInterface
																appId={appId}
																event={event}
																config={{ attach_widget_snapshots: false }}
																onNavigate={navigate}
															/>
														</ChatFeedbackEnabledContext.Provider>
													)}
												</div>
											</ExecutionEngineProviderComponent>
										</Suspense>
									)}
								</div>
							</QueryParamScopeContext.Provider>
						</ClientNavigationContext.Provider>
					</ExecutionServiceContext.Provider>
				</QueryClientProvider>
			</AssetSourceResolverContext.Provider>
		</BackendContext.Provider>
	);
}
