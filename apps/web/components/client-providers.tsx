"use client";
import {
	ExecutionEngineProviderComponent,
	ExecutionServiceProvider,
	QueryClient,
	QueryClientProvider,
	ReactFlowProvider,
} from "@flow-like/flow-like-ui";
import { ThemeProvider } from "@flow-like/flow-like-ui/components/theme-provider";
import { NetworkStatusIndicator } from "@flow-like/flow-like-ui/components/ui/network-status-indicator";
import { Toaster } from "@flow-like/flow-like-ui/components/ui/sonner";
import { TooltipProvider } from "@flow-like/flow-like-ui/components/ui/tooltip";
import { GlobalUpgradeDialog } from "@flow-like/flow-like-ui/components/upgrade/upgrade-dialog";
import { useNetworkStatus } from "@flow-like/flow-like-ui/hooks/use-network-status";
import { runIDBCleanup } from "@flow-like/flow-like-ui/lib/idb-cleanup";
import {
	cleanupLegacyQueryCacheBlob,
	createSmartQueryPersister,
} from "@flow-like/flow-like-ui/lib/query-persister";
import { I18nProvider } from "@flow-like/locales";
import dynamic from "next/dynamic";
import { useEffect } from "react";
import { AppSidebar } from "../components/app-sidebar";
import { WebAuthProvider } from "../components/auth-provider";
import { OAuthCallbackHandler } from "../components/oauth-callback-handler";
import { OAuthExecutionProvider } from "../components/oauth-execution-provider";
import { RuntimeVariablesProviderComponent } from "../components/runtime-variables-provider";
import { SpotlightWrapper } from "../components/spotlight-wrapper";
import { TelemetryProvider } from "../components/telemetry-provider";
import { ThemeLoader } from "../components/theme-loader";
import { WebProvider } from "../components/web-provider";

// Keep the always-mounted chat surfaces out of the synchronous root-layout chunk. The bridge is
// still rendered unconditionally once its client chunk loads, so it keeps listening independently
// of whether the overlay is open.
const GlobalChatOverlay = dynamic(
	() =>
		import("@flow-like/flow-like-ui/components/global-chat/global-chat-overlay").then(
			(module) => module.GlobalChatOverlay,
		),
	{ ssr: false },
);
const GlobalToolBridge = dynamic(
	() =>
		import("@flow-like/flow-like-ui/components/global-chat/global-tool-bridge").then(
			(module) => module.GlobalToolBridge,
		),
	{ ssr: false },
);
const FlowPilotBubbleButton = dynamic(
	() =>
		import("@flow-like/flow-like-ui/components/global-chat/flowpilot-bubble-button").then(
			(module) => module.FlowPilotBubbleButton,
		),
	{ ssr: false },
);

// Per-query persistence: each query is written individually as it
// resolves and restored lazily on first mount — no whole-client blob.
// Retention policy (denylist + size cap) lives in query-persister.ts.
const queryPersister = createSmartQueryPersister();
const queryClient = new QueryClient({
	defaultOptions: {
		queries: {
			networkMode: "always",
			staleTime: 30 * 1000,
			gcTime: 24 * 60 * 60 * 1000,
			refetchOnWindowFocus: false,
			refetchOnReconnect: false,
			refetchOnMount: true,
			retry: 1,
			retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
			persister: queryPersister.persisterFn,
		},
	},
});

function NetworkAwareProvider({ children }: { children: React.ReactNode }) {
	const isOnline = useNetworkStatus();

	useEffect(() => {
		if (isOnline) {
			console.log("Network reconnected - refetching stale queries");
			queryClient.refetchQueries({
				type: "active",
				stale: true,
			});
		}
	}, [isOnline]);

	return <>{children}</>;
}

export function ClientProviders({ children }: { children: React.ReactNode }) {
	useEffect(() => {
		const timer = setTimeout(() => {
			runIDBCleanup().catch(() => {});
			void queryPersister.persisterGc();
			void cleanupLegacyQueryCacheBlob();
		}, 5_000);
		return () => clearTimeout(timer);
	}, []);

	return (
		<ReactFlowProvider>
			<QueryClientProvider client={queryClient}>
				<NetworkAwareProvider>
					<NetworkStatusIndicator />
					<I18nProvider>
						<ThemeProvider
							attribute="class"
							defaultTheme="system"
							enableSystem
							storageKey="theme"
							disableTransitionOnChange
						>
							<TooltipProvider>
								<Toaster />
								<WebProvider>
									<WebAuthProvider>
										<ThemeLoader />
										<OAuthCallbackHandler>
											<OAuthExecutionProvider>
												<RuntimeVariablesProviderComponent>
													<ExecutionServiceProvider>
														<ExecutionEngineProviderComponent>
															<SpotlightWrapper>
																<TelemetryProvider>
																	<AppSidebar>{children}</AppSidebar>
																	<GlobalToolBridge />
																	<GlobalChatOverlay />
																	<FlowPilotBubbleButton />
																	<GlobalUpgradeDialog />
																</TelemetryProvider>
															</SpotlightWrapper>
														</ExecutionEngineProviderComponent>
													</ExecutionServiceProvider>
												</RuntimeVariablesProviderComponent>
											</OAuthExecutionProvider>
										</OAuthCallbackHandler>
									</WebAuthProvider>
								</WebProvider>
							</TooltipProvider>
						</ThemeProvider>
					</I18nProvider>
				</NetworkAwareProvider>
			</QueryClientProvider>
		</ReactFlowProvider>
	);
}
