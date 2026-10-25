// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IBit } from "../../../packages/ui/lib/schema/bit/bit";
import type { ISettingsProfile } from "../../../packages/ui/types";

const harness = vi.hoisted(() => {
	const createProfile = (): ISettingsProfile => ({
		hub_profile: {
			id: "profile-1",
			name: "Test profile",
			bits: [],
			apps: [],
			created: "2026-01-01T00:00:00Z",
			updated: "2026-01-01T00:00:00Z",
		},
		execution_settings: { gpu_mode: false, max_context_size: 8192 },
		created: "2026-01-01T00:00:00Z",
		updated: "2026-01-01T00:00:00Z",
	});
	return {
		createProfile,
		profile: createProfile(),
		apiGet: vi.fn(),
		apiPost: vi.fn(),
		toastError: vi.fn(),
	};
});

vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await import("../../../packages/ui/state/backend-state")),
	...(await import("../../../packages/ui/state/download-manager")),
	...(await import("../../../packages/ui/lib/schema/bit/bit")),
	...(await import("../../../packages/ui/lib/bit/tts-asset-repair")),
	useQueryClient: (await import("@tanstack/react-query")).useQueryClient,
	isAzureBlobStorageUrl: () => false,
	LoadingScreen: () => <output>Loading</output>,
}));
vi.mock("@flow-like/locales", () => ({
	i18n: { t: (_key: string, fallback: string) => fallback },
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("../lib/web-states", async () => {
	class UnusedState {}
	class WebUserState {
		async getSettingsProfile() {
			return harness.profile;
		}
		async getInfo() {
			return { tier: "FREE" };
		}
	}
	return {
		...Object.fromEntries(
			[
				"WebAIState",
				"WebApiKeyState",
				"WebApiState",
				"WebAppState",
				"WebBoardState",
				"WebDatabaseState",
				"WebEventState",
				"WebGraphState",
				"WebHelperState",
				"WebPageState",
				"WebQueryState",
				"WebRegistryState",
				"WebRoleState",
				"WebRouteState",
				"WebSinkState",
				"WebStorageState",
				"WebTeamState",
				"WebTemplateState",
				"WebUsageState",
				"WebWidgetState",
			].map((name) => [name, UnusedState]),
		),
		WebUserState,
		WebBitState: (await import("../lib/web-states/bit-state")).WebBitState,
	};
});
vi.mock("../lib/web-states/analytics-state", () => ({
	WebAnalyticsState: class {},
}));
vi.mock("../lib/web-states/sales-state", () => ({ WebSalesState: class {} }));
vi.mock("../lib/web-states/api-utils", () => ({
	apiGet: harness.apiGet,
	apiPost: harness.apiPost,
	apiPut: vi.fn(),
	apiDelete: vi.fn(),
}));
vi.mock("../../../packages/ui/hooks/use-hub", () => ({
	useHub: () => ({ hub: {} }),
}));
vi.mock("../../../packages/ui/hooks/use-invoke", async () => {
	const { useQuery } = await import("@tanstack/react-query");
	return {
		useInvoke: (
			fn: (...args: unknown[]) => unknown,
			state: unknown,
			args: unknown[],
		) =>
			useQuery({
				queryKey: [fn.name, ...args],
				queryFn: () => fn.apply(state, args),
			}),
	};
});
vi.mock(
	"../../../packages/ui/components/settings/devices/models/use/device-model-badge",
	() => ({ DeviceBitBadge: () => null }),
);
vi.mock("../../../packages/ui/components/ui/model-kit", () => ({
	DeploymentLabel: () => null,
	ModalityFlow: () => null,
	ProviderGlyph: () => null,
	providerLabel: () => "Forecasting",
}));
vi.mock("../../../packages/ui/components/ui/model-benchmarks", () => ({
	IntelligenceIndexBadge: () => null,
}));
vi.mock("sonner", () => ({ toast: { error: harness.toastError } }));

const { useBackendStore } = await import(
	"../../../packages/ui/state/backend-state"
);
const { useDownloadManager } = await import(
	"../../../packages/ui/state/download-manager"
);
const { IBitTypes } = await import("../../../packages/ui/lib/schema/bit/bit");
const { ModelCard } = await import(
	"../../../packages/ui/components/ui/model-card"
);
const { WebBackend, WebProvider } = await import("./web-provider");

function forecastingBit(): IBit {
	return {
		id: "forecast",
		hub: "hub.example",
		type: IBitTypes.Forecasting,
		authors: [],
		created: "now",
		updated: "now",
		hash: "forecast-hash",
		dependency_tree_hash: "",
		dependencies: [],
		meta: {
			en: {
				name: "Forecast model",
				description: "Test checkpoint",
				created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
				updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
				preview_media: [],
				tags: [],
			},
		},
		size: 100,
		file_name: "model.safetensors",
		download_link: "https://models.example/model.safetensors",
		parameters: {
			model_type: "timestral",
			provider: { provider_name: "local:forecast" },
			assets: [{ bit: "hub.example:config", relative_path: "config.json" }],
		},
	};
}

let root: Root;
let container: HTMLDivElement;
let queryClient: QueryClient;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	useBackendStore.setState({ backend: null });
	useDownloadManager.getState().setDownloadBackend(null);
	harness.profile = harness.createProfile();
	harness.apiGet.mockReset().mockResolvedValue({
		...forecastingBit(),
		id: "config",
		hash: "config-hash",
		type: IBitTypes.File,
		file_name: "config.json",
		size: 10,
		parameters: {},
	});
	harness.apiPost.mockReset().mockResolvedValue(undefined);
	harness.toastError.mockReset();
	queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	queryClient.clear();
	useBackendStore.setState({ backend: null });
	useDownloadManager.getState().setDownloadBackend(null);
	vi.restoreAllMocks();
});

async function renderProvider(client = queryClient) {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<WebProvider>
					<ModelCard bit={forecastingBit()} />
				</WebProvider>
			</QueryClientProvider>,
		),
	);
}

