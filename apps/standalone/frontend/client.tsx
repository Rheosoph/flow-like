"use client";

import { ChatInterface } from "@flow-like/flow-like-ui/components/interfaces/chat-default";
import { ChatFeedbackEnabledContext } from "@flow-like/flow-like-ui/components/interfaces/chat-default/message";
import { PageInterface } from "@flow-like/flow-like-ui/components/interfaces/page-interface";
import { ThemeProvider } from "@flow-like/flow-like-ui/components/theme-provider";
import { Toaster } from "@flow-like/flow-like-ui/components/ui/sonner";
import { TooltipProvider } from "@flow-like/flow-like-ui/components/ui/tooltip";
import { QueryParamNavigationContext } from "@flow-like/flow-like-ui/lib/set-query-params";
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
} from "./lib/backend";
import { type ServiceRequest, createServiceRequest } from "./lib/transport";

interface Session {
	request: ServiceRequest;
	inventory: Inventory;
	controller: AbortController;
}

export default function Service() {
	const [session, setSession] = useState<Session>();
	const [input, setInput] = useState("");
	const [error, setError] = useState("");
	const [busy, setBusy] = useState(false);
	useEffect(() => () => session?.controller.abort(), [session]);
	const lock = useCallback(() => {
		session?.controller.abort();
		setSession(undefined);
		setInput("");
		setError("");
	}, [session]);
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
						this tab until you lock it or close it.
					</p>
					<label className="block space-y-2">
						<span className="text-sm font-medium">Service access token</span>
						<input
							aria-label="Service access token"
							type="password"
							autoComplete="off"
							value={input}
							onChange={(event) => setInput(event.target.value)}
							disabled={busy}
							className="w-full rounded-md border bg-background px-3 py-2"
						/>
					</label>
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					<button
						disabled={busy || !input}
						className="w-full rounded-md bg-primary px-4 py-2 font-medium text-primary-foreground disabled:opacity-50"
						type="submit"
					>
						{busy ? "Opening…" : "Open service"}
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
	const events = session.inventory.events.filter(
		(event) => event.default_page_id || event.event_type === "simple_chat",
	);
	const [selected, setSelected] = useState(
		() => events.find((event) => event.is_default)?.id ?? events[0]?.id,
	);
	const [ready, setReady] = useState(false);
	const [page, setPage] = useState<PageBootstrap>();
	const [error, setError] = useState("");
	const event = events.find((event) => event.id === selected);
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
	const navigate = useCallback(
		(route: string) => {
			const next = events.find((event) => event.route === route);
			if (!next) {
				setError("That route is not deployed on this service.");
				return;
			}
			setSelected(next.id);
		},
		[events],
	);
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
			<QueryParamNavigationContext.Provider value={updateQuery}>
				<main className="flex h-dvh min-h-0 flex-col bg-background text-foreground">
					<header className="flex min-h-14 shrink-0 items-center gap-3 border-b px-4">
						<span className="font-semibold">Flow-Like</span>
						<label className="flex min-w-0 flex-1 items-center gap-2">
							<span className="sr-only">Deployed interface</span>
							<select
								aria-label="Deployed interface"
								value={selected ?? ""}
								onChange={(event) => setSelected(event.target.value)}
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
							This deployment has no Page or chat interface. Its API endpoints
							are available to clients.
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
												onNavigate={navigate}
											/>
										) : (
											<p className="p-6">Loading Page…</p>
										)
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
		</QueryClientProvider>
	);
}
