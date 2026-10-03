"use client";

import { ChatInterface } from "@flow-like/flow-like-ui/components/interfaces/chat-default";
import { ChatFeedbackEnabledContext } from "@flow-like/flow-like-ui/components/interfaces/chat-default/message";
import { GenericEventFormInterface } from "@flow-like/flow-like-ui/components/interfaces/generic-event-form";
import { PageInterface } from "@flow-like/flow-like-ui/components/interfaces/page-interface";
import { ThemeProvider } from "@flow-like/flow-like-ui/components/theme-provider";
import { Toaster } from "@flow-like/flow-like-ui/components/ui/sonner";
import { TooltipProvider } from "@flow-like/flow-like-ui/components/ui/tooltip";
import {
	type ClientNavigation,
	ClientNavigationContext,
} from "@flow-like/flow-like-ui/lib/client-navigation";
import type { IEvent } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import { QueryParamNavigationContext } from "@flow-like/flow-like-ui/lib/set-query-params";
import { parseUint8ArrayToJson } from "@flow-like/flow-like-ui/lib/uint8";
import { useBackendStore } from "@flow-like/flow-like-ui/state/backend-state";
import { ExecutionEngineProviderComponent } from "@flow-like/flow-like-ui/state/execution-engine-context";
import { I18nProvider } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { AuthProvider } from "react-oidc-context";
import {
	type Inventory,
	type PageBootstrap,
	createServiceBackend,
	isPersonStarted,
} from "./lib/backend";
import { clearServiceHistory } from "./lib/history";
import {
	SERVICE_BASE,
	appRouteHref,
	initialServiceEvent,
	resolveServiceNavigation,
	serviceEventHref,
} from "./lib/navigation";
import { type ServiceRequest, createServiceRequest } from "./lib/transport";

interface Session {
	request: ServiceRequest;
	inventory: Inventory;
	controller: AbortController;
}

const servesInterface = (event: IEvent) =>
	Boolean(event.default_page_id) || event.event_type === "simple_chat";

/** What a form shows besides its fields: the routes it links to, as the service lists them. */
function formConfig(event: IEvent) {
	const config = parseUint8ArrayToJson(event.config);
	const routes = config?.navigate_to_routes;
	return Array.isArray(routes) ? { navigate_to_routes: routes } : {};
}

export default function Service() {
	const [session, setSession] = useState<Session>();
	const [input, setInput] = useState("");
	const [error, setError] = useState("");
	const [busy, setBusy] = useState(false);
	const [clearing, setClearing] = useState(false);
	useEffect(() => () => session?.controller.abort(), [session]);
	const lock = useCallback(() => {
		session?.controller.abort();
		setSession(undefined);
		setInput("");
		setError("");
		window.history.replaceState(null, "", SERVICE_BASE);
		setClearing(true);
	}, [session]);
	// Runs after the interface unmounted, so no late write can restore stored data. The reload
	// also drops the Page state the shared UI keeps in memory.
	useEffect(() => {
		if (!clearing) return;
		clearServiceHistory()
			.then(() => window.location.replace(SERVICE_BASE))
			.catch(() => {
				setError(
					"Earlier conversations and Page data could not be removed from this browser. Clear this site's data and close this tab before someone else uses it.",
				);
				setClearing(false);
			});
	}, [clearing]);
	if (!session)
		return (
			<main className="grid min-h-dvh place-items-center bg-background p-6 text-foreground">
				<form
					className="w-full max-w-sm space-y-5 rounded-xl border bg-card p-7 shadow-sm"
					onSubmit={async (event) => {
						event.preventDefault();
						setBusy(true);
						setError("");
						const controller = new AbortController();
						try {
							const request = createServiceRequest(input);
							const inventory = (await (
								await request("/services", { signal: controller.signal })
							).json()) as Inventory;
							if (
								typeof inventory.project_id !== "string" ||
								!Array.isArray(inventory.events)
							)
								throw new Error("The service returned an invalid inventory.");
							setInput("");
							setSession({ request, inventory, controller });
						} catch (cause) {
							controller.abort();
							setError(
								cause instanceof Error
									? cause.message
									: "The service could not be opened.",
							);
						} finally {
							setBusy(false);
						}
					}}
				>
					<div>
						<p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">
							Flow-Like
						</p>
						<h1 className="mt-2 text-2xl font-semibold">Open this service</h1>
					</div>
					<p className="text-sm text-muted-foreground">
						Enter the access token supplied by the device owner. It stays in
						this tab until you lock it or close it. Conversations and Page data
						stay in this browser until you select Lock.
					</p>
					<label className="block space-y-2">
						<span className="text-sm font-medium">Service access token</span>
						<input
							aria-label="Service access token"
							type="password"
							autoComplete="off"
							value={input}
							onChange={(event) => setInput(event.target.value)}
							disabled={busy || clearing}
							className="w-full rounded-md border bg-background px-3 py-2"
						/>
					</label>
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					<button
						disabled={busy || clearing || !input}
						className="w-full rounded-md bg-primary px-4 py-2 font-medium text-primary-foreground disabled:opacity-50"
						type="submit"
					>
						{clearing ? "Locking…" : busy ? "Opening…" : "Open service"}
					</button>
				</form>
			</main>
		);
	return (
		<AuthProvider
			authority={window.location.origin}
			client_id="standalone-service-interface"
			redirect_uri={`${window.location.origin}/ui/`}
			automaticSilentRenew={false}
			skipSigninCallback
		>
			<I18nProvider>
				<ThemeProvider attribute="class" defaultTheme="system" enableSystem>
					<TooltipProvider>
						<SessionView session={session} lock={lock} />
						<Toaster />
					</TooltipProvider>
				</ThemeProvider>
			</I18nProvider>
		</AuthProvider>
	);
}

