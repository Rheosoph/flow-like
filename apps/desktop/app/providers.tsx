"use client";
// IMPORTANT: keep this the first import. It replaces window.indexedDB with
// the SQLite-backed shim before Dexie/idb-keyval capture the native one.
import "../lib/init-idb-sqlite";
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
import { purgeLegacyPageSurfaceCache } from "@flow-like/flow-like-ui/lib/page-surface-cache";
import { isWebkitLite } from "@flow-like/flow-like-ui/lib/platform";
import {
	cleanupLegacyQueryCacheBlob,
	createSmartQueryPersister,
} from "@flow-like/flow-like-ui/lib/query-persister";
import { I18nProvider } from "@flow-like/locales";
import dynamic from "next/dynamic";
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
import { PendingInviteRedeemer } from "../components/pending-invite-redeemer";
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

// Keep the always-mounted chat surfaces out of the synchronous root-layout chunk. The bridge is
// still rendered unconditionally once its client chunk loads, so it keeps listening independently
// of whether the overlay is open.
const GlobalChatOverlay = dynamic(
	() =>
		import(
			"@flow-like/flow-like-ui/components/global-chat/global-chat-overlay"
		).then((module) => module.GlobalChatOverlay),
	{ ssr: false },
);
const GlobalToolBridge = dynamic(
	() =>
		import(
			"@flow-like/flow-like-ui/components/global-chat/global-tool-bridge"
		).then((module) => module.GlobalToolBridge),
	{ ssr: false },
);
const FlowPilotBubbleButton = dynamic(
	() =>
		import(
			"@flow-like/flow-like-ui/components/global-chat/flowpilot-bubble-button"
		).then((module) => module.FlowPilotBubbleButton),
	{ ssr: false },
);

initBlobOffload();

// Per-query persistence: each query is written individually as it
// resolves and restored lazily on first mount. There is no whole-client blob.
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
		// Tag the document so CSS can disable WebKit-expensive effects (backdrop
		// filters etc.) on WKWebView/Safari while Chromium keeps the full look.
		document.documentElement.dataset.engine = isWebkitLite()
			? "webkit"
			: "blink";
	}, []);

	useEffect(() => {
		// Off the critical path: sweep expired persisted queries, drop the legacy
		// whole-client cache blob (12MB) from the old persister, and remove page
		// surfaces written under the scheme that had no eviction at all.
		const handle = window.setTimeout(() => {
			void queryPersister.persisterGc();
			void cleanupLegacyQueryCacheBlob();
			void purgeLegacyPageSurfaceCache();
		}, 5000);
		return () => window.clearTimeout(handle);
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
				<QueryClientProvider client={queryClient}>
					<NetworkAwareProvider>
						<IOSWebviewHardening />
						<NetworkStatusIndicator />
						<UpdateProvider />
						<TrayProvider />
						<GlobalAnchorHandler />
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
									<ToastProvider />
									<TauriProvider>
										<DownloadNotificationProvider />
										<RpaPermissionProvider />
										<DeeplinkNavigationHandler>
											<OAuthCallbackHandler>
												<OAuthExecutionProvider>
													<DesktopAuthProvider>
														<PendingInviteRedeemer />
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
						</I18nProvider>
					</NetworkAwareProvider>
				</QueryClientProvider>
			</ReactFlowProvider>
		</IdbMigrationGate>
	);
}
