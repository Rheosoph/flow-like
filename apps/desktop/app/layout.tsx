"use client";
import {
	ExecutionEngineProviderComponent,
	ExecutionServiceProvider,
	PersistQueryClientProvider,
	QueryClient,
	ReactFlowProvider,
} from "@flow-like/flow-like-ui";
import { ThemeProvider } from "@flow-like/flow-like-ui/components/theme-provider";
import { Toaster } from "@flow-like/flow-like-ui/components/ui/sonner";
import { TooltipProvider } from "@flow-like/flow-like-ui/components/ui/tooltip";
import "@flow-like/flow-like-ui/global.css";
import { NetworkStatusIndicator } from "@flow-like/flow-like-ui/components/ui/network-status-indicator";
import { useNetworkStatus } from "@flow-like/flow-like-ui/hooks/use-network-status";
import { createIDBPersister } from "@flow-like/flow-like-ui/lib/persister";
import { isWebkitLite } from "@flow-like/flow-like-ui/lib/platform";
import {
	Architects_Daughter,
	DM_Sans,
	Fira_Code,
	Geist,
	Geist_Mono,
	IBM_Plex_Mono,
	IBM_Plex_Sans,
	Inter,
	JetBrains_Mono,
	Libre_Baskerville,
	Lora,
	Merriweather,
	Montserrat,
	Open_Sans,
	Outfit,
	Oxanium,
	Playfair_Display,
	Plus_Jakarta_Sans,
	Poppins,
	Roboto,
	Roboto_Mono,
	Source_Code_Pro,
	Source_Serif_4,
	Space_Grotesk,
	Space_Mono,
} from "next/font/google";
import { useEffect } from "react";
import { AppSidebar } from "../components/app-sidebar";
import { DesktopAuthProvider } from "../components/auth-provider";
import { DeeplinkNavigationHandler } from "../components/deeplink-navigation-handler";
import DownloadNotificationProvider from "../components/download-notification-provider";
import GlobalAnchorHandler from "../components/global-anchor-component";
import { GlobalChatOverlay } from "../components/global-chat/global-chat-overlay";
import { GlobalToolBridge } from "../components/global-chat/global-tool-bridge";
import { IOSWebviewHardening } from "../components/ios-webview-hardening";
import NotificationProvider from "../components/notification-provider";
import { OAuthCallbackHandler } from "../components/oauth-callback-handler";
import { OAuthExecutionProvider } from "../components/oauth-execution-provider";
import { RpaPermissionProvider } from "../components/rpa";
import { RuntimeVariablesProviderComponent } from "../components/runtime-variables-provider";
import { SpotlightWrapper } from "../components/spotlight-wrapper";
import { TauriProvider } from "../components/tauri-provider";
import { ThemeLoader } from "../components/theme-loader";
import ToastProvider from "../components/toast-provider";
import TrayProvider from "../components/tray-provider";
import { UpdateProvider } from "../components/update-provider";
import { initBlobOffload } from "../lib/init-blob-offload";
import PostHogPageView from "./PostHogPageView";
import { PHProvider } from "./provider";

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

const inter = Inter({ subsets: ["latin"], preload: true });
const dmSans = DM_Sans({ subsets: ["latin"], preload: true });
const firaCode = Fira_Code({ subsets: ["latin"], preload: true });
const geist = Geist({ subsets: ["latin"], preload: true });
const geistMono = Geist_Mono({ subsets: ["latin"], preload: true });
const ibmPlexMono = IBM_Plex_Mono({
	subsets: ["latin"],
	weight: ["100", "200", "300", "400", "500", "600", "700"],
	preload: true,
});
const ibmPlexSans = IBM_Plex_Sans({
	subsets: ["latin"],
	weight: ["100", "200", "300", "400", "500", "600", "700"],
	preload: true,
});
const jetBrainsMono = JetBrains_Mono({ subsets: ["latin"], preload: true });
const libreBaskerville = Libre_Baskerville({
	subsets: ["latin"],
	weight: ["400", "700"],
	preload: true,
});
const lora = Lora({ subsets: ["latin"], preload: true });
const merriweather = Merriweather({ subsets: ["latin"], preload: true });
const montserrat = Montserrat({ subsets: ["latin"], preload: true });
const openSans = Open_Sans({ subsets: ["latin"], preload: true });
const outfit = Outfit({ subsets: ["latin"], preload: true });
const oxanium = Oxanium({ subsets: ["latin"], preload: true });
const playfairDisplay = Playfair_Display({ subsets: ["latin"], preload: true });
const plusJakartaSans = Plus_Jakarta_Sans({
	subsets: ["latin"],
	preload: true,
});
const poppins = Poppins({
	subsets: ["latin"],
	weight: ["100", "200", "300", "400", "500", "600", "700", "800", "900"],
	preload: true,
});
const roboto = Roboto({
	subsets: ["latin"],
	weight: ["100", "300", "400", "500", "700", "900"],
	preload: true,
});
const robotoMono = Roboto_Mono({ subsets: ["latin"], preload: true });
const sourceCodePro = Source_Code_Pro({ subsets: ["latin"], preload: true });
const sourceSerif4 = Source_Serif_4({ subsets: ["latin"], preload: true });
const spaceGrotesk = Space_Grotesk({ subsets: ["latin"], preload: true });
const spaceMono = Space_Mono({
	subsets: ["latin"],
	weight: ["400", "700"],
	preload: true,
});
const architectsDaughter = Architects_Daughter({
	subsets: ["latin"],
	weight: ["400"],
	preload: true,
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

export default function RootLayout({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	return (
		<html
			lang="en"
			data-desktop-app="true"
			suppressHydrationWarning
			suppressContentEditableWarning
			className="min-h-screen"
		>
			{/* <ReactScan /> */}
			<PHProvider>
				<ReactFlowProvider>
					<PersistQueryClientProvider
						client={queryClient}
						persistOptions={{
							persister,
							maxAge: 24 * 60 * 60 * 1000,
						}}
					>
						<NetworkAwareProvider>
							<body className={inter.className} data-desktop-app="true">
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
																			<PostHogPageView />
																			<ThemeLoader />
																			<AppSidebar>{children}</AppSidebar>
																			<GlobalToolBridge />
																			<GlobalChatOverlay />
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
							</body>
						</NetworkAwareProvider>
					</PersistQueryClientProvider>
				</ReactFlowProvider>
			</PHProvider>
		</html>
	);
}