describe("web model downloads", () => {
	it("adds a forecasting model to the profile through the browser backend", async () => {
		await renderProvider();
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		const backend = useBackendStore.getState().backend;
		if (!backend) throw new Error("Web backend was not initialized");
		expect(useDownloadManager.getState().backend).toBe(backend);
		const downloadBit = vi.spyOn(backend.bitState, "downloadBit");
		const addButton = [...container.querySelectorAll("button")].find((button) =>
			button.textContent?.includes("Add to profile"),
		);
		expect(addButton).toBeDefined();
		if (!addButton) throw new Error("Add to profile button was not rendered");
		await act(async () => {
			addButton.click();
		});
		expect(downloadBit).toHaveBeenCalledOnce();
		expect(harness.apiGet).toHaveBeenCalledWith(
			"bit/config?hub=hub.example",
			undefined,
		);
		expect(harness.apiPost).toHaveBeenCalledWith(
			"profile/profile-1",
			{ bit_ids: ["forecast"] },
			undefined,
		);
		expect(harness.toastError).not.toHaveBeenCalled();
	});

	it("updates both stores when the provider gets a new query client", async () => {
		await renderProvider();
		const oldBackend = useDownloadManager.getState().backend;
		const nextClient = new QueryClient();
		await renderProvider(nextClient);
		expect(useDownloadManager.getState().backend).not.toBe(oldBackend);
		expect(useDownloadManager.getState().backend).toBe(
			useBackendStore.getState().backend,
		);
		expect(
			(useDownloadManager.getState().backend as InstanceType<typeof WebBackend>)
				.queryClient,
		).toBe(nextClient);
		nextClient.clear();
	});

	it("uses the active hosted backend and preserves its ownership on provider cleanup", async () => {
		await renderProvider();
		const previous = useDownloadManager.getState().backend;
		if (!previous) throw new Error("Download backend was not initialized");
		const oldDownload = vi.spyOn(previous.bitState, "downloadBit");
		const replacement = new WebBackend(() => undefined, queryClient);
		const newDownload = vi.spyOn(replacement.bitState, "downloadBit");
		await act(async () => useBackendStore.getState().setBackend(replacement));
		await useDownloadManager.getState().download(forecastingBit());
		expect(newDownload).toHaveBeenCalledOnce();
		expect(oldDownload).not.toHaveBeenCalled();
		useDownloadManager.getState().setDownloadBackend(replacement);
		await act(async () => root.render(null));
		expect(useDownloadManager.getState().backend).toBe(replacement);
		expect(useBackendStore.getState().backend).toBe(replacement);
	});

	it("clears its backend when unmounted and reports missing setup without a TypeError", async () => {
		await renderProvider();
		await act(async () => root.render(null));
		expect(useDownloadManager.getState().backend).toBeNull();
		expect(useBackendStore.getState().backend).toBeNull();
		await expect(
			useDownloadManager.getState().download(forecastingBit()),
		).rejects.toThrow("The download backend is not ready");
		useDownloadManager.setState({
			backend: {} as InstanceType<typeof WebBackend>,
		});
		await expect(
			useDownloadManager.getState().download(forecastingBit()),
		).rejects.toThrow("Backend does not support downloading bits.");
	});
});
