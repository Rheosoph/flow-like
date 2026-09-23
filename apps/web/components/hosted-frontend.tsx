"use client";

import { ChatInterface } from "@flow-like/flow-like-ui/components/interfaces/chat-default";
import { ChatFeedbackEnabledContext } from "@flow-like/flow-like-ui/components/interfaces/chat-default/message";
import { Container } from "@flow-like/flow-like-ui/components/interfaces/container";
import { GenericEventFormInterface } from "@flow-like/flow-like-ui/components/interfaces/generic-event-form";
import type {
	ISidebarActions,
	IToolBarActions,
} from "@flow-like/flow-like-ui/components/interfaces/interfaces";
import { PageInterface } from "@flow-like/flow-like-ui/components/interfaces/page-interface";
import { ScopedCustomCss } from "@flow-like/flow-like-ui/components/scoped-custom-css";
import { ThemeProvider } from "@flow-like/flow-like-ui/components/theme-provider";
import { Toaster } from "@flow-like/flow-like-ui/components/ui/sonner";
import { TooltipProvider } from "@flow-like/flow-like-ui/components/ui/tooltip";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { QueryParamNavigationContext } from "@flow-like/flow-like-ui/lib/set-query-params";
import { parseUint8ArrayToJson } from "@flow-like/flow-like-ui/lib/uint8";
import { useBackendStore } from "@flow-like/flow-like-ui/state/backend-state";
import { ExecutionEngineProviderComponent } from "@flow-like/flow-like-ui/state/execution-engine-context";
import { I18nProvider } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import type { UserManagerSettings } from "oidc-client-ts";
import {
	type ReactElement,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { AuthProvider, useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	type HostedBootstrap,
	HostedHttpError,
	type HostedRoute,
	createHostedBackend,
	createHostedRequest,
} from "../lib/hosted-backend";
import {
	type HostedTarget,
	hostedNavigationPath,
	isHostedQueryPath,
	parseHostedTarget,
} from "../lib/hosted-route";

import { getWebOidcSettings } from "../lib/oidc-settings";
import { saveReturnUrl } from "../lib/return-url";

function Status({
	error,
	children,
}: { error?: boolean; children: React.ReactNode }) {
	return (
		<main className="grid min-h-dvh place-items-center p-8">
			<div className="max-w-md space-y-4" role={error ? "alert" : "status"}>
				<p>{children}</p>
				{error && (
					<button
						type="button"
						className="rounded-md border px-4 py-2"
						onClick={() => window.location.reload()}
					>
						Try again
					</button>
				)}
			</div>
		</main>
	);
}

export function HostedFrontend() {
	const [settings, setSettings] = useState<UserManagerSettings>();
	const [publicBootstrap, setPublicBootstrap] = useState<HostedBootstrap>();
	const [error, setError] = useState<string>();
	useEffect(() => {
		const controller = new AbortController();
		(async () => {
			try {
				const target = parseHostedTarget(new URL(window.location.href));
				const anonymousSettings: UserManagerSettings = {
					authority: window.location.origin,
					client_id: "flow-like-hosted-anonymous",
					redirect_uri: new URL("/callback", window.location.origin).href,
					automaticSilentRenew: false,
				};
				if (!target) {
					setSettings(anonymousSettings);
					return;
				}
				try {
					const response = await createHostedRequest(target)("", {
						signal: controller.signal,
					});
					setPublicBootstrap((await response.json()) as HostedBootstrap);
					// getUser only reads browser storage. Anonymous interfaces never request OIDC metadata.
					setSettings(anonymousSettings);
					return;
				} catch (cause) {
					if (!(cause instanceof HostedHttpError) || cause.status !== 401)
						throw cause;
				}
				const response = await fetch(getApiUrl(undefined, "auth/openid"), {
					signal: controller.signal,
					credentials: "omit",
				});
				if (!response.ok)
					throw new Error("Sign-in configuration could not be loaded.");
				setSettings(
					getWebOidcSettings((await response.json()) as UserManagerSettings),
				);
			} catch (cause) {
				if (!controller.signal.aborted)
					setError(
						cause instanceof Error
							? cause.message
							: "The interface could not be loaded.",
					);
			}
		})();
		return () => controller.abort();
	}, []);
	if (error) return <Status error>{error}</Status>;
	if (!settings) return <Status>Loading…</Status>;
	return (
		<AuthProvider {...settings} skipSigninCallback>
			<HostedSession initialData={publicBootstrap} />
		</AuthProvider>
	);
}

function HostedSession({ initialData }: { initialData?: HostedBootstrap }) {
	const auth = useAuth();
	const searchParams = useSearchParams();
	const [target] = useState<HostedTarget | null>(() =>
		typeof window === "undefined"
			? null
			: parseHostedTarget(new URL(window.location.href)),
	);
	const [data, setData] = useState<HostedBootstrap | undefined>(initialData);
	const [error, setError] = useState<string>();
	const redirecting = useRef(false);
	const token = auth.user?.access_token;
	const request = useMemo(
		() => (target ? createHostedRequest(target, token) : null),
		[target, token],
	);
	useEffect(() => {
		if (initialData) return;
		if (!request || auth.isLoading || auth.activeNavigator) return;
		const controller = new AbortController();
		setData(undefined);
		setError(undefined);
		(async () => {
			try {
				const response = await request("", { signal: controller.signal });
				setData((await response.json()) as HostedBootstrap);
			} catch (cause) {
				if (controller.signal.aborted) return;
				if (
					cause instanceof HostedHttpError &&
					cause.status === 401 &&
					!auth.isAuthenticated &&
					!redirecting.current
				) {
					redirecting.current = true;
					const returnPath =
						window.location.pathname +
						window.location.search +
						window.location.hash;
					saveReturnUrl(returnPath);
					try {
						await auth.signinRedirect({ url_state: returnPath });
					} catch (signInError) {
						setError(
							signInError instanceof Error
								? signInError.message
								: "Sign-in could not be started.",
						);
					}
					return;
				}
				setError(
					cause instanceof Error
						? cause.message
						: "This interface is unavailable.",
				);
			}
		})();
		return () => controller.abort();
	}, [
		initialData,
		request,
		auth.isLoading,
		auth.isAuthenticated,
		auth.activeNavigator,
		auth.signinRedirect,
	]);
	const runtimeRequest = useMemo(
		() =>
			target && data
				? createHostedRequest(
						{ ...target, variant: data.bootstrap.servedVariant ?? "stable" },
						token,
					)
				: null,
		[target, data, token],
	);
	if (auth.error) return <Status error>{auth.error.message}</Status>;
	if (error) return <Status error>{error}</Status>;
	if (auth.isLoading || auth.activeNavigator)
		return <Status>Completing sign-in…</Status>;
	if (!target)
		return (
			<Status>
				Open a published chat, form, or page using its shared link.
			</Status>
		);
	if (!data || !runtimeRequest)
		return (
			<Status>
				{redirecting.current ? "Redirecting to sign in…" : "Loading…"}
			</Status>
		);
	return (
		<HostedRuntime
			data={data}
			target={target}
			request={runtimeRequest}
			queryParams={Object.fromEntries(searchParams.entries())}
		/>
	);
}

function HostedRuntime({
	data,
	target,
	request,
	queryParams,
}: {
	data: HostedBootstrap;
	target: HostedTarget;
	request: ReturnType<typeof createHostedRequest>;
	queryParams: Record<string, string>;
}) {
	const backend = useMemo(
		() => createHostedBackend(data, request),
		[data, request],
	);
	const config = useMemo(
		() => parseUint8ArrayToJson(data.bootstrap.event.config) ?? {},
		[data.bootstrap.event.config],
	);
	const [ready, setReady] = useState(false);
	const updateQueryParams = useCallback((href: string, replace: boolean) => {
		window.history[replace ? "replaceState" : "pushState"](
			null,
			"",
			href + window.location.hash,
		);
	}, []);
	const [queryClient] = useState(
		() =>
			new QueryClient({
				defaultOptions: {
					queries: { retry: false, refetchOnWindowFocus: false },
				},
			}),
	);
	const [toolbar, setToolbar] = useState<ReactElement[]>([]);
	const [navigation, setNavigation] = useState<ReactElement[]>([]);
	const toolbarRef = useRef<IToolBarActions>({
		pushToolbarElements: setToolbar,
		pushNavElements: setNavigation,
	});
	const sidebarRef = useRef<ISidebarActions>(
		null,
	) as React.RefObject<ISidebarActions>;
	const onNavigate = useCallback(
		async (
			route: string,
			replace: boolean,
			params?: Record<string, string>,
		) => {
			try {
				const routes = (await (
					await request("/routes")
				).json()) as HostedRoute[];
				const path = hostedNavigationPath(
					target.app,
					route,
					routes,
					params,
					isHostedQueryPath(window.location.pathname),
				);
				if (replace) window.location.replace(path);
				else window.location.assign(path);
			} catch (cause) {
				toast.error(
					cause instanceof Error
						? cause.message
						: "The page could not be opened.",
				);
			}
		},
		[request, target.app],
	);
	useEffect(() => {
		const previousBackend = useBackendStore.getState().backend;
		useBackendStore.getState().setBackend(backend);
		setReady(true);
		return () => {
			queryClient.clear();
			if (useBackendStore.getState().backend === backend) {
				useBackendStore.setState({ backend: previousBackend });
			}
		};
	}, [backend, queryClient]);
	if (!ready) return <Status>Loading…</Status>;
	const { bootstrap, app_id: appId } = data;
	const event = bootstrap.event;
	const props = { appId, event, config, toolbarRef, sidebarRef, onNavigate };
	return (
		<I18nProvider>
			<QueryClientProvider client={queryClient}>
				<ThemeProvider
					attribute="class"
					defaultTheme={config?.color_scheme ?? "system"}
					enableSystem
				>
					<TooltipProvider>
						<QueryParamNavigationContext.Provider value={updateQueryParams}>
							<ExecutionEngineProviderComponent>
								<main
									id="hosted-interface"
									className="flex h-dvh min-h-0 flex-col overflow-hidden"
								>
									<ScopedCustomCss
										css={bootstrap.appCustomCss}
										scopeSelector="#hosted-interface"
									/>
									{data.kind !== "u" && (
										<header className="flex min-h-14 shrink-0 items-center gap-2 border-b px-4">
											<span className="truncate font-medium">{event.name}</span>
											<nav className="flex flex-1 items-center gap-2">
												{navigation}
											</nav>
											<div className="flex items-center gap-1">{toolbar}</div>
										</header>
									)}
									<Container ref={sidebarRef}>
										{data.kind === "c" ? (
											<ChatFeedbackEnabledContext.Provider value={false}>
												<ChatInterface {...props} />
											</ChatFeedbackEnabledContext.Provider>
										) : data.kind === "f" ? (
											<GenericEventFormInterface {...props} />
										) : bootstrap.page ? (
											<PageInterface
												{...props}
												page={bootstrap.page}
												pageRevision={bootstrap.revision ?? undefined}
												pageExecutionRevision={
													bootstrap.executionRevision ?? undefined
												}
												pageElementDemand={bootstrap.elementDemand ?? undefined}
												route={bootstrap.canonicalRoute ?? undefined}
												queryParams={queryParams}
												onNavigationMessage={(message) => {
													if (message.type === "navigateTo") {
														void onNavigate(
															message.route,
															message.replace,
															message.queryParams,
														);
														return;
													}
													const next = new URL(window.location.href);
													if (message.value === undefined)
														next.searchParams.delete(message.key);
													else
														next.searchParams.set(message.key, message.value);
													window.history[
														message.replace ? "replaceState" : "pushState"
													](null, "", next.pathname + next.search + next.hash);
												}}
											/>
										) : (
											<Status error>
												This event does not have a published page.
											</Status>
										)}
									</Container>
								</main>
								<Toaster />
							</ExecutionEngineProviderComponent>
						</QueryParamNavigationContext.Provider>
					</TooltipProvider>
				</ThemeProvider>
			</QueryClientProvider>
		</I18nProvider>
	);
}
