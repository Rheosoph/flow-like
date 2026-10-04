import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import {
	QueryClient,
	QueryClientProvider,
	notifyManager,
} from "@tanstack/react-query";
import { Window } from "happy-dom";
import { createInstance } from "i18next";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import store from "../../../locales/locales/en/store.json";
import type { IPage } from "../../state/backend-state/page-state";
import type {
	MicroWidgetInstanceComponent,
	SurfaceComponent,
} from "../a2ui/types";

const window = new Window({ url: "https://packages.local" });
Object.assign(window, { SyntaxError, TypeError });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	IS_REACT_ACT_ENVIRONMENT: true,
});
notifyManager.setNotifyFunction((callback) => act(callback));
const i18n = createInstance();
await i18n.init({
	lng: "en",
	resources: { en: { store } },
	interpolation: { escapeValue: false },
});

const OLD_HASH = "a".repeat(64);
const NEW_HASH = "b".repeat(64);
const contract: WidgetContract = {
	contractVersion: 1,
	id: "chart",
	inputs: { title: { type: "string", default: "Sales" } },
	events: { selected: { payloadSchema: { type: "string" } } },
	queries: {},
};

function widget(id: string, packageId = "sales"): SurfaceComponent {
	return {
		id,
		component: {
			id,
			type: "microWidgetInstance",
			instanceId: id,
			packageId,
			widgetId: "chart",
			packageVersion: "1.0.0",
			bundleHash: OLD_HASH,
			contract,
			props: { title: "Configured title" },
			actionBindings: { selected: { nodeId: "event" } },
		},
	};
}

function page(
	id: string,
	boardId: string,
	components: SurfaceComponent[],
): IPage {
	return {
		id,
		name: id,
		boardId,
		layoutType: "freeform",
		components,
		content: [],
		createdAt: "2026-10-01T00:00:00Z",
		updatedAt: "2026-10-01T00:00:00Z",
	};
}

let root: Root;
let host: HTMLElement;
let client: QueryClient;
let pages: IPage[];
let offline: boolean;
let pinnedVersion: string;
let availableVersion: string;
const trace: string[] = [];
const saves: IPage[] = [];
const errors: string[] = [];
const patches: { path: string; body: { version: string } }[] = [];
const installPackage = mock(async () => {});
const addPackage = mock(async () => {});

function packageRecord(packageId: string) {
	return {
		id: packageId,
		appId: "app",
		packageId,
		packageName: packageId === "sales" ? "Sales kit" : "Other kit",
		version: packageId === "sales" ? pinnedVersion : "1.0.0",
		autoUpdate: false,
		stale: false,
		addedAt: "2026-10-01T00:00:00Z",
	};
}

function resolvedPackage(packageId: string) {
	const pkg = packageRecord(packageId);
	return {
		id: packageId,
		version: pkg.version,
		installedAt: pkg.addedAt,
		manifest: {
			name: pkg.packageName,
			widgetBundleHash:
				packageId === "sales" && (offline || pinnedVersion !== "1.0.0")
					? NEW_HASH
					: OLD_HASH,
			widgets: [{ id: "chart", name: "Chart", description: "", contract }],
		},
	};
}

const backend = {
	isOffline: async () => offline,
	userState: { getProfile: async () => ({ id: "profile" }) },
	appState: {
		listPackages: async () => ({ sales: pinnedVersion, other: "1.0.0" }),
		addPackage,
	},
	apiState: {
		get: async (_profile: unknown, path: string) => {
			if (path === "apps/app/packages/updates") {
				trace.push("get-updates");
				return pinnedVersion === availableVersion
					? []
					: [{ packageId: "sales", latestVersion: availableVersion }];
			}
			if (path === "apps/app/packages")
				return [packageRecord("sales"), packageRecord("other")];
			throw new Error(`Unexpected request: ${path}`);
		},
		patch: async (
			_profile: unknown,
			path: string,
			body: { version: string },
		) => {
			trace.push("patch-version");
			patches.push({ path, body });
			pinnedVersion = body.version;
		},
	},
	registryState: {
		getPackage: async (packageId: string) => {
			trace.push(`manifest:${packageId}`);
			return resolvedPackage(packageId);
		},
		getInstalledPackages: async () => [
			resolvedPackage("sales"),
			resolvedPackage("other"),
		],
		installPackage,
	},
	boardState: { getCatalog: async () => [], getBoard: async () => {} },
	pageState: {
		getPagesAuthoritative: async () =>
			pages.map((stored) => ({
				pageId: stored.id,
				name: stored.name,
				boardId: stored.boardId,
			})),
		getPageAuthoritative: async (
			_appId: string,
			pageId: string,
			boardId?: string,
		) => {
			const stored = pages.find((entry) => entry.id === pageId);
			expect(stored?.boardId).toBe(boardId);
			return structuredClone(stored);
		},
		updatePage: async (_appId: string, updated: IPage) => {
			trace.push(`save:${updated.id}`);
			saves.push(structuredClone(updated));
			pages = pages.map((stored) =>
				stored.id === updated.id ? updated : stored,
			);
		},
		getPage: async () => {},
		getPages: async () => {},
		getPageBootstrap: async () => {},
	},
};

