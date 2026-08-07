"use client";
// IMPORTANT: keep this the first import. It replaces window.indexedDB with
// the SQLite-backed shim before Dexie/idb-keyval capture the native one.
import "../lib/init-idb-sqlite";
import {
	ExecutionEngineProviderComponent,
	ExecutionServiceProvider,
	PersistQueryClientProvider,
	QueryClient,
	ReactFlowProvider,
} from "@flow-like/flow-like-ui";
import { FlowPilotBubbleButton } from "@flow-like/flow-like-ui/components/global-chat/flowpilot-bubble-button";
import { GlobalChatOverlay } from "@flow-like/flow-like-ui/components/global-chat/global-chat-overlay";
import { GlobalToolBridge } from "@flow-like/flow-like-ui/components/global-chat/global-tool-bridge";
import { ThemeProvider } from "@flow-like/flow-like-ui/components/theme-provider";
import { NetworkStatusIndicator } from "@flow-like/flow-like-ui/components/ui/network-status-indicator";
import { Toaster } from "@flow-like/flow-like-ui/components/ui/sonner";
import { TooltipProvider } from "@flow-like/flow-like-ui/components/ui/tooltip";
import { GlobalUpgradeDialog } from "@flow-like/flow-like-ui/components/upgrade/upgrade-dialog";
import { useNetworkStatus } from "@flow-like/flow-like-ui/hooks/use-network-status";
import { createIDBPersister } from "@flow-like/flow-like-ui/lib/persister";
import { isWebkitLite } from "@flow-like/flow-like-ui/lib/platform";
import { useEffect } from "react";
import { AppSidebar } from "../components/app-sidebar";
import { DesktopAuthProvider } from "../components/auth-provider";
import { DeeplinkNavigationHandler } from "../components/deeplink-navigation-handler";
import DownloadNotificationProvider from "../components/download-notification-provider";
import GlobalAnchorHandler from "../components/global-anchor-component";
import { IdbMigrationGate } from "../components/idb-migration-gate";
import { IOSWebviewHardening } from "../components/ios-webview-hardening";
import NotificationProvider from "../components/notification-provider";
import { OAuthCallbackHandler } from "../components/oauth-callback-handler";
import { OAuthExecutionProvider } from "../components/oauth-execution-provider";
import { RpaPermissionProvider } from "../components/rpa";
import { RuntimeVariablesProviderComponent } from "../components/runtime-variables-provider";
import { SpotlightWrapper } from "../components/spotlight-wrapper";
import { TauriProvider } from "../components/tauri-provider";
import { TelemetryProvider } from "../components/telemetry-provider";
import { ThemeLoader } from "../components/theme-loader";
import ToastProvider from "../components/toast-provider";
import TrayProvider from "../components/tray-provider";
import { UpdateProvider } from "../components/update-provider";
import { initBlobOffload } from "../lib/init-blob-offload";

initBlobOffload();

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
		// Tag the document so CSS can disable WebKit-expensive effects (backdrop
		// filters etc.) on WKWebView/Safari while Chromium keeps the full look.
		document.documentElement.dataset.engine = isWebkitLite()
			? "webkit"
			: "blink";
	}, []);

	useEffect(() => {
		// When network comes back online, refetch all active queries
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

export function Providers({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	return (
		<IdbMigrationGate>
			<ReactFlowProvider>
				<PersistQueryClientProvider
					client={queryClient}
					persistOptions={{
						persister,
						maxAge: 24 * 60 * 60 * 1000,
					}}
				>
					<NetworkAwareProvider>
						<IOSWebviewHardening />
						<NetworkStatusIndicator />
						<UpdateProvider />
						<TrayProvider />
						<GlobalAnchorHandler />
						<ThemeProvider
							attribute="class"
							defaultTheme="system"
							enableSystem
							storageKey="theme"
							disableTransitionOnChange
						>
							<TooltipProvider>
								<Toaster />
								<ToastProvider />
								<TauriProvider>
									<DownloadNotificationProvider />
									<RpaPermissionProvider />
									<DeeplinkNavigationHandler>
										<OAuthCallbackHandler>
											<OAuthExecutionProvider>
												<DesktopAuthProvider>
													<NotificationProvider />
													<RuntimeVariablesProviderComponent>
														<ExecutionServiceProvider>
															<ExecutionEngineProviderComponent>
																<SpotlightWrapper>
																	<TelemetryProvider>
																		<ThemeLoader />
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
												</DesktopAuthProvider>
											</OAuthExecutionProvider>
										</OAuthCallbackHandler>
									</DeeplinkNavigationHandler>
								</TauriProvider>
							</TooltipProvider>
						</ThemeProvider>
					</NetworkAwareProvider>
				</PersistQueryClientProvider>
			</ReactFlowProvider>
		</IdbMigrationGate>
	);
}
