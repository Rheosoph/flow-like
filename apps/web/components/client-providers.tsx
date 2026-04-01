"use client";
import {
	ExecutionEngineProviderComponent,
	ExecutionServiceProvider,
	PersistQueryClientProvider,
	QueryClient,
	ReactFlowProvider,
} from "@tm9657/flow-like-ui";
import { ThemeProvider } from "@tm9657/flow-like-ui/components/theme-provider";
import { NetworkStatusIndicator } from "@tm9657/flow-like-ui/components/ui/network-status-indicator";
import { Toaster } from "@tm9657/flow-like-ui/components/ui/sonner";
import { TooltipProvider } from "@tm9657/flow-like-ui/components/ui/tooltip";
import { useNetworkStatus } from "@tm9657/flow-like-ui/hooks/use-network-status";
import { createIDBPersister } from "@tm9657/flow-like-ui/lib/persister";
import { runIDBCleanup } from "@tm9657/flow-like-ui/lib/idb-cleanup";
import { useEffect } from "react";
import { AppSidebar } from "../components/app-sidebar";
import { WebAuthProvider } from "../components/auth-provider";
import { OAuthCallbackHandler } from "../components/oauth-callback-handler";
import { OAuthExecutionProvider } from "../components/oauth-execution-provider";
import { RuntimeVariablesProviderComponent } from "../components/runtime-variables-provider";
import { SpotlightWrapper } from "../components/spotlight-wrapper";
import { ThemeLoader } from "../components/theme-loader";
import { WebProvider } from "../components/web-provider";

const persister = createIDBPersister();
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
		const timer = setTimeout(() => runIDBCleanup().catch(() => {}), 5_000);
		return () => clearTimeout(timer);
	}, []);

	return (
		<ReactFlowProvider>
			<PersistQueryClientProvider
				client={queryClient}
				persistOptions={{
					persister,
					maxAge: 24 * 60 * 60 * 1000,
				}}
			>
				<NetworkAwareProvider>
					<NetworkStatusIndicator />
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
															<AppSidebar>{children}</AppSidebar>
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
				</NetworkAwareProvider>
			</PersistQueryClientProvider>
		</ReactFlowProvider>
	);
}
