import {
	afterEach,
	beforeEach,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { AppPackageWidget } from "../../lib/package-widgets";
import type { IWidget } from "../../state/backend-state/widget-state";
import type {
	WidgetAccessRequest,
	WidgetGrantRequest,
	WidgetGrantResponse,
	WidgetPolicyDescriptor,
	WidgetPolicyRequest,
} from "../a2ui/micro-widget-policy";
import type { SurfaceComponent } from "../a2ui/types";

// The first render imports the A2UI component graph, several seconds on a cold cache.
setDefaultTimeout(60_000);

const PROJECT = "app-1";
const ACCESS = "eyJhbGciOiJFUzI1NiJ9.eyJwa2ciOiJ4In0.YWNjZXNz";
const DIGEST = `sha256:${"a".repeat(64)}`;
const BUNDLE_HASH = "b".repeat(64);
const CONTRACT = {
	contractVersion: 1,
	id: "live-map",
	capabilities: {},
} as WidgetContract;

const PINNED: AppPackageWidget = {
	packageId: "com.acme.maps",
	packageName: "Acme Maps",
	packageVersion: "1.2.0",
	bundleHash: BUNDLE_HASH,
	widget: {
		id: "live-map",
		name: "Live Map",
		description: "",
		contract: CONTRACT,
	},
};

const MAP: SurfaceComponent = {
	id: "root",
	component: {
		id: "root",
		type: "microWidgetInstance",
		instanceId: "map-1",
		packageId: PINNED.packageId,
		widgetId: PINNED.widget.id,
		packageVersion: PINNED.packageVersion,
		bundleHash: BUNDLE_HASH,
		contract: CONTRACT,
		props: {},
	},
};

const SALES_CARD: IWidget = {
	id: "sales-card",
	name: "Sales card",
	rootComponentId: "root",
	components: [MAP],
	dataModel: [],
	customizationOptions: [],
	tags: [],
	createdAt: "2026-09-30T10:00:00Z",
	updatedAt: "2026-09-30T10:00:00Z",
};

const THROUGH_PROJECT = {
	packageId: PINNED.packageId,
	packageVersion: PINNED.packageVersion,
	appId: PROJECT,
};

interface RegistryCalls {
	describe: WidgetPolicyRequest[];
	mint: WidgetGrantRequest[];
	access: WidgetAccessRequest[];
}

let window: Window;
let root: Root;
let host: HTMLElement;
let calls: RegistryCalls;
let client: import("@tanstack/react-query").QueryClient;
const cleanup: (() => void)[] = [];

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/library/config" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLIFrameElement: window.HTMLIFrameElement,
		SVGElement: window.SVGElement,
		Element: window.Element,
		Text: window.Text,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		ResizeObserver: window.ResizeObserver,
		NodeFilter: window.NodeFilter,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	Object.assign(window, { SyntaxError });
	cleanup.push(() => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	});

	// Keep the real frame lifecycle while loading a blank document without network access.
	const framePrototype = window.HTMLIFrameElement.prototype;
	const src = Object.getOwnPropertyDescriptor(framePrototype, "src");
	if (!src) throw new Error("The iframe source descriptor is missing");
	Object.defineProperty(framePrototype, "src", {
		...src,
		get: () => "about:blank",
	});
	cleanup.push(() => Object.defineProperty(framePrototype, "src", src));

	const [
		consent,
		grants,
		{ useBackendStore },
		{ QueryClient },
		{ createRoot },
	] = await Promise.all([
		import("../a2ui/micro-widget-capability-consent"),
		import("../a2ui/use-micro-widget-grant"),
		import("../../state/backend-state"),
		import("@tanstack/react-query"),
		import("react-dom/client"),
	]);
	consent.resetMicroWidgetConsentForTests();
	grants.resetMicroWidgetGrantCacheForTests();

	calls = { describe: [], mint: [], access: [] };
	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
	useBackendStore.getState().setBackend({
		userState: { getProfile: async () => null },
		eventState: {},
		boardState: {},
		widgetState: {
			getWidgets: async () => [[PROJECT, SALES_CARD.id, { name: "Sales" }]],
			getWidget: async () => SALES_CARD,
		},
		registryState: {
			describeWidgetPolicy: async (
				request: WidgetPolicyRequest,
			): Promise<WidgetPolicyDescriptor> => {
				calls.describe.push(request);
				return {
					source: "registry:api.flow-like.com",
					packageId: request.packageId,
					packageVersion: request.packageVersion,
					bundleHash: request.bundleHash ?? "",
					widgetId: request.widgetId,
					preview: request.preview,
					status: "ok",
					policy: {},
					policyDigest: DIGEST,
					networkInputs: [],
				};
			},
			mintWidgetGrant: async (
				request: WidgetGrantRequest,
			): Promise<WidgetGrantResponse> => {
				calls.mint.push(request);
				return {
					grant: null,
					expiresIn: 86_400,
					policyDigest: request.policyDigest,
					runtime: null,
				};
			},
			getWidgetAccess: async (request: WidgetAccessRequest) => {
				calls.access.push(request);
				return { access: ACCESS, expiresIn: 43_200 };
			},
		},
	} as never);

	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	cleanup.push(() => client.clear());
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	await window.happyDOM.abort();
	for (const restore of cleanup.splice(0).reverse()) restore();
});