mock.module("../../state/backend-state", () => ({
	useBackend: () => backend,
	useBackendReady: () => true,
}));
const navigation = { ...(await import("next/navigation")) };
mock.module("next/navigation", () => ({
	...navigation,
	useRouter: () => ({ push: () => {} }),
}));
mock.module("sonner", () => ({
	toast: {
		error: (message: string) => errors.push(message),
		warning: (message: string) => errors.push(message),
		info: () => {},
		success: () => {},
	},
}));
mock.module("./app-packages/package-tile", () => ({
	PackageTile: ({
		name,
		widgetUpdates,
	}: { name: string; widgetUpdates: ReactNode }) => (
		<article aria-label={name}>{widgetUpdates}</article>
	),
}));
mock.module("./app-packages/package-access-section", () => ({
	PackageAccessSection: () => null,
}));
mock.module("./app-packages/package-nodes-section", () => ({
	PackageNodeCategories: () => null,
	PackageNodeList: () => null,
}));
mock.module("./app-packages/package-updates-banner", () => ({
	PackageUpdatesBanner: () => null,
}));
mock.module("./app-packages/package-widgets-section", () => ({
	PackageWidgetsSection: () => null,
}));
mock.module("./package-search-dialog", () => ({
	PackageSearchDialog: () => null,
}));
mock.module("./widget-permissions", () => ({
	WidgetPermissionsButton: () => null,
	WidgetPermissionsSheet: () => null,
	useMicroWidgetConsentEntries: () => [],
}));
mock.module("../ui/empty-state", () => ({ EmptyState: () => null }));
const childrenOnly = ({ children }: { children: ReactNode }) => <>{children}</>;
mock.module("../ui/alert", () => ({
	Alert: childrenOnly,
	AlertTitle: childrenOnly,
	AlertDescription: childrenOnly,
}));
mock.module("../ui/tabs", () => ({
	Tabs: childrenOnly,
	TabsList: childrenOnly,
	TabsTrigger: () => null,
	TabsContent: ({ children, value }: { children: ReactNode; value: string }) =>
		value === "overview" ? <>{children}</> : null,
}));

const { AppPackagesPage } = await import("./app-packages-page");

async function until(condition: () => boolean) {
	for (let attempt = 0; attempt < 100; attempt++) {
		if (condition()) return;
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
	throw new Error(
		`Expected package page state did not arrive: ${host.textContent}`,
	);
}

async function mount() {
	await act(() =>
		root.render(
			<I18nextProvider i18n={i18n}>
				<QueryClientProvider client={client}>
					<AppPackagesPage appId="app" />
				</QueryClientProvider>
			</I18nextProvider>,
		),
	);
	await until(
		() =>
			client.isFetching() === 0 &&
			host.textContent?.includes("2 outdated widgets on 2 pages") === true,
	);
}

async function updateSalesWidgets() {
	const update = host.querySelector<HTMLButtonElement>(
		'button[aria-label="Update all widgets of Sales kit"]',
	);
	expect(update).not.toBeNull();
	expect(update?.disabled).toBe(false);
	await act(() => update?.click());
	await until(
		() =>
			saves.length === 2 &&
			client.isMutating() === 0 &&
			client.isFetching() === 0,
	);
}

beforeEach(() => {
	offline = false;
	pinnedVersion = "1.0.0";
	availableVersion = "2.0.0";
	pages = [
		page("overview", "board-a", [
			widget("sales-chart"),
			widget("other", "other"),
		]),
		page("quiz", "board-b", [widget("quiz-chart")]),
		page("other-page", "board-c", [widget("untouched", "other")]),
	];
	for (const items of [trace, saves, patches, errors]) items.length = 0;
	installPackage.mockClear();
	addPackage.mockClear();
	client = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	await act(() => root.unmount());
	client.clear();
	window.document.body.innerHTML = "";
});
afterAll(async () => window.happyDOM.abort());

test("the package action refreshes the release before updating only its widgets across boards", async () => {
	await mount();
	expect(host.textContent).toContain("Updates the package to v2.0.0 first.");
	const unrelatedWidget = structuredClone(pages[0].components[1]);
	const unrelatedPage = structuredClone(pages[2]);
	availableVersion = "2.1.0";
	trace.length = 0;
	await updateSalesWidgets();
	expect(patches).toEqual([
		{ path: "apps/app/packages/sales", body: { version: "2.1.0" } },
	]);
	expect(trace[0]).toBe("get-updates");
	expect(trace.indexOf("patch-version")).toBeLessThan(
		trace.indexOf("save:overview"),
	);
	expect(saves.map((saved) => [saved.id, saved.boardId])).toEqual([
		["overview", "board-a"],
		["quiz", "board-b"],
	]);
	for (const saved of saves) {
		const instance = saved.components[0]
			.component as MicroWidgetInstanceComponent;
		expect(instance.packageVersion).toBe("2.1.0");
		expect(instance.bundleHash).toBe(NEW_HASH);
		expect(instance.props).toEqual({ title: "Configured title" });
		expect(instance.actionBindings).toEqual({ selected: { nodeId: "event" } });
	}
	expect(pages[0].components[1]).toEqual(unrelatedWidget);
	expect(pages[2]).toEqual(unrelatedPage);
	expect(errors).toEqual([]);
});

test("a local package action refreshes the installed hash without patching or installing", async () => {
	offline = true;
	await mount();
	expect(host.textContent).not.toContain("Updates the package to");
	await updateSalesWidgets();
	expect(patches).toEqual([]);
	expect(installPackage).not.toHaveBeenCalled();
	expect(addPackage).not.toHaveBeenCalled();
	expect(trace).not.toContain("get-updates");
	for (const saved of saves) {
		const instance = saved.components[0]
			.component as MicroWidgetInstanceComponent;
		expect(instance.packageVersion).toBe("1.0.0");
		expect(instance.bundleHash).toBe(NEW_HASH);
	}
	expect(errors).toEqual([]);
});
