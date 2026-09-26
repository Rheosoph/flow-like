import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { PackagePermissionBits } from "../../lib/permission/wasm-package-permission";
import {
	type PackageMeta,
	PackageStatus,
	type RegistryEntry,
} from "../../lib/schema/wasm";
import type { GenericFetcher } from "../pages/store/store-package-detail";
import type { PackageDetailViewProps } from "./package-detail-view";

/** The first dynamic import transpiles the detail view's graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 60_000;
const PACKAGE_ID = "simple-math";
const STORE_TABS = ["overview", "nodes", "permissions", "versions", "reviews"];

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({
		url: `https://app.flow-like.com/store/packages?id=${PACKAGE_ID}`,
	});
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLAnchorElement: window.HTMLAnchorElement,
		HTMLImageElement: window.HTMLImageElement,
		Element: window.Element,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		ShadowRoot: window.ShadowRoot,
		DOMRect: window.DOMRect,
		Range: window.Range,
		Selection: window.Selection,
		HTMLCollection: window.HTMLCollection,
		NodeList: window.NodeList,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		IntersectionObserver: window.IntersectionObserver,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		NodeFilter: window.NodeFilter,
		ResizeObserver: window.ResizeObserver,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	Object.assign(window, { SyntaxError, TypeError });
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	await window.happyDOM.abort();
	restoreGlobals();
});

function registryEntry(permission?: number): RegistryEntry {
	return {
		id: PACKAGE_ID,
		manifest: {
			manifestVersion: 1,
			id: PACKAGE_ID,
			name: "Simple Math",
			version: "0.4.0",
			description: "",
			authors: [],
			keywords: [],
			primaryCategory: "DATA_TRANSFORMATION",
			permissions: {} as RegistryEntry["manifest"]["permissions"],
			metadata: {},
		},
		nodes: [],
		versions: [
			{
				version: "0.4.0",
				wasmHash: "",
				wasmSize: 0,
				publishedAt: "2026-09-02T00:00:00Z",
				yanked: false,
				status: PackageStatus.PendingReview,
			},
			{
				version: "0.3.0",
				wasmHash: "",
				wasmSize: 0,
				publishedAt: "2026-09-01T00:00:00Z",
				yanked: false,
				status: PackageStatus.Active,
			},
		],
		status: PackageStatus.Active,
		downloadCount: 0,
		createdAt: "2026-09-01T00:00:00Z",
		updatedAt: "2026-09-02T00:00:00Z",
		source: { type: "remote" },
		verified: false,
		price: 0,
		visibility: "public",
		currentUserPermission: permission,
	};
}

const EMPTY_META: PackageMeta = {
	id: PACKAGE_ID,
	lang: "en",
	name: "Simple Math",
	tags: [],
};

function recordingFetcher() {
	const paths: string[] = [];
	const fetcher = (async (_profile, path) => {
		paths.push(path);
		return path.endsWith("/meta") ? EMPTY_META : [];
	}) as GenericFetcher;
	return { paths, fetcher };
}

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

async function render(props: Partial<PackageDetailViewProps>) {
	const { act } = await import("react");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { useBackendStore } = await import("../../state/backend-state");
	const { PackageDetailView } = await import("./package-detail-view");

	useBackendStore.getState().setBackend({
		profile: undefined,
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "hub" } }),
		},
	} as never);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<PackageDetailView
					pkg={undefined}
					isLoading={false}
					installedVersion={null}
					onBack={() => {}}
					onInstall={() => {}}
					onUninstall={() => {}}
					{...props}
				/>
			</QueryClientProvider>,
		),
	);
	await flush();
}

/** Radix trigger ids end in `-trigger-<value>`. */
function tabs(): string[] {
	return Array.from(host.querySelectorAll('[role="tab"]')).map(
		(tab) => tab.id.split("-trigger-")[1] ?? "",
	);
}

function byText<T extends Element>(selector: string, label: string) {
	return Array.from(host.querySelectorAll(selector)).find(
		(candidate) => candidate.textContent?.trim() === label,
	) as T | undefined;
}

async function openTab(value: string) {
	const { act } = await import("react");
	const trigger = Array.from(host.querySelectorAll('[role="tab"]')).find(
		(tab) => tab.id.endsWith(`-trigger-${value}`),
	);
	if (!trigger) throw new Error(`No ${value} tab`);
	await act(async () => {
		trigger.dispatchEvent(
			new window.MouseEvent("mousedown", {
				bubbles: true,
				button: 0,
			}) as unknown as Event,
		);
	});
	await flush();
}

describe("PackageDetailView owner surface", () => {
	test(
		"a maintainer gets Manage package into the workspace and no owner tabs or owner requests",
		async () => {
			const { paths, fetcher } = recordingFetcher();
			await render({
				pkg: registryEntry(PackagePermissionBits.Maintainer),
				currentUserPermission: PackagePermissionBits.Maintainer,
				fetcher,
			});

			const manage = byText<HTMLAnchorElement>("a", "Manage package");
			expect(manage?.getAttribute("href")).toBe(
				`/store/package-workspace?id=${PACKAGE_ID}`,
			);
			expect(tabs()).toEqual(STORE_TABS);
			expect(paths).toEqual([`registry/package/${PACKAGE_ID}/meta`]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test("a buyer sees the store tabs without Manage package", async () => {
		const { fetcher } = recordingFetcher();
		await render({
			pkg: registryEntry(PackagePermissionBits.Buyer),
			currentUserPermission: PackagePermissionBits.Buyer,
			fetcher,
		});

		expect(byText("a", "Manage package")).toBeUndefined();
		expect(tabs()).toEqual(STORE_TABS);
	});

	test("the versions list offers no install buttons, even to a maintainer", async () => {
		const { fetcher } = recordingFetcher();
		await render({
			pkg: registryEntry(PackagePermissionBits.Owner),
			currentUserPermission: PackagePermissionBits.Owner,
			fetcher,
		});
		await openTab("versions");

		const panel = host.querySelector('[role="tabpanel"][data-state="active"]');
		expect(panel?.id.endsWith("-content-versions")).toBe(true);
		expect(panel?.textContent).toContain("0.3.0");
		expect(panel?.querySelectorAll("button")).toHaveLength(0);
	});

	test("an expired session with nothing loaded offers Sign in", async () => {
		const onSignIn = mock(() => {});
		const onRetry = mock(() => {});
		await render({ onSignIn, onRetry });

		expect(host.textContent).toContain("Your session expired");
		expect(byText("button", "Try again")).toBeUndefined();
		const { act } = await import("react");
		await act(async () => {
			byText<HTMLButtonElement>("button", "Sign in")?.click();
		});
		expect(onSignIn).toHaveBeenCalledTimes(1);
		expect(onRetry).not.toHaveBeenCalled();
	});
});
