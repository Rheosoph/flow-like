// @vitest-environment happy-dom
import { type ReactNode, act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

type Children = { children?: ReactNode };

const mocks = vi.hoisted(() => ({
	signedIn: false,
	onComplete: vi.fn(() => () => undefined),
	getAvailability: vi.fn(async () => ({ status: "available" })),
}));

vi.mock("../../lib/init-idb-sqlite", () => ({}));
vi.mock("../../lib/init-blob-offload", () => ({ initBlobOffload: () => {} }));
vi.mock("next/dynamic", () => ({ default: () => () => null }));
vi.mock("@flow-like/flow-like-ui", async () => {
	const query = await import("@tanstack/react-query");
	return {
		QueryClient: query.QueryClient,
		QueryClientProvider: query.QueryClientProvider,
		ExecutionEngineProviderComponent: ({ children }: Children) => children,
		ExecutionServiceProvider: ({ children }: Children) => children,
		useDownloadManager: (
			select: (state: { onComplete: typeof mocks.onComplete }) => unknown,
		) => select({ onComplete: mocks.onComplete }),
	};
});
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => ({
		profile: { id: "local-profile", hub: "api.flow-like.test", secure: true },
		userState: {
			getProfile: () => undefined,
			getAvailability: mocks.getAvailability,
		},
	}),
}));
vi.mock("@flow-like/flow-like-ui/hooks/use-invoke", () => ({
	useInvoke: () => ({ data: undefined }),
}));
vi.mock("@flow-like/flow-like-ui/hooks/use-network-status", () => ({
	useNetworkStatus: () => true,
}));
vi.mock("@flow-like/flow-like-ui/lib/platform", () => ({
	isWebkitLite: () => true,
}));
vi.mock("@flow-like/flow-like-ui/lib/query-persister", () => ({
	createSmartQueryPersister: () => ({ persisterGc: async () => {} }),
	cleanupLegacyQueryCacheBlob: async () => {},
}));
vi.mock("@flow-like/flow-like-ui/lib/page-surface-cache", () => ({
	purgeLegacyPageSurfaceCache: async () => {},
}));
vi.mock("@flow-like/locales", () => ({
	I18nProvider: ({ children }: Children) => children,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
	isPermissionGranted: async () => true,
	requestPermission: async () => "granted",
	sendNotification: () => {},
}));
vi.mock("../../components/auth-provider", async () => {
	const { AuthContext } = await import("react-oidc-context");
	const { createElement } = await import("react");
	return {
		DesktopAuthProvider: ({ children }: Children) =>
			createElement(
				AuthContext.Provider,
				{
					value: {
						isAuthenticated: mocks.signedIn,
						isLoading: false,
						user: mocks.signedIn
							? {
									profile: {
										sub: "person",
										iss: "https://auth.flow-like.test",
									},
								}
							: undefined,
					} as AuthContextProps,
				},
				children,
			),
	};
});

// Keep the actual shell composition and download notification hooks. Other
// native services are irrelevant to whether their auth context is available.
vi.mock(
	"@flow-like/flow-like-ui/components/bots-tab/app-local-principals",
	() => ({ AppLocalPrincipals: ({ children }: Children) => children }),
);
vi.mock("@flow-like/flow-like-ui/components/theme-provider", () => ({
	ThemeProvider: ({ children }: Children) => children,
}));
vi.mock(
	"@flow-like/flow-like-ui/components/ui/network-status-indicator",
	() => ({ NetworkStatusIndicator: () => null }),
);
vi.mock("@flow-like/flow-like-ui/components/ui/sonner", () => ({
	Toaster: () => null,
}));
vi.mock("@flow-like/flow-like-ui/components/ui/tooltip", () => ({
	TooltipProvider: ({ children }: Children) => children,
}));
vi.mock("@flow-like/flow-like-ui/components/upgrade/upgrade-dialog", () => ({
	GlobalUpgradeDialog: () => null,
}));
vi.mock("@flow-like/flow-like-ui/lib/use-navigation-provider", () => ({
	UseNavigationProvider: ({ children }: Children) => children,
}));
vi.mock("../../components/app-sidebar", () => ({
	AppSidebar: ({ children }: Children) => children,
}));
vi.mock("../../components/deeplink-navigation-handler", () => ({
	DeeplinkNavigationHandler: ({ children }: Children) => children,
}));
vi.mock("../../components/device-model-unlock-provider", () => ({
	DeviceModelUnlockProvider: () => null,
}));
vi.mock("../../components/global-anchor-component", () => ({
	default: () => null,
}));
vi.mock("../../components/idb-migration-gate", () => ({
	IdbMigrationGate: ({ children }: Children) => children,
}));
vi.mock("../../components/ios-webview-hardening", () => ({
	IOSWebviewHardening: () => null,
}));
vi.mock("../../components/local-bots-bridge", () => ({
	LocalBotsBridge: () => null,
}));
vi.mock("../../components/local-sink", () => ({
	LocalSinkConsentProvider: () => null,
}));
vi.mock("../../components/native-integration-provider", () => ({
	NativeIntegrationProvider: () => null,
}));
vi.mock("../../components/native-realtime-bridge", () => ({
	NativeRealtimeBridge: () => null,
}));
vi.mock("../../components/notification-provider", () => ({
	default: () => null,
}));
vi.mock("../../components/oauth-callback-handler", () => ({
	OAuthCallbackHandler: ({ children }: Children) => children,
}));
vi.mock("../../components/oauth-execution-provider", () => ({
	OAuthExecutionProvider: ({ children }: Children) => children,
}));
vi.mock("../../components/offline-writes-notifier", () => ({
	OfflineWritesNotifier: () => null,
}));
vi.mock("../../components/pending-invite-redeemer", () => ({
	PendingInviteRedeemer: () => null,
}));
vi.mock("../../components/rpa", () => ({ RpaPermissionProvider: () => null }));
vi.mock("../../components/runtime-variables-provider", () => ({
	RuntimeVariablesProviderComponent: ({ children }: Children) => children,
}));
vi.mock("../../components/spotlight-wrapper", () => ({
	SpotlightWrapper: ({ children }: Children) => children,
}));
vi.mock("../../components/tauri-provider", () => ({
	TauriProvider: ({ children }: Children) => children,
}));
vi.mock("../../components/telemetry-provider", () => ({
	TelemetryProvider: ({ children }: Children) => children,
}));
vi.mock("../../components/theme-loader", () => ({ ThemeLoader: () => null }));
vi.mock("../../components/toast-provider", () => ({ default: () => null }));
vi.mock("../../components/tray-provider", () => ({ default: () => null }));
vi.mock("../../components/update-provider", () => ({
	UpdateProvider: () => null,
}));

import { Providers } from "../providers";

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	mocks.onComplete.mockClear();
	mocks.getAvailability.mockClear();
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

test.each([false, true])(
	"startup provides auth to download notifications (signed in: %s)",
	async (signedIn) => {
		mocks.signedIn = signedIn;
		await act(async () => {
			root.render(
				createElement(
					Providers,
					null,
					createElement("main", null, "App ready"),
				),
			);
		});
		expect(container.textContent).toBe("App ready");
		expect(mocks.onComplete).toHaveBeenCalledOnce();
		if (signedIn) expect(mocks.getAvailability).toHaveBeenCalled();
	},
);