function SessionView({
	session,
	lock,
}: { session: Session; lock: () => void }) {
	const [client] = useState(
		() =>
			new QueryClient({
				defaultOptions: {
					queries: { retry: false, refetchOnWindowFocus: false, gcTime: 0 },
				},
			}),
	);
	const { backend, bootstrap } = useMemo(
		() =>
			createServiceBackend(
				session.inventory,
				session.request,
				session.controller.signal,
			),
		[session],
	);
	const events = useMemo(
		() => [
			...session.inventory.events.filter(servesInterface),
			...session.inventory.events.filter(isPersonStarted),
		],
		[session.inventory.events],
	);
	const [selected, setSelected] = useState(
		() => initialServiceEvent(window.location.search, events)?.id,
	);
	const [ready, setReady] = useState(false);
	const [page, setPage] = useState<PageBootstrap>();
	const [error, setError] = useState("");
	const event = events.find((event) => event.id === selected);
	const eventConfig = useMemo(() => (event ? formConfig(event) : {}), [event]);
	useEffect(() => {
		const previous = useBackendStore.getState().backend;
		useBackendStore.getState().setBackend(backend);
		setReady(true);
		return () => {
			client.clear();
			if (useBackendStore.getState().backend === backend)
				useBackendStore.setState({ backend: previous });
		};
	}, [backend, client]);
	useEffect(() => {
		let current = true;
		setPage(undefined);
		setError("");
		if (event?.default_page_id)
			bootstrap(session.inventory.project_id, event.id)
				.then((page) => {
					if (current) setPage(page);
				})
				.catch((cause) => {
					if (current)
						setError(
							cause instanceof Error
								? cause.message
								: "The Page could not be opened.",
						);
				});
		return () => {
			current = false;
		};
	}, [
		event?.id,
		event?.default_page_id,
		bootstrap,
		session.inventory.project_id,
	]);
	const navigation = useMemo<ClientNavigation>(() => {
		const resolve = (href: string) =>
			resolveServiceNavigation(
				href,
				session.inventory.project_id,
				events,
				new URL(window.location.href),
			);
		return {
			href: (href) => {
				const target = resolve(href);
				return target.kind === "event" || target.kind === "query"
					? target.href
					: href;
			},
			navigate: (href, replace) => {
				const target = resolve(href);
				if (target.kind === "unsupported") {
					setError("That route is not deployed on this service.");
					return;
				}
				if (target.kind === "external") {
					window.open(target.href, "_blank", "noopener,noreferrer");
					return;
				}
				if (target.kind === "event") {
					setError("");
					setSelected(target.eventId);
				}
				window.history[replace ? "replaceState" : "pushState"](
					null,
					"",
					target.href,
				);
			},
		};
	}, [events, session.inventory.project_id]);
	const navigate = useCallback(
		(route: string, replace: boolean, queryParams?: Record<string, string>) =>
			navigation.navigate(appRouteHref(route, queryParams), replace),
		[navigation],
	);
	const choose = useCallback(
		(id: string) => {
			const target = events.find((event) => event.id === id);
			if (!target) return;
			setError("");
			setSelected(id);
			window.history.pushState(null, "", serviceEventHref(target));
		},
		[events],
	);
	useEffect(() => {
		const restore = () => {
			setError("");
			setSelected(initialServiceEvent(window.location.search, events)?.id);
		};
		window.addEventListener("popstate", restore);
		return () => window.removeEventListener("popstate", restore);
	}, [events]);
	const updateQuery = useCallback((href: string, replace: boolean) => {
		const target = new URL(href, window.location.origin);
		if (target.origin !== window.location.origin) return;
		window.history[replace ? "replaceState" : "pushState"](
			null,
			"",
			`/ui/${target.search}`,
		);
	}, []);
	return (
		<QueryClientProvider client={client}>
			<ClientNavigationContext.Provider value={navigation}>
				<QueryParamNavigationContext.Provider value={updateQuery}>
					<main className="flex h-dvh min-h-0 flex-col bg-background text-foreground">
						<header className="flex min-h-14 shrink-0 items-center gap-3 border-b px-4">
							<span className="font-semibold">Flow-Like</span>
							<label className="flex min-w-0 flex-1 items-center gap-2">
								<span className="sr-only">Deployed interface</span>
								<select
									aria-label="Deployed interface"
									value={selected ?? ""}
									onChange={(event) => choose(event.target.value)}
									className="max-w-sm rounded-md border bg-background px-3 py-1.5"
								>
									{events.map((event) => (
										<option value={event.id} key={event.id}>
											{event.name}
										</option>
									))}
								</select>
							</label>
							<button
								type="button"
								onClick={lock}
								className="rounded-md border px-3 py-1.5 text-sm"
							>
								Lock
							</button>
						</header>
						{error && (
							<p role="alert" className="border-b p-4 text-destructive">
								{error}
							</p>
						)}
						{!event ? (
							<p className="p-6 text-muted-foreground">
								This deployment has no Page, chat, form or quick action. Its API
								endpoints are available to clients.
							</p>
						) : !ready ? (
							<p className="p-6">Loading…</p>
						) : (
							<Suspense fallback={<p className="p-6">Loading interface…</p>}>
								<ExecutionEngineProviderComponent>
									<div
										className="flex min-h-0 flex-1 flex-col overflow-hidden"
										key={event.id}
									>
										{event.default_page_id ? (
											page ? (
												<PageInterface
													appId={session.inventory.project_id}
													event={page.event}
													page={{ ...page.page, noCache: true }}
													pageRevision={page.execution_revision}
													pageExecutionRevision={page.execution_revision}
													pageElementDemand={page.element_demand}
													route={page.route}
													config={{}}
												/>
											) : (
												<p className="p-6">Loading Page…</p>
											)
										) : isPersonStarted(event) ? (
											<GenericEventFormInterface
												appId={session.inventory.project_id}
												event={event}
												config={eventConfig}
												onNavigate={navigate}
											/>
										) : (
											<ChatFeedbackEnabledContext.Provider value={false}>
												<ChatInterface
													appId={session.inventory.project_id}
													event={event}
													config={{}}
													onNavigate={navigate}
												/>
											</ChatFeedbackEnabledContext.Provider>
										)}
									</div>
								</ExecutionEngineProviderComponent>
							</Suspense>
						)}
					</main>
				</QueryParamNavigationContext.Provider>
			</ClientNavigationContext.Provider>
		</QueryClientProvider>
	);
}