/** Describe, mint and access resolve over several microtask hops; let every one land. */
async function settle() {
	const { act } = await import("react");
	for (let round = 0; round < 6; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}

async function render(node: ReactNode) {
	const [{ act }, { QueryClientProvider }, { AppRouterContext }] =
		await Promise.all([
			import("react"),
			import("@tanstack/react-query"),
			import("next/dist/shared/lib/app-router-context.shared-runtime"),
		]);
	await act(async () => {
		root.render(
			<AppRouterContext.Provider value={{} as never}>
				<QueryClientProvider client={client}>{node}</QueryClientProvider>
			</AppRouterContext.Provider>,
		);
	});
	await settle();
}

async function startPreview() {
	const { act } = await import("react");
	const button = Array.from(host.querySelectorAll("button")).find((candidate) =>
		candidate.textContent?.includes("Preview"),
	);
	if (!button) throw new Error("The widget card offers no preview");
	await act(async () => (button as HTMLButtonElement).click());
	await settle();
}

const frameSrc = () => host.querySelector("iframe")?.getAttribute("src") ?? "";

describe("package widget previews in a project", () => {
	test("the Packages page previews the pinned version through the project", async () => {
		const { PackageWidgetsSection } = await import(
			"./app-packages/package-widgets-section"
		);
		await render(
			<PackageWidgetsSection
				appId={PROJECT}
				widgets={[PINNED]}
				loading={false}
			/>,
		);
		expect(calls.describe).toHaveLength(0);

		await startPreview();

		expect(calls.describe).toHaveLength(1);
		expect(calls.describe[0]).toMatchObject({
			...THROUGH_PROJECT,
			widgetId: PINNED.widget.id,
			preview: true,
		});
		expect(calls.access).toEqual([THROUGH_PROJECT]);
		expect(frameSrc()).toContain(`/~${ACCESS}/`);
	});

	test("the store previews a version outside any project", async () => {
		const { WidgetCard } = await import("./widget-card");
		await render(
			<WidgetCard
				widget={PINNED.widget}
				packageId={PINNED.packageId}
				packageVersion={PINNED.packageVersion}
				bundleHash={BUNDLE_HASH}
			/>,
		);

		await startPreview();

		expect(calls.describe).toHaveLength(1);
		expect(calls.describe[0]).not.toHaveProperty("appId");
		expect(calls.access).toEqual([
			{ packageId: PINNED.packageId, packageVersion: PINNED.packageVersion },
		]);
	});

	test("the Widgets page renders a widget's package widget through the project", async () => {
		const { WidgetList } = await import("../pages/widget-list");
		await render(<WidgetList appId={PROJECT} />);

		expect(calls.describe[0]).toMatchObject({
			...THROUGH_PROJECT,
			widgetId: PINNED.widget.id,
		});
		expect(calls.access).toEqual([THROUGH_PROJECT]);
	});

	test("FlowPilot previews proposed components through their project", async () => {
		const { PendingComponentsView } = await import(
			"../flowpilot/PendingComponentsView"
		);
		await render(
			<PendingComponentsView
				appId={PROJECT}
				components={[MAP]}
				onApply={() => undefined}
				onDismiss={() => undefined}
			/>,
		);

		expect(calls.describe[0]).toMatchObject({
			...THROUGH_PROJECT,
			widgetId: PINNED.widget.id,
		});
		expect(calls.access).toEqual([THROUGH_PROJECT]);
	});

	test("the chat's review card previews through the project the run staged them for", async () => {
		const [{ useGlobalChatStore }, { PendingComponentsCard }] =
			await Promise.all([
				import("../../state/global-chat/global-chat-store"),
				import("../global-chat/pending-components-card"),
			]);
		useGlobalChatStore.getState().setPendingComponents("run-1", {
			components: [MAP],
			surfaceId: "builder-1",
			appId: PROJECT,
		});
		cleanup.push(() =>
			useGlobalChatStore.getState().setPendingComponents("run-1", null),
		);
		await render(<PendingComponentsCard />);

		expect(calls.describe[0]).toMatchObject({
			...THROUGH_PROJECT,
			widgetId: PINNED.widget.id,
		});
		expect(calls.access).toEqual([THROUGH_PROJECT]);
	});
});
