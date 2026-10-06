import {
	afterEach,
	beforeEach,
	describe,
	expect,
	setDefaultTimeout,
	spyOn,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AppPackageWidget } from "../../../lib/package-widgets";
import type { FlwEnvelope } from "../micro-widget-host";
import {
	type WidgetAccessRequest,
	type WidgetAccessResponse,
	type WidgetGrantRequest,
	type WidgetGrantResponse,
	WidgetPolicyChangedError,
	type WidgetPolicyDescriptor,
	type WidgetPolicyRequest,
} from "../micro-widget-policy";
import type { MicroWidgetReloader } from "../micro-widget-reload";
import type { MicroWidgetInstanceComponent } from "../types";

let window: Window;
let root: Root;
let host: HTMLElement;
let client: import("@tanstack/react-query").QueryClient | undefined;
let registryState: Record<string, unknown> | undefined;
let restoreFrameSrc: () => void;
let restoreGlobals: () => void;
let restoreTimers: () => void;
let restoreBackend: (() => void) | undefined;
let readyTimeout: (() => void) | undefined;
const cleanup: (() => void)[] = [];

interface ArmedTimer {
	delay: number;
	handle: unknown;
	run: () => void;
}
/** Timers of a second or more that are neither fired nor cleared; tests fire them instead of waiting. */
const armedTimers: ArmedTimer[] = [];

// The first render imports the widget graph (about 5 s cold), past bun's 5 s default on a loaded machine.
setDefaultTimeout(60_000);

const SOURCE = "registry:hub.example.com";
const BUNDLE_HASH = "b".repeat(64);
const DIGEST_A = `sha256:${"a".repeat(64)}`;
const DIGEST_B = `sha256:${"c".repeat(64)}`;
const GRANT_A = "1".repeat(64);
const GRANT_B = "2".repeat(64);
const MAP_HOST = "https://tiles.example.com";

beforeEach(async () => {
	client = undefined;
	registryState = undefined;
	restoreBackend = undefined;
	readyTimeout = undefined;
	armedTimers.length = 0;
	const scheduleTimeout = globalThis.setTimeout;
	const cancelTimeout = globalThis.clearTimeout;
	const disarm = (timer: ArmedTimer | undefined) => {
		const index = timer ? armedTimers.indexOf(timer) : -1;
		if (index !== -1) armedTimers.splice(index, 1);
	};
	const timerSpy = spyOn(globalThis, "setTimeout");
	timerSpy.mockImplementation(((...args: Parameters<typeof setTimeout>) => {
		const [callback, delay, ...callbackArgs] = args;
		if (delay === 10_000) readyTimeout = () => callback(...callbackArgs);
		if (typeof delay !== "number" || delay < 1_000) {
			return scheduleTimeout(...args);
		}
		const fire = () => {
			disarm(timer);
			callback(...callbackArgs);
		};
		const handle = scheduleTimeout(fire, delay);
		const timer: ArmedTimer = {
			delay,
			handle,
			run: () => {
				cancelTimeout(handle);
				fire();
			},
		};
		armedTimers.push(timer);
		return handle;
	}) as typeof setTimeout);
	const clearSpy = spyOn(globalThis, "clearTimeout");
	clearSpy.mockImplementation(((...args: Parameters<typeof clearTimeout>) => {
		disarm(armedTimers.find((timer) => timer.handle === args[0]));
		cancelTimeout(...args);
	}) as typeof clearTimeout);
	restoreTimers = () => {
		timerSpy.mockRestore();
		clearSpy.mockRestore();
	};
	window = new Window({ url: "https://local/use" });
	const globals = {
		document: window.document,
		HTMLElement: window.HTMLElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLIFrameElement: window.HTMLIFrameElement,
		Element: window.Element,
		Text: window.Text,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		navigator: window.navigator,
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
		window,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	Object.assign(window, { __TAURI_INTERNALS__: {}, SyntaxError });
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	// Keep the real frame lifecycle while loading a blank document without network access.
	const framePrototype = window.HTMLIFrameElement.prototype;
	const src = Object.getOwnPropertyDescriptor(framePrototype, "src");
	if (!src) throw new Error("The iframe source descriptor is missing");
	Object.defineProperty(framePrototype, "src", {
		...src,
		get: () => "about:blank",
	});
	restoreFrameSrc = () => Object.defineProperty(framePrototype, "src", src);
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);

	const [consent, grants] = await Promise.all([
		import("../micro-widget-capability-consent"),
		import("../use-micro-widget-grant"),
	]);
	consent.resetMicroWidgetConsentForTests();
	grants.resetMicroWidgetGrantCacheForTests();
});

afterEach(async () => {
	await act(() => root.unmount());
	for (const restore of cleanup.splice(0).reverse()) restore();
	client?.clear();
	restoreBackend?.();
	restoreTimers();
	restoreFrameSrc();
	await window.happyDOM.abort();
	restoreGlobals();
});

const component = (
	overrides: Partial<MicroWidgetInstanceComponent> = {},
): MicroWidgetInstanceComponent => ({
	id: "sales-chart",
	type: "microWidgetInstance",
	instanceId: "sales-chart",
	packageId: "com.example.sales",
	widgetId: "chart",
	packageVersion: "1.0.0",
	bundleHash: BUNDLE_HASH,
	props: { title: "Sales" },
	...overrides,
});

interface RegistryCalls {
	describe: WidgetPolicyRequest[];
	mint: WidgetGrantRequest[];
}

type MintAnswer = Omit<WidgetGrantResponse, "runtime"> &
	Partial<Pick<WidgetGrantResponse, "runtime">>;

/** Desktop-shaped registry backend: descriptors echo the request tuple. */
function stubRegistry(
	describe: (
		call: number,
		request: WidgetPolicyRequest,
	) => Partial<WidgetPolicyDescriptor> = () => ({}),
	mint: (
		request: WidgetGrantRequest,
		call: number,
	) => MintAnswer | Promise<MintAnswer> = (request) => ({
		grant: GRANT_A,
		expiresIn: 86_400,
		policyDigest: request.policyDigest,
	}),
): RegistryCalls {
	const calls: RegistryCalls = { describe: [], mint: [] };
	registryState = {
		describeWidgetPolicy: async (
			request: WidgetPolicyRequest,
		): Promise<WidgetPolicyDescriptor> => {
			calls.describe.push(request);
			return {
				source: SOURCE,
				packageId: request.packageId,
				bundleHash: request.bundleHash ?? "",
				widgetId: request.widgetId,
				preview: request.preview,
				status: "ok",
				policy: {},
				policyDigest: DIGEST_A,
				networkInputs: [],
				...describe(calls.describe.length, request),
			};
		},
		mintWidgetGrant: async (
			request: WidgetGrantRequest,
		): Promise<WidgetGrantResponse> => {
			calls.mint.push(request);
			return { runtime: null, ...(await mint(request, calls.mint.length)) };
		},
	};
	return calls;
}

/** Describe and mint resolve over several microtask hops; let every one land. */
async function settle() {
	for (let round = 0; round < 4; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}

/**
 * Without `page` the widget mounts outside a page, where actions never run.
 * A `reloader` stands in for the page builder.
 */
async function renderWidget(
	widgets: MicroWidgetInstanceComponent | MicroWidgetInstanceComponent[],
	page?: { router: Record<string, unknown>; appId?: string },
	reloader?: MicroWidgetReloader,
) {
	const [
		{ A2UIMicroWidget },
		{ ActionProvider },
		{ useBackendStore },
		{ QueryClient, QueryClientProvider },
		{ AppRouterContext },
		{ MicroWidgetReloadContext },
	] = await Promise.all([
		import("./A2UIMicroWidget"),
		import("../ActionHandler"),
		import("../../../state/backend-state"),
		import("@tanstack/react-query"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("../micro-widget-reload"),
	]);
	if (!client) {
		client = new QueryClient();
		const previousBackend = useBackendStore.getState().backend;
		restoreBackend = () =>
			useBackendStore.setState({ backend: previousBackend });
		useBackendStore.getState().setBackend({
			userState: { getProfile: async () => null },
			eventState: {},
			boardState: {},
			registryState,
		} as never);
	}
	const list = Array.isArray(widgets) ? widgets : [widgets];
	const widgetElement = createElement(
		MicroWidgetReloadContext.Provider,
		{ value: reloader ?? null },
		...list.map((widget, index) =>
			createElement(A2UIMicroWidget, {
				key: widget.id,
				component: widget as never,
				componentId: index === 0 ? "sales-chart" : widget.id,
				surfaceId: "page-1",
			} as never),
		),
	);
	await act(async () => {
		root.render(
			createElement(
				AppRouterContext.Provider,
				{ value: (page?.router ?? {}) as never },
				createElement(
					QueryClientProvider,
					{ client } as never,
					page
						? createElement(
								ActionProvider,
								{
									surfaceId: "page-1",
									appId: page.appId,
									isPreviewMode: true,
									components: {},
								} as never,
								widgetElement,
							)
						: widgetElement,
				),
			),
		);
	});
	await settle();
}

const frame = () => host.querySelector("iframe");
const frameSrc = () => frame()?.getAttribute("src") ?? null;

/** The URL of every document a frame is given from now on, in order. */
function recordDocuments(): string[] {
	const urls: string[] = [];
	const prototype = window.HTMLIFrameElement.prototype as unknown as {
		setAttribute: (name: string, value: string) => void;
	};
	const setAttribute = prototype.setAttribute;
	Object.defineProperty(prototype, "setAttribute", {
		configurable: true,
		writable: true,
		value(this: unknown, name: string, value: string) {
			if (name === "src") urls.push(value);
			setAttribute.call(this, name, value);
		},
	});
	cleanup.push(() => Reflect.deleteProperty(prototype, "setAttribute"));
	return urls;
}

/**
 * Completes the handshake for the mounted frame and returns how it speaks
 * afterwards. The host only accepts messages whose source is its own iframe
 * window (the wrapper, which relays for the widget).
 */
async function openFrameChannel() {
	const { createEnvelope } = await import("../micro-widget-host");
	const frameWindow = frame()?.contentWindow;
	if (!frameWindow) throw new Error("The widget iframe has no window");
	const posted = spyOn(frameWindow, "postMessage").mockImplementation(() => {});
	cleanup.push(() => posted.mockRestore());
	const fromFrame = (data: unknown) =>
		act(async () => {
			window.dispatchEvent(
				new window.MessageEvent("message", {
					data,
					source: frameWindow as never,
				}),
			);
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

	await fromFrame(createEnvelope("hello", {}, "", "sales-chart"));
	const init = posted.mock.calls
		.map(([envelope]) => envelope as FlwEnvelope)
		.find((envelope) => envelope.type === "init");
	if (!init) throw new Error("The host did not send init to the iframe");
	return (type: "event" | "ready", payload: unknown) =>
		fromFrame(
			createEnvelope(type, payload as never, init.nonce, "sales-chart"),
		);
}
const grantStatus = () =>
	host.querySelector("[data-widget-grant]")?.getAttribute("data-widget-grant");
const bodyText = () => window.document.body.textContent ?? "";

const findButton = (text: string) =>
	Array.from(window.document.body.querySelectorAll("button")).find((button) =>
		button.textContent?.includes(text),
	) ?? null;

interface Attributed {
	getAttribute(name: string): string | null;
}

const isInert = (button: Attributed | null) =>
	button?.getAttribute("aria-disabled") === "true";

async function sleep(ms: number) {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

const armed = (delay: number) =>
	armedTimers.filter((timer) => timer.delay === delay);

/** Fires the newest timer armed for `delay`, as if that time had passed. */
async function runTimer(delay: number) {
	const timer = armed(delay).at(-1);
	if (!timer) throw new Error(`No ${delay} ms timer is armed`);
	await act(async () => timer.run());
	await settle();
}

/** Allow controls ignore activation for a moment after they appear (§14.5.3). */
async function waitUntilActive(button: Attributed | null) {
	for (let tries = 0; tries < 30 && isInert(button); tries++) await sleep(50);
	if (isInert(button)) throw new Error("The button stayed inert");
}

async function click(text: string) {
	const button = findButton(text);
	if (!button) throw new Error(`No button labelled "${text}"`);
	await waitUntilActive(button);
	await act(async () => {
		button.click();
	});
	await settle();
}

async function pressEscape() {
	await act(async () => {
		(window.document.activeElement ?? window.document.body).dispatchEvent(
			new window.KeyboardEvent("keydown", {
				key: "Escape",
				bubbles: true,
			}) as never,
		);
	});
	await settle();
}

const dialogs = () =>
	window.document.body.querySelectorAll('[role="dialog"]').length;
const dialogTitle = () =>
	window.document.body.querySelector("[data-widget-consent-title]");
const activeText = () => window.document.activeElement?.textContent ?? "";
const isPrimary = (text: string) =>
	findButton(text)?.className.includes("bg-primary") ?? false;

function installLocalStorage(): Map<string, string> {
	const items = new Map<string, string>();
	const storage = {
		getItem: (key: string) => items.get(key) ?? null,
		setItem: (key: string, value: string) => void items.set(key, value),
		removeItem: (key: string) => void items.delete(key),
		key: (index: number) => [...items.keys()][index] ?? null,
		get length() {
			return items.size;
		},
	};
	const previous = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
	Object.defineProperty(globalThis, "localStorage", {
		value: storage,
		configurable: true,
	});
	cleanup.push(() => {
		if (previous) Object.defineProperty(globalThis, "localStorage", previous);
		else Reflect.deleteProperty(globalThis, "localStorage");
	});
	return items;
}

const PSL_VERSION = "2026-09-15_10-18-26_UTC";
const LEVELS = ["known", "external", "shared", "broad"] as const;
type Level = (typeof LEVELS)[number];

interface SourceSpec {
	source: string;
	level: Level;
	directives?: string[];
	kind?: string;
	host?: string;
	emphasis?: string;
	provider?: string;
	aboutKey?: string;
	origin?: "declared" | "runtime";
	slot?: string;
}

const maxLevel = (levels: Level[]): Level =>
	LEVELS[Math.max(0, ...levels.map((level) => LEVELS.indexOf(level)))];

/** A descriptor as a classifying backend describes it: policy plus `network`. */
function described(
	purposes: { reason: string; sources: SourceSpec[] }[],
	capabilities: Record<string, boolean> = {},
): Partial<WidgetPolicyDescriptor> {
	const csp: Record<string, string[]> = {};
	const network = purposes.map(({ reason, sources }) => ({
		reason,
		level: maxLevel(sources.map((spec) => spec.level)),
		sources: sources.map((spec) => {
			const directives = spec.directives ?? ["connectSrc"];
			for (const directive of directives)
				csp[directive] = [...(csp[directive] ?? []), spec.source];
			const host =
				spec.host ?? spec.source.slice(spec.source.indexOf("://") + 3);
			return {
				source: spec.source,
				directives,
				origin: spec.origin ?? "declared",
				...(spec.slot ? { slot: spec.slot } : {}),
				kind: spec.kind ?? "exact",
				level: spec.level,
				host,
				emphasis:
					spec.emphasis ??
					host.replace(/^\*\./, "").split(".").slice(-2).join("."),
				...(spec.provider ? { provider: spec.provider } : {}),
				...(spec.aboutKey ? { aboutKey: spec.aboutKey } : {}),
			};
		}),
	}));
	for (const directive of Object.keys(csp)) csp[directive].sort();
	return {
		policy: { ...capabilities, csp },
		network: {
			level: maxLevel(network.map((purpose) => purpose.level)),
			catalogVersion: 1,
			pslVersion: PSL_VERSION,
			stale: false,
			purposes: network,
		} as WidgetPolicyDescriptor["network"],
	};
}

const TILE_SERVER: SourceSpec = { source: MAP_HOST, level: "external" };
const S3_BUCKETS: SourceSpec = {
	source: "https://*.s3.eu-central-1.amazonaws.com",
	level: "broad",
	directives: ["connectSrc", "imgSrc"],
	kind: "shared-wildcard",
	emphasis: "s3.eu-central-1.amazonaws.com",
	provider: "Amazon S3",
	aboutKey: "widgetSourceAboutObjectStorage",
};

describe("micro widget bundle refresh", () => {
	test("retries a failed widget when a synced bundle replaces its old revision", async () => {
		stubRegistry();
		await renderWidget(component({ bundleHash: "old-bundle" }));
		expect(frame() !== null).toBe(true);
		expect(readyTimeout).toBeDefined();
		await act(() => readyTimeout?.());
		expect(frame() === null).toBe(true);
		expect(host.textContent).toContain("did not become ready");

		await renderWidget(
			component({ packageVersion: "1.0.1", bundleHash: "synced-bundle" }),
		);
		expect(frame()).not.toBeNull();
		expect(frameSrc()).toContain("synced-bundle");
		expect(host.textContent).not.toContain("did not become ready");
	});

	test("keeps the live iframe for ordinary input updates", async () => {
		stubRegistry();
		await renderWidget(component());
		const mounted = frame();
		expect(mounted).not.toBeNull();

		await renderWidget(component({ props: { title: "Updated sales" } }));
		expect(frame()).toBe(mounted);
	});
});

describe("micro widget SDK handshake", () => {
	/** Real SDK in its own document; only the wrapper's two postMessage hops are simulated. */
	async function connectSdk(dropHello = false) {
		const { mountFlowWidget } = await import(
			"../../../../widget-sdk/src/mount"
		);
		const wrapper = frame()?.contentWindow;
		if (!wrapper) throw new Error("The widget frame is missing");
		const events = new window.EventTarget();
		const widgetDocument = window.document.implementation.createHTMLDocument();
		widgetDocument.title = "Synthetic widget";
		const outbound: FlwEnvelope[] = [];
		const parent = {
			postMessage(envelope: FlwEnvelope) {
				outbound.push(envelope);
				if (dropHello && envelope.type === "hello") return;
				queueMicrotask(() => {
					window.dispatchEvent(
						new window.MessageEvent("message", {
							data: envelope,
							source: wrapper as never,
						}),
					);
				});
			},
		};
		const widgetWindow = {
			parent,
			self: {},
			top: parent,
			addEventListener: events.addEventListener.bind(events),
			removeEventListener: events.removeEventListener.bind(events),
		};
		const inWidget = <T,>(run: () => T): T => {
			const previousWindow = globalThis.window;
			const previousDocument = globalThis.document;
			Object.assign(globalThis, {
				window: widgetWindow,
				document: widgetDocument,
			});
			try {
				return run();
			} finally {
				Object.assign(globalThis, {
					window: previousWindow,
					document: previousDocument,
				});
			}
		};
		const inbound: FlwEnvelope[] = [];
		const posted = spyOn(wrapper, "postMessage").mockImplementation((data) => {
			const envelope = data as FlwEnvelope;
			inbound.push(envelope);
			queueMicrotask(() =>
				inWidget(() =>
					events.dispatchEvent(
						new window.MessageEvent("message", {
							data: envelope,
							source: parent as never,
						}),
					),
				),
			);
		});
		const bridge = inWidget(() =>
			mountFlowWidget({
				id: "chart",
				name: "Synthetic chart",
				description: "Exercises the widget protocol without loading a package.",
				sizing: { resizable: false },
			}),
		);
		cleanup.push(() => {
			inWidget(() => bridge.dispose());
			posted.mockRestore();
		});
		await settle();
		return { bridge, inbound, outbound };
	}

	test("SDK hello completes the handshake after the outer load event", async () => {
		stubRegistry();
		await renderWidget(component());
		const sdk = await connectSdk();
		expect(sdk.bridge.$mode.get()).toBe("hosted");
		expect(sdk.outbound.map((message) => message.type)).toEqual([
			"hello",
			"ready",
		]);
		expect(host.querySelector('[data-slot="skeleton"]')).toBeNull();
		await act(() => readyTimeout?.());
		expect(bodyText()).not.toContain("did not become ready");
	});

	test("outer load completes the handshake when the SDK hello was missed", async () => {
		stubRegistry();
		await renderWidget(component());
		const sdk = await connectSdk(true);
		expect(sdk.bridge.$mode.get()).toBe("connecting");
		await act(async () =>
			frame()?.dispatchEvent(new window.Event("load") as never),
		);
		await settle();
		expect(sdk.bridge.$mode.get()).toBe("hosted");
		expect(host.querySelector('[data-slot="skeleton"]')).toBeNull();
		await act(() => readyTimeout?.());
		expect(bodyText()).not.toContain("did not become ready");
	});

	test("outer load after ready receives a second SDK ready for its repeated init", async () => {
		stubRegistry();
		await renderWidget(component());
		const sdk = await connectSdk();
		expect(sdk.bridge.$mode.get()).toBe("hosted");
		await act(async () =>
			frame()?.dispatchEvent(new window.Event("load") as never),
		);
		await settle();
		expect(
			sdk.inbound.filter((message) => message.type === "init"),
		).toHaveLength(2);
		expect(
			sdk.outbound.filter((message) => message.type === "ready"),
		).toHaveLength(2);
		expect(host.querySelector('[data-slot="skeleton"]')).toBeNull();
		await act(() => readyTimeout?.());
		expect(bodyText()).not.toContain("did not become ready");
	});
});

describe("micro widget reload in the page builder", () => {
	const CSP_CONTRACT = {
		contractVersion: 2,
		id: "chart",
		csp: [{ reason: "Loads map tiles", connectSrc: [MAP_HOST] }],
	} as unknown as MicroWidgetInstanceComponent["contract"];

	/** Desktop after a local rebuild: the placed bundle was pruned from the widget store. */
	function stubPrunedRegistry() {
		registryState = {
			describeWidgetPolicy: async (request: WidgetPolicyRequest) => {
				throw new Error(
					`Widget bundle ${request.bundleHash} of package '${request.packageId}' is not installed`,
				);
			},
		};
	}

	function stubReloader(installedHash: string) {
		const calls = { reload: [] as string[], refresh: 0 };
		const reloader: MicroWidgetReloader = {
			updateFor: (placed) =>
				placed.bundleHash === installedHash
					? null
					: ({
							packageId: placed.packageId,
							packageName: "Sales",
							packageVersion: placed.packageVersion,
							bundleHash: installedHash,
							widget: {
								id: placed.widgetId,
								name: "Chart",
								description: "",
								icon: null,
								thumbnail: null,
								contract: { contractVersion: 1, id: "chart" },
								keywords: [],
							},
						} satisfies AppPackageWidget),
			reload: async (componentId) => {
				calls.reload.push(componentId);
			},
			refresh: () => {
				calls.refresh++;
			},
		};
		return { reloader, calls };
	}

	test("a pruned bundle offers the installed build and looks for newer builds", async () => {
		stubPrunedRegistry();
		const { reloader, calls } = stubReloader("rebuilt-bundle");
		await renderWidget(
			component({ bundleHash: "pruned-bundle", contract: CSP_CONTRACT }),
			{ router: {} },
			reloader,
		);
		expect(host.textContent).toContain("is not installed");
		expect(calls.refresh).toBeGreaterThan(0);
		const button = findButton("Reload widget");
		expect(button?.hasAttribute("data-builder-interactive")).toBe(true);

		await act(async () => {
			button?.click();
		});
		expect(calls.reload).toEqual(["sales-chart"]);
	});

	test("a widget that never becomes ready offers the installed build", async () => {
		stubRegistry();
		const { reloader } = stubReloader("rebuilt-bundle");
		await renderWidget(
			component({ bundleHash: "pruned-bundle" }),
			undefined,
			reloader,
		);
		expect(findButton("Reload widget")).toBeNull();

		await act(() => readyTimeout?.());
		expect(host.textContent).toContain("did not become ready");
		expect(findButton("Reload widget")).not.toBeNull();
	});

	test.each(["failed", "unsupported"] as const)(
		"%s widgets can update the project and disable both actions during the update",
		async (failure) => {
			if (failure === "failed") {
				stubPrunedRegistry();
			} else {
				registryState = {
					describeWidgetPolicy: async () => {
						throw new Error("404 Not Found");
					},
				};
			}
			const { reloader, calls } = stubReloader("rebuilt-bundle");
			let projectUpdates = 0;
			reloader.reloadAll = async () => {
				projectUpdates++;
			};
			const broken = component({
				bundleHash: "pruned-bundle",
				contract: CSP_CONTRACT,
			});
			await renderWidget(broken, { router: {} }, reloader);
			const updateAll = findButton("Update all");
			expect(updateAll?.hasAttribute("data-builder-interactive")).toBe(true);
			expect(updateAll?.getAttribute("title")).toBe(
				"Update outdated package widgets on every page in this project.",
			);
			await click("Update all");
			expect(projectUpdates).toBe(1);
			expect(calls.reload).toEqual([]);

			await renderWidget(
				broken,
				{ router: {} },
				{
					...reloader,
					isReloadingAll: true,
				},
			);
			expect(findButton("Reload widget")?.disabled).toBe(true);
			expect(findButton("Update all")?.disabled).toBe(true);
			expect(
				findButton("Update all")?.querySelector(".animate-spin"),
			).not.toBeNull();
			await click("Reload widget");
			await click("Update all");
			expect(calls.reload).toEqual([]);
			expect(projectUpdates).toBe(1);
		},
	);

	test("viewers and current builds get no reload control", async () => {
		stubPrunedRegistry();
		const broken = component({
			bundleHash: "pruned-bundle",
			contract: CSP_CONTRACT,
		});
		await renderWidget(broken, { router: {} });
		expect(host.textContent).toContain("is not installed");
		expect(findButton("Reload widget")).toBeNull();
		expect(findButton("Update all")).toBeNull();

		await renderWidget(
			broken,
			{ router: {} },
			stubReloader("pruned-bundle").reloader,
		);
		expect(host.textContent).toContain("is not installed");
		expect(findButton("Reload widget")).toBeNull();
		expect(findButton("Update all")).toBeNull();
	});
});

describe("micro widget event dispatch", () => {
	test("an iframe event payload cannot change the targets of the page actions it starts", async () => {
		stubRegistry();
		const [{ appGlobalState, pageLocalState }, uiState] = await Promise.all([
			import("../../../lib/idb-storage"),
			import("../../../db/ui-state-db"),
		]);
		const opened = spyOn(window, "open").mockImplementation(() => null);
		const spies = [
			spyOn(appGlobalState, "getAll").mockResolvedValue({}),
			spyOn(pageLocalState, "getAll").mockResolvedValue({}),
			spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
			spyOn(uiState.uiElementValues, "set").mockResolvedValue(
				undefined as never,
			),
			spyOn(uiState, "pruneElementValues").mockResolvedValue(),
			spyOn(console, "log").mockImplementation(() => {}),
			opened,
		];
		cleanup.push(() => {
			for (const spy of spies) spy.mockRestore();
		});
		const navigations: string[] = [];
		const router = {
			push: (href: string) => navigations.push(href),
			replace: (href: string) => navigations.push(`replace:${href}`),
			prefetch: () => {},
			back: () => {},
			forward: () => {},
			refresh: () => {},
		};

		await renderWidget(
			component({
				contract: {
					contractVersion: 1,
					id: "chart",
					events: { entityClicked: {} },
				} as MicroWidgetInstanceComponent["contract"],
				eventHandlers: {
					entityClicked: [
						{ name: "navigate_page", context: { route: "/authored" } },
						{
							name: "external_link",
							context: { url: "https://authored.example/docs" },
						},
					],
				},
			}),
			{ router },
		);

		const say = await openFrameChannel();
		await say("event", {
			name: "entityClicked",
			payload: {
				route: "/attacker",
				queryParams: { steal: "1" },
				url: "https://attacker.example/phish",
			},
		});

		expect(navigations).toEqual(["/authored"]);
		expect(opened.mock.calls).toEqual([
			["https://authored.example/docs", "_blank", "noopener,noreferrer"],
		]);
	});
});

describe("micro widget consent and grants", () => {
	const frameUrl = (grant: string) =>
		`/com.example.sales/${BUNDLE_HASH}/frame/chart/${grant}`;

	test("page JSON without capabilities still prompts when the descriptor lists hosts", async () => {
		const calls = stubRegistry(() => ({
			policy: { csp: { connectSrc: [MAP_HOST] } },
		}));
		await renderWidget(
			component({
				contract: {
					contractVersion: 1,
					id: "chart",
					capabilities: {},
				} as MicroWidgetInstanceComponent["contract"],
			}),
			{ router: {} },
		);

		expect(calls.describe).toEqual([
			{
				packageId: "com.example.sales",
				packageVersion: "1.0.0",
				bundleHash: BUNDLE_HASH,
				widgetId: "chart",
				preview: false,
			},
		]);
		expect(frame()).toBeNull();
		expect(grantStatus()).toBe("pending");
		// Without a classification every host is treated as the riskiest level.
		expect(bodyText()).toContain("Widget requests broad network access");
		expect(bodyText()).toContain("Send and receive data");
		expect(bodyText()).toContain("tiles.example.com");
		expect(bodyText()).toContain("Flow-Like could not check who runs");
		expect(bodyText()).toContain("Registry hub.example.com");
		expect(bodyText()).toContain("chart from com.example.sales");
		expect(calls.mint).toEqual([]);
	});

	test("holds the iframe while pending and mounts the minted grant after Allow", async () => {
		const calls = stubRegistry(() => ({
			policy: { downloads: true, csp: { imgSrc: [MAP_HOST] } },
		}));
		await renderWidget(component(), { router: {} });
		expect(frame()).toBeNull();
		expect(findButton("Always allow for this project")).toBeNull();

		await click("Allow this time");
		expect(calls.mint).toEqual([
			{
				packageId: "com.example.sales",
				packageVersion: "1.0.0",
				bundleHash: BUNDLE_HASH,
				widgetId: "chart",
				preview: false,
				policyDigest: DIGEST_A,
			},
		]);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_A));
		expect(frameSrc()).not.toContain("?");
		expect(frame()?.getAttribute("sandbox")).toBe(
			"allow-scripts allow-downloads",
		);
		expect(findButton("Allow this time")).toBeNull();
	});

	test("blocking shows the host count and can run the widget at baseline", async () => {
		const calls = stubRegistry(() => ({
			policy: {
				downloads: true,
				csp: {
					connectSrc: ["https://a.example.com", "wss://a.example.com"],
					imgSrc: ["https://b.example.com"],
				},
			},
		}));
		await renderWidget(component(), { router: {} });

		await click("Don't allow");
		expect(frame()).toBeNull();
		expect(host.textContent).toContain("Blocked network access to 2 sites");
		expect(host.textContent).toContain("Anyone can receive");
		expect(activeText()).toBe("Review permissions");

		await click("Run without these permissions");
		expect(frameSrc()).toEndWith(frameUrl("0"));
		expect(frame()?.getAttribute("sandbox")).toBe("allow-scripts");
		expect(host.textContent).toContain(
			"runs without the permissions you blocked",
		);
		expect(calls.mint).toEqual([]);

		await click("Review permissions");
		expect(frame()).toBeNull();
		expect(findButton("Allow this time")).not.toBeNull();
	});

	test("a project grant is remembered on this device", async () => {
		const consent = await import("../micro-widget-capability-consent");
		const items = installLocalStorage();
		const policy = { microphone: true, csp: { connectSrc: [MAP_HOST] } };
		const calls = stubRegistry(() =>
			described([{ reason: "Loads sales figures", sources: [TILE_SERVER] }], {
				microphone: true,
			}),
		);

		await renderWidget(component(), { router: {}, appId: "app-1" });
		await click("Always allow for this project");
		expect(frameSrc()).toEndWith(frameUrl(GRANT_A));
		const key = `widget-consent:v2:${JSON.stringify([
			SOURCE,
			"app-1",
			"com.example.sales",
			"chart",
		])}`;
		expect([...items.keys()]).toEqual([key]);
		expect(JSON.parse(items.get(key) ?? "null")).toMatchObject({
			v: 2,
			policy,
			levels: { [MAP_HOST]: "external" },
		});

		consent.resetMicroWidgetConsentForTests();
		await act(() => root.unmount());
		root = createRoot(host);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(findButton("Allow this time")).toBeNull();
		expect(frameSrc()).toEndWith(frameUrl(GRANT_A));
		expect(calls.mint).toHaveLength(1);
	});

	test("a stale digest describes again and re-prompts for the new hosts", async () => {
		const extraHost = "https://extra.example.com";
		const calls = stubRegistry(
			(call) =>
				call === 1
					? { policy: { csp: { connectSrc: [MAP_HOST] } } }
					: {
							policy: { csp: { connectSrc: [extraHost, MAP_HOST] } },
							policyDigest: DIGEST_B,
						},
			(request, call) => {
				if (call === 1) throw new WidgetPolicyChangedError();
				return {
					grant: GRANT_B,
					expiresIn: 86_400,
					policyDigest: request.policyDigest,
				};
			},
		);
		await renderWidget(component(), { router: {} });

		await click("Allow this time");
		expect(calls.describe).toHaveLength(2);
		expect(frame()).toBeNull();
		expect(grantStatus()).toBe("pending");
		expect(bodyText()).toContain("extra.example.com");

		await click("Allow this time");
		expect(calls.mint.map((request) => request.policyDigest)).toEqual([
			DIGEST_A,
			DIGEST_B,
		]);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_B));
	});

	test("a digest the backend keeps rejecting ends in an error instead of a loop", async () => {
		const calls = stubRegistry(
			() => ({ policy: { csp: { connectSrc: [MAP_HOST] } } }),
			() => {
				throw new WidgetPolicyChangedError();
			},
		);
		await renderWidget(component(), { router: {} });

		await click("Allow this time");
		expect(calls.mint).toHaveLength(1);
		expect(calls.describe).toHaveLength(2);
		expect(frame()).toBeNull();
		expect(host.textContent).toContain("changed while they were being granted");
	});

	test("an expired grant is re-minted on frame load at most once per minute", async () => {
		const calls = stubRegistry(
			() => ({ policy: { workers: true } }),
			(request, call) => ({
				grant: call === 1 ? GRANT_A : GRANT_B,
				expiresIn: 60,
				policyDigest: request.policyDigest,
			}),
		);
		await renderWidget(component(), { router: {} });
		await click("Allow this time");
		expect(calls.mint.length).toBeGreaterThanOrEqual(1);

		const load = async () => {
			await act(async () => {
				frame()?.dispatchEvent(new window.Event("load") as never);
			});
			await settle();
		};
		await load();
		expect(calls.mint).toHaveLength(2);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_B));

		await load();
		await load();
		expect(calls.mint).toHaveLength(2);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_B));
	});

	test("grants the backend revoked are minted again instead of reused from the cache", async () => {
		const { forgetMicroWidgetGrants } = await import(
			"../use-micro-widget-grant"
		);
		const calls = stubRegistry(
			() => ({ policy: { csp: { connectSrc: [MAP_HOST] } } }),
			(request, call) => ({
				grant: call === 1 ? GRANT_A : GRANT_B,
				expiresIn: 86_400,
				policyDigest: request.policyDigest,
			}),
		);
		const forget = async (packageId: string, widgetId?: string) => {
			await act(async () => forgetMicroWidgetGrants(packageId, widgetId));
			await settle();
		};
		await renderWidget(component(), { router: {} });
		await click("Allow this time");
		expect(frameSrc()).toEndWith(frameUrl(GRANT_A));

		await forget("com.example.other");
		await forget("com.example.sales", "legend");
		expect(calls.mint).toHaveLength(1);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_A));

		await forget("com.example.sales", "chart");
		expect(calls.mint).toHaveLength(2);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_B));

		await act(() => root.unmount());
		await forget("com.example.sales");
		root = createRoot(host);
		await renderWidget(component(), { router: {} });
		expect(findButton("Allow this time")).toBeNull();
		expect(calls.mint).toHaveLength(3);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_B));
	});

	test("a grant that arrives after the backend revoked it is never cached", async () => {
		const { forgetMicroWidgetGrants } = await import(
			"../use-micro-widget-grant"
		);
		let resolveFirst: (response: MintAnswer) => void = () => {};
		const calls = stubRegistry(
			() => ({ policy: { csp: { connectSrc: [MAP_HOST] } } }),
			(request, call) =>
				call === 1
					? new Promise<MintAnswer>((resolve) => {
							resolveFirst = resolve;
						})
					: {
							grant: GRANT_B,
							expiresIn: 86_400,
							policyDigest: request.policyDigest,
						},
		);
		await renderWidget(component(), { router: {} });
		await click("Allow this time");
		expect(calls.mint).toHaveLength(1);
		expect(frame()).toBeNull();

		await act(async () => {
			forgetMicroWidgetGrants("com.example.sales");
			resolveFirst({
				grant: GRANT_A,
				expiresIn: 86_400,
				policyDigest: DIGEST_A,
			});
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		await settle();
		expect(calls.mint).toHaveLength(2);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_B));

		await act(() => root.unmount());
		root = createRoot(host);
		await renderWidget(component(), { router: {} });
		expect(calls.mint).toHaveLength(2);
		expect(frameSrc()).toEndWith(frameUrl(GRANT_B));
	});

	test("the sandbox note never claims the widget cannot reach the network", async () => {
		stubRegistry(() => ({ policy: { downloads: true } }));
		await renderWidget(component(), { router: {} });
		const note = () =>
			window.document.body.querySelector("[data-widget-sandbox-note]")
				?.textContent ?? "";
		expect(bodyText()).toContain("File downloads");
		expect(note()).toBe("");
		await click("Details");
		expect(
			window.document.body.querySelector("[data-widget-local-only]")
				?.textContent,
		).toBe("local data only (no network)");
		expect(note()).toContain("Browsers cannot block every channel");
		expect(note()).toContain("file downloads");
		expect(bodyText()).not.toContain("cannot make network requests");
	});

	test("revoking consent unmounts the widget and prompts again", async () => {
		const consent = await import("../micro-widget-capability-consent");
		stubRegistry(() => ({ policy: { csp: { connectSrc: [MAP_HOST] } } }));
		await renderWidget(component(), { router: {} });
		await click("Allow this time");
		expect(frameSrc()).toEndWith(frameUrl(GRANT_A));

		await act(async () => {
			consent.revokeMicroWidgetConsent({
				source: SOURCE,
				packageId: "com.example.sales",
				widgetId: "chart",
			});
		});
		await settle();
		expect(frame()).toBeNull();
		expect(findButton("Allow this time")).not.toBeNull();
	});

	test("an invalid policy runs at baseline with a notice and no prompt", async () => {
		const calls = stubRegistry(() => ({
			status: "invalid",
			invalidReason: "Widget 'chart': https://hub.example.com is reserved",
			policy: {},
		}));
		await renderWidget(component(), { router: {} });

		expect(findButton("Allow this time")).toBeNull();
		expect(frameSrc()).toEndWith(frameUrl("0"));
		expect(host.textContent).toContain("could not be verified");
		expect(calls.mint).toEqual([]);
	});

	test("an empty policy mounts the baseline frame without a prompt", async () => {
		const calls = stubRegistry();
		await renderWidget(component(), { router: {} });
		expect(findButton("Allow this time")).toBeNull();
		expect(frameSrc()).toEndWith(frameUrl("0"));
		expect(frame()?.getAttribute("sandbox")).toBe("allow-scripts");
		expect(calls.mint).toEqual([]);
	});

	test("a backend that cannot sign grants mounts the approved widget at baseline", async () => {
		stubRegistry(
			() => ({ policy: { csp: { connectSrc: [MAP_HOST] } } }),
			() => {
				throw Object.assign(new Error("Service Unavailable"), { status: 503 });
			},
		);
		await renderWidget(component(), { router: {} });
		await click("Allow this time");
		expect(frameSrc()).toEndWith(frameUrl("0"));
		expect(host.textContent).toContain("cannot grant widget permissions");
	});

	test("a backend without describe falls back to the legacy frame for v1 contracts", async () => {
		await renderWidget(
			component({
				contract: {
					contractVersion: 1,
					id: "chart",
					capabilities: { downloads: true },
				} as MicroWidgetInstanceComponent["contract"],
			}),
			{ router: {} },
		);
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("Package registry");

		await click("Allow this time");
		expect(frameSrc()).toEndWith(
			`/com.example.sales/${BUNDLE_HASH}/frame/chart?downloads=1`,
		);
		expect(frame()?.getAttribute("sandbox")).toBe(
			"allow-scripts allow-downloads",
		);
	});

	test("a contract with csp needs a newer server when describe fails", async () => {
		registryState = {
			describeWidgetPolicy: async () => {
				throw new Error("404 Not Found");
			},
		};
		await renderWidget(
			component({
				contract: {
					contractVersion: 2,
					id: "chart",
					csp: [
						{
							reason: "Loads map tiles from the example tile server",
							connectSrc: [MAP_HOST],
						},
					],
				} as unknown as MicroWidgetInstanceComponent["contract"],
			}),
			{ router: {} },
		);
		expect(frame()).toBeNull();
		expect(findButton("Allow this time")).toBeNull();
		expect(host.textContent).toContain("needs a newer server");
		expect(host.textContent).toContain("404 Not Found");
	});
});

const ACCESS = "eyJhbGciOiJFUzI1NiJ9.eyJwa2ciOiJ4In0.YWNjZXNz";
const WEB_GRANT = "eyJhbGciOiJFUzI1NiJ9.eyJncmFudCI6IngifQ.Z3JhbnQ";
const HOUR = 3_600_000;
const sandboxPath = (access: string, grant = "0") =>
	`/registry/package/com.example.sales/widget-sandbox/1.0.0/~${access}/frame/chart/${grant}`;

/** The web API's grants: each mint answers a new one, `${WEB_GRANT}1`, `${WEB_GRANT}2`, …, living an hour unless `lifetime` says otherwise; mint number `failing` fails. */
const webGrants =
	({
		lifetime = 3_600,
		failing,
	}: { lifetime?: number; failing?: number } = {}) =>
	(request: WidgetGrantRequest, call: number): MintAnswer => {
		if (call === failing) throw new TypeError("Failed to fetch");
		return {
			grant: `${WEB_GRANT}${call}`,
			expiresIn: lifetime,
			policyDigest: request.policyDigest,
		};
	};

function onTheWeb(
	access: (
		request: WidgetAccessRequest,
		call: number,
	) => Promise<WidgetAccessResponse>,
	registry: () => RegistryCalls = stubRegistry,
) {
	Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
	const calls = registry();
	const accessCalls: WidgetAccessRequest[] = [];
	(registryState as Record<string, unknown>).getWidgetAccess = (
		request: WidgetAccessRequest,
	) => {
		accessCalls.push(request);
		return access(request, accessCalls.length);
	};
	return { calls, accessCalls };
}

/** Each call answers a new token, `${ACCESS}1`, `${ACCESS}2`, …, living 12 h unless `expiresIn` says otherwise. */
const numberedTokens = (
	expiresIn: (call: number) => number = () => 43_200,
	registry?: () => RegistryCalls,
) =>
	onTheWeb(
		async (_request, call) => ({
			access: `${ACCESS}${call}`,
			expiresIn: expiresIn(call),
		}),
		registry,
	);

/** Moves `Date.now()` forward like a device asleep; the monotonic clock does not see it. */
function shiftWallClock() {
	const wallNow = Date.now.bind(Date);
	let shift = 0;
	const spy = spyOn(Date, "now").mockImplementation(() => wallNow() + shift);
	cleanup.push(() => spy.mockRestore());
	return (ms: number) => {
		shift += ms;
	};
}

/** Moves both clocks forward, like time passing on a device that stays awake. */
function passTime() {
	const sleep = shiftWallClock();
	const monotonicNow = performance.now.bind(performance);
	let passed = 0;
	const spy = spyOn(performance, "now").mockImplementation(
		() => monotonicNow() + passed,
	);
	cleanup.push(() => spy.mockRestore());
	return (ms: number) => {
		sleep(ms);
		passed += ms;
	};
}

/** Reports `state` as the page's visibility and announces the change, like a tab switch or a wake from sleep. */
async function setVisibility(state: "visible" | "hidden") {
	Object.defineProperty(window.document, "visibilityState", {
		configurable: true,
		get: () => state,
	});
	await act(async () => {
		window.document.dispatchEvent(
			new window.Event("visibilitychange") as never,
		);
	});
	await settle();
}

describe("micro widget web sandbox access", () => {
	/** A widget that declares a host, so its frame needs a real grant. */
	const grantingRegistry = (grants = webGrants()) =>
		stubRegistry(
			() => ({ policy: { csp: { connectSrc: [MAP_HOST] } } }),
			grants,
		);

	/** The inline page runtime moves its portal host between a card slot and its parking slot, which reloads the frame. */
	async function moveHost() {
		const slot = window.document.createElement("div");
		window.document.body.appendChild(slot);
		await act(async () => {
			slot.appendChild(host as never);
		});
		await settle();
	}

	async function remount() {
		await act(() => root.unmount());
		root = createRoot(host);
		await renderWidget(component(), { router: {}, appId: "app-1" });
	}

	const RETRY_DELAYS = [2_000, 5_000, 15_000];
	const CHECK_INTERVAL = 60_000;
	const READY_TIMEOUT = 10_000;
	const failure = (status: number, message: string) =>
		Object.assign(new Error(message), { status });
	const skeleton = () => host.querySelector('[data-slot="skeleton"]');
	/** Document URLs from the registry on, as `sandboxPath` writes them. */
	const paths = (urls: string[]) =>
		urls.map((url) => url.slice(url.indexOf("/registry/")));

	/** The replacement is refused, then the four attempts of the next check fail without a ruling. */
	const refusalThenOutage = (afterwards: () => WidgetAccessResponse) =>
		onTheWeb(async (_request, call) => {
			if (call === 1) return { access: `${ACCESS}1`, expiresIn: 43_200 };
			if (call === 2) throw failure(403, "Forbidden");
			if (call <= 6) throw new TypeError("Failed to fetch");
			return afterwards();
		});

	/** Iframes mounted from now on never fire `load`, like a document whose request stalls. */
	function stallFrameLoads() {
		const prototype = window.HTMLIFrameElement.prototype as unknown as {
			dispatchEvent: (event: { type: string }) => boolean;
		};
		const dispatch = prototype.dispatchEvent;
		Object.defineProperty(prototype, "dispatchEvent", {
			configurable: true,
			writable: true,
			value(this: unknown, event: { type: string }) {
				return event.type === "load" || dispatch.call(this, event);
			},
		});
		cleanup.push(() => Reflect.deleteProperty(prototype, "dispatchEvent"));
	}

	test("the frame carries the viewer's access, asked for through the project", async () => {
		const { calls, accessCalls } = onTheWeb(async () => ({
			access: ACCESS,
			expiresIn: 43_200,
		}));
		await renderWidget(component(), { router: {}, appId: "app-1" });

		expect(calls.describe[0]?.appId).toBe("app-1");
		expect(accessCalls).toEqual([
			{
				packageId: "com.example.sales",
				packageVersion: "1.0.0",
				appId: "app-1",
			},
		]);
		expect(frameSrc()).toEndWith(
			`/registry/package/com.example.sales/widget-sandbox/1.0.0/~${ACCESS}/frame/chart/0`,
		);
	});

	test("a public package loads anonymously", async () => {
		onTheWeb(async () => ({ access: null, expiresIn: 43_200 }));
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(frameSrc()).toEndWith(
			"/registry/package/com.example.sales/widget-sandbox/1.0.0/frame/chart/0",
		);
	});

	test("a viewer without access sees why instead of waiting for the ready timeout", async () => {
		const { accessCalls } = onTheWeb(async () => {
			throw failure(403, "You have no access to this package");
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("could not be opened");
		expect(bodyText()).toContain("You have no access to this package");
		expect(accessCalls).toHaveLength(1);
		expect(RETRY_DELAYS.flatMap(armed)).toEqual([]);
	});

	test("a request that failed without a refusal is asked again while the frame keeps loading", async () => {
		const { accessCalls } = onTheWeb(async (_request, call) => {
			if (call === 1) throw failure(503, "Service Unavailable");
			return { access: ACCESS, expiresIn: 43_200 };
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(accessCalls).toHaveLength(1);
		expect(frame()).toBeNull();
		expect(skeleton()).not.toBeNull();
		expect(bodyText()).not.toContain("could not be opened");

		await runTimer(RETRY_DELAYS[0]);
		expect(accessCalls).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(ACCESS));
	});

	test("one failed request does not end the other widgets of the package", async () => {
		const { accessCalls } = onTheWeb(async (_request, call) => {
			if (call === 1) throw new TypeError("Failed to fetch");
			return { access: null, expiresIn: 43_200 };
		});
		await renderWidget(
			[component(), component({ id: "second", instanceId: "second" })],
			{ router: {}, appId: "app-1" },
		);
		expect(accessCalls).toHaveLength(1);
		expect(host.querySelectorAll("iframe")).toHaveLength(0);

		for (const timer of armed(RETRY_DELAYS[0])) {
			await act(async () => timer.run());
		}
		await settle();
		expect(accessCalls).toHaveLength(2);
		expect(host.querySelectorAll("iframe")).toHaveLength(2);
		expect(bodyText()).not.toContain("could not be opened");
	});

	test("the failure shows once every attempt failed", async () => {
		const { accessCalls } = onTheWeb(async () => {
			throw new TypeError("Failed to fetch");
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		for (const delay of RETRY_DELAYS) {
			expect(frame()).toBeNull();
			expect(bodyText()).not.toContain("could not be opened");
			await runTimer(delay);
		}
		expect(accessCalls).toHaveLength(RETRY_DELAYS.length + 1);
		expect(bodyText()).toContain("could not be opened");
		expect(bodyText()).toContain("Failed to fetch");
		expect(RETRY_DELAYS.flatMap(armed)).toEqual([]);
	});

	test("a token replacement that fails is asked again instead of ending the frame", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = onTheWeb(async (_request, call) => {
			if (call === 2) throw failure(502, "Bad Gateway");
			return { access: `${ACCESS}${call}`, expiresIn: 43_200 };
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		elapse(12 * HOUR);
		await moveHost();
		expect(accessCalls).toHaveLength(2);
		expect(frame()).toBeNull();
		expect(bodyText()).not.toContain("could not be opened");

		await runTimer(RETRY_DELAYS[0]);
		expect(accessCalls).toHaveLength(3);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}3`));
	});

	test("an unmounted frame stops asking and stops checking its token", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = onTheWeb(async (_request, call) => {
			if (call !== 2) throw failure(503, "Service Unavailable");
			return { access: `${ACCESS}${call}`, expiresIn: 43_200 };
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(armed(RETRY_DELAYS[0])).toHaveLength(1);
		await act(() => root.unmount());
		expect(armed(RETRY_DELAYS[0])).toEqual([]);

		const listened = spyOn(window.document, "addEventListener");
		const unlistened = spyOn(window.document, "removeEventListener");
		cleanup.push(() => {
			listened.mockRestore();
			unlistened.mockRestore();
		});
		const visibilityListeners = (spy: typeof listened) =>
			spy.mock.calls
				.filter(([type]) => type === "visibilitychange")
				.map(([, listener]) => listener);
		root = createRoot(host);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
		expect(armed(CHECK_INTERVAL)).toHaveLength(1);
		expect(visibilityListeners(listened)).toHaveLength(1);

		elapse(12 * HOUR);
		await setVisibility("visible");
		expect(accessCalls).toHaveLength(3);
		expect(armed(RETRY_DELAYS[0])).toHaveLength(1);
		await act(() => root.unmount());
		expect(armed(RETRY_DELAYS[0])).toEqual([]);
		expect(armed(CHECK_INTERVAL)).toEqual([]);
		expect(visibilityListeners(unlistened)).toEqual(
			visibilityListeners(listened),
		);

		root = createRoot(host);
		elapse(60_000);
		await setVisibility("visible");
		expect(accessCalls).toHaveLength(3);
	});

	test("a baseline frame keeps a healthy token and gets a fresh one when it reloads past the deadline", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}1`));

		elapse(6 * HOUR);
		await moveHost();
		expect(frame()).toBe(built);
		expect(accessCalls).toHaveLength(1);

		elapse(6 * HOUR);
		await moveHost();
		expect(accessCalls).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
		expect(bodyText()).not.toContain("did not become ready");
	});

	test("an expired token is replaced on frame load at most once per minute", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens((call) =>
			call === 1 ? 43_200 : 60,
		);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		elapse(12 * HOUR);
		await moveHost();
		expect(accessCalls).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));

		await moveHost();
		expect(accessCalls).toHaveLength(2);

		elapse(60_000);
		await moveHost();
		expect(accessCalls).toHaveLength(3);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}3`));
	});

	test("a new frame reuses a token only while half its lifetime is left, counting time asleep", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });

		elapse(5 * HOUR);
		await remount();
		expect(accessCalls).toHaveLength(1);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}1`));

		elapse(2 * HOUR);
		await remount();
		expect(accessCalls).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
	});

	test("a frame that failed with its token makes later mounts fetch a fresh one", async () => {
		const { accessCalls } = numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });
		await remount();
		expect(accessCalls).toHaveLength(1);

		await act(() => readyTimeout?.());
		expect(bodyText()).toContain("did not become ready");

		await remount();
		expect(accessCalls).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
	});

	test("a mounted frame replaces a token past its deadline without loading again", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();

		elapse(11 * HOUR + 54 * 60_000);
		await runTimer(CHECK_INTERVAL);
		expect(frame()).toBe(built);
		expect(accessCalls).toHaveLength(1);

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(frame()).not.toBe(built);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
		expect(bodyText()).not.toContain("did not become ready");
	});

	test("a granted frame that slept past its grant gets a fresh grant with its new token", async () => {
		const elapse = shiftWallClock();
		const { calls, accessCalls } = numberedTokens(undefined, grantingRegistry);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		await click("Allow this time");
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}1`, `${WEB_GRANT}1`));

		elapse(14 * HOUR);
		await setVisibility("visible");
		expect(accessCalls).toHaveLength(2);
		expect(calls.mint).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`, `${WEB_GRANT}2`));
		expect(bodyText()).not.toContain("could not be opened");
	});

	test("a running frame past both deadlines loads one new document, with its new token and a new grant", async () => {
		const elapse = passTime();
		const documents = recordDocuments();
		const { calls, accessCalls } = numberedTokens(undefined, grantingRegistry);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		await click("Allow this time");
		const say = await openFrameChannel();
		await say("ready", {});

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(calls.mint).toHaveLength(2);
		expect(paths(documents)).toEqual([
			sandboxPath(`${ACCESS}1`, `${WEB_GRANT}1`),
			sandboxPath(`${ACCESS}2`, `${WEB_GRANT}2`),
		]);
	});

	test("a running frame whose grant is within its deadline gets its new token in place", async () => {
		const elapse = shiftWallClock();
		const documents = recordDocuments();
		const { calls } = numberedTokens(undefined, () =>
			grantingRegistry(webGrants({ lifetime: 86_400 })),
		);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		await click("Allow this time");

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(calls.mint).toHaveLength(1);
		expect(paths(documents)).toEqual([
			sandboxPath(`${ACCESS}1`, `${WEB_GRANT}1`),
			sandboxPath(`${ACCESS}2`, `${WEB_GRANT}1`),
		]);
	});

	test("a grant renewal that fails leaves the running frame, and the next check renews it", async () => {
		const elapse = passTime();
		const documents = recordDocuments();
		const { calls, accessCalls } = numberedTokens(undefined, () =>
			grantingRegistry(webGrants({ failing: 2 })),
		);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		await click("Allow this time");
		const built = frame();
		const say = await openFrameChannel();
		await say("ready", {});

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(calls.mint).toHaveLength(2);
		expect(frame()).toBe(built);
		expect(skeleton()).toBeNull();
		expect(bodyText()).not.toContain("could not be granted");

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(calls.mint).toHaveLength(3);
		expect(paths(documents)).toEqual([
			sandboxPath(`${ACCESS}1`, `${WEB_GRANT}1`),
			sandboxPath(`${ACCESS}2`, `${WEB_GRANT}3`),
		]);
	});

	test("a hidden page keeps its frame until it is visible again", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();

		await setVisibility("hidden");
		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		await setVisibility("hidden");
		expect(frame()).toBe(built);
		expect(accessCalls).toHaveLength(1);

		await setVisibility("visible");
		expect(accessCalls).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
	});

	test("a healthy token survives the page becoming visible", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();

		elapse(11 * HOUR);
		await setVisibility("hidden");
		await setVisibility("visible");
		expect(frame()).toBe(built);
		expect(accessCalls).toHaveLength(1);
	});

	test("a mounted frame replaces its token at most once per minute", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens((call) =>
			call === 1 ? 43_200 : 60,
		);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		elapse(12 * HOUR);
		await setVisibility("visible");
		expect(accessCalls).toHaveLength(2);

		await setVisibility("visible");
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(3);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}3`));
	});

	test("a running widget stays until its replacement token arrived", async () => {
		const elapse = shiftWallClock();
		let answer: (response: WidgetAccessResponse) => void = () => {};
		const { accessCalls } = onTheWeb((_request, call) =>
			call === 1
				? Promise.resolve({ access: `${ACCESS}1`, expiresIn: 43_200 })
				: new Promise((resolve) => {
						answer = resolve;
					}),
		);
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();
		const say = await openFrameChannel();
		await say("ready", {});
		expect(skeleton()).toBeNull();

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(frame()).toBe(built);
		expect(skeleton()).toBeNull();

		await act(async () => answer({ access: `${ACCESS}2`, expiresIn: 43_200 }));
		await settle();
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
		expect(accessCalls).toHaveLength(2);
		expect(skeleton()).not.toBeNull();
	});

	test("a replacement document that never loads ends in the ready timeout", async () => {
		const elapse = shiftWallClock();
		numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const say = await openFrameChannel();
		await say("ready", {});
		expect(skeleton()).toBeNull();
		expect(armed(READY_TIMEOUT)).toEqual([]);

		stallFrameLoads();
		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`));
		expect(skeleton()).not.toBeNull();
		expect(frame()?.className).toContain("opacity-0");

		await runTimer(READY_TIMEOUT);
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("did not become ready");
	});

	test("a frame that failed stays failed when its token is replaced", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = numberedTokens();
		await renderWidget(component(), { router: {}, appId: "app-1" });
		await runTimer(READY_TIMEOUT);
		expect(bodyText()).toContain("did not become ready");

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("did not become ready");
	});

	test("a replacement that fails leaves the running frame for the next check", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = onTheWeb(async (_request, call) => {
			if (call > 1 && call <= 5) throw new TypeError("Failed to fetch");
			return { access: `${ACCESS}${call}`, expiresIn: 43_200 };
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();

		elapse(12 * HOUR);
		await setVisibility("visible");
		for (const delay of RETRY_DELAYS) await runTimer(delay);
		expect(accessCalls).toHaveLength(5);
		expect(frame()).toBe(built);
		expect(bodyText()).not.toContain("could not be opened");

		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(5);

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(6);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}6`));
	});

	test("a refused replacement keeps the running frame, and the next check asks again in place", async () => {
		const elapse = shiftWallClock();
		let answer: (response: WidgetAccessResponse) => void = () => {};
		const { accessCalls } = onTheWeb((_request, call) => {
			if (call === 1) {
				return Promise.resolve({ access: `${ACCESS}1`, expiresIn: 43_200 });
			}
			if (call === 2) return Promise.reject(failure(403, "Forbidden"));
			return new Promise((resolve) => {
				answer = resolve;
			});
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();
		const say = await openFrameChannel();
		await say("ready", {});

		elapse(12 * HOUR);
		await setVisibility("visible");
		expect(accessCalls).toHaveLength(2);
		expect(frame()).toBe(built);
		expect(bodyText()).not.toContain("could not be opened");
		expect(RETRY_DELAYS.flatMap(armed)).toEqual([]);

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(3);
		expect(frame()).toBe(built);
		expect(skeleton()).toBeNull();

		await act(async () => answer({ access: `${ACCESS}3`, expiresIn: 43_200 }));
		await settle();
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}3`));
	});

	test("an outage after a refusal leaves the running frame, which recovers once the backend answers", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = refusalThenOutage(() => ({
			access: `${ACCESS}7`,
			expiresIn: 43_200,
		}));
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();

		elapse(12 * HOUR);
		await setVisibility("visible");
		expect(accessCalls).toHaveLength(2);

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		for (const delay of RETRY_DELAYS) await runTimer(delay);
		expect(accessCalls).toHaveLength(6);
		expect(frame()).toBe(built);
		expect(bodyText()).not.toContain("could not be opened");

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(7);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}7`));
	});

	test("a second refusal ends the frame even with an outage in between", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = refusalThenOutage(() => {
			throw failure(403, "You have no access to this package");
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });

		elapse(12 * HOUR);
		await setVisibility("visible");
		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		for (const delay of RETRY_DELAYS) await runTimer(delay);
		expect(frame()).not.toBeNull();

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(7);
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("You have no access to this package");
	});

	test("a refusal is a repeat only until a replacement arrived", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = onTheWeb(async (_request, call) => {
			if (call === 2 || call >= 4) throw failure(403, "Forbidden");
			return { access: `${ACCESS}${call}`, expiresIn: 43_200 };
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}3`));
		const replaced = frame();

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(4);
		expect(frame()).toBe(replaced);
		expect(bodyText()).not.toContain("could not be opened");

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(5);
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("could not be opened");
	});

	test("a viewer who lost access sees why once the refusal repeats", async () => {
		const elapse = shiftWallClock();
		const { accessCalls } = onTheWeb(async (_request, call) => {
			if (call === 1) return { access: `${ACCESS}1`, expiresIn: 43_200 };
			throw failure(403, "You have no access to this package");
		});
		await renderWidget(component(), { router: {}, appId: "app-1" });
		const built = frame();

		elapse(12 * HOUR);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(2);
		expect(frame()).toBe(built);

		elapse(60_000);
		await runTimer(CHECK_INTERVAL);
		expect(accessCalls).toHaveLength(3);
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("could not be opened");
		expect(bodyText()).toContain("You have no access to this package");
		expect(armed(CHECK_INTERVAL)).toEqual([]);
	});

	test("frames that load anonymously never check for a token", async () => {
		onTheWeb(async () => ({ access: null, expiresIn: 43_200 }));
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(frame()).not.toBeNull();
		expect(armed(CHECK_INTERVAL)).toEqual([]);

		await act(() => root.unmount());
		root = createRoot(host);
		Object.assign(window, { __TAURI_INTERNALS__: {} });
		await renderWidget(component(), { router: {}, appId: "app-1" });
		expect(frame()).not.toBeNull();
		expect(armed(CHECK_INTERVAL)).toEqual([]);
	});
});

describe("micro widget runtime sources", () => {
	const TILE_A = "https://a.tiles.example.org";
	const TILE_B = "https://b.tiles.example.org";
	const SLOT = { path: "layers[].url", purpose: 0, directives: ["imgSrc"] };
	const RUNTIME_DIGEST = `sha256:${"d".repeat(64)}`;
	const declaredPolicy = { csp: { connectSrc: [MAP_HOST] } };
	const tiles = (...hosts: string[]) => ({
		title: "Sales",
		layers: hosts.map((host) => ({ url: `${host}/1/2/3.png?sig=secret` })),
	});

	function stubRuntimeRegistry(
		runtime: { level?: Level; kind?: string } = {},
		mint: Parameters<typeof stubRegistry>[1] = (request, call) => ({
			grant: call === 1 ? GRANT_A : GRANT_B,
			expiresIn: 86_400,
			policyDigest: request.policyDigest,
		}),
	) {
		const runtimeLevel = runtime.level ?? "external";
		return stubRegistry(
			(_call, request) => {
				const sources = request.runtimeSources?.flatMap(
					(entry) => entry.sources,
				);
				const base = {
					networkInputs: [SLOT] as WidgetPolicyDescriptor["networkInputs"],
				};
				if (!sources?.length) {
					return {
						...base,
						...described([
							{
								reason: "Loads map tiles given to it at runtime",
								sources: [TILE_SERVER],
							},
						]),
						runtime: { status: "none", declaredDigest: DIGEST_A, rejected: [] },
					};
				}
				const classify = (
					source: string,
					directives: string[],
					origin: string,
				) => {
					const host = new URL(source).host;
					return {
						source,
						directives,
						origin,
						...(origin === "runtime" ? { slot: SLOT.path } : {}),
						kind: origin === "runtime" ? (runtime.kind ?? "exact") : "exact",
						level: origin === "runtime" ? runtimeLevel : "external",
						host,
						emphasis: host.split(".").slice(-2).join("."),
					};
				};
				return {
					...base,
					policy: { csp: { ...declaredPolicy.csp, imgSrc: sources } },
					policyDigest:
						sources.length > 1 ? DIGEST_B : `sha256:${"e".repeat(64)}`,
					network: {
						level: maxLevel(["external", runtimeLevel]),
						catalogVersion: 1,
						pslVersion: "2026-09-15_10-18-26_UTC",
						stale: false,
						purposes: [
							{
								reason: "Loads map tiles given to it at runtime",
								level: maxLevel(["external", runtimeLevel]),
								inputs: [SLOT.path],
								sources: [
									classify(MAP_HOST, ["connectSrc"], "declared"),
									...sources.map((source) =>
										classify(source, ["imgSrc"], "runtime"),
									),
								],
							},
						],
					} as WidgetPolicyDescriptor["network"],
					runtime: {
						status: "ok",
						declaredDigest: DIGEST_A,
						runtimeDigest: RUNTIME_DIGEST,
						rejected: [],
					},
				};
			},
			(request, call) => mint(request, call),
		);
	}

	async function waitForDebounce() {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 320));
		});
		await settle();
	}

	test("the prompt lists the origins from the props and Allow mints with exactly them", async () => {
		const calls = stubRuntimeRegistry();
		await renderWidget(component({ props: tiles(TILE_A) }), {
			router: {},
			appId: "app-1",
		});

		expect(calls.describe).toHaveLength(2);
		expect(calls.describe[1]).toMatchObject({
			appId: "app-1",
			runtimeSources: [{ slot: SLOT.path, sources: [TILE_A] }],
		});
		expect(frame()).toBeNull();
		expect(bodyText()).toContain("a.tiles.example.org");
		expect(bodyText()).not.toContain("sig=secret");
		for (const chip of window.document.body.querySelectorAll(
			"[data-widget-source-runtime]",
		)) {
			const group = chip.closest("[data-widget-runtime-sources]");
			expect(
				group?.firstElementChild?.hasAttribute("data-widget-runtime-heading"),
			).toBe(true);
			expect(group?.firstElementChild?.textContent).toContain(
				"Provided while the app runs",
			);
			expect(group?.firstElementChild?.textContent).toContain(SLOT.path);
		}
		expect(
			window.document.body.querySelectorAll("[data-widget-source-runtime]"),
		).toHaveLength(1);

		await click("Allow this time");
		expect(calls.mint).toHaveLength(1);
		expect(calls.mint[0]).toMatchObject({
			policyDigest: `sha256:${"e".repeat(64)}`,
			appId: "app-1",
			runtimeSources: [{ slot: SLOT.path, sources: [TILE_A] }],
		});
		expect(frameSrc()).toEndWith(
			`/com.example.sales/${BUNDLE_HASH}/frame/chart/${GRANT_A}`,
		);
	});

	test("a new address while the widget runs keeps the frame and asks through the banner", async () => {
		const calls = stubRuntimeRegistry();
		await renderWidget(component({ props: tiles(TILE_A) }), {
			router: {},
			appId: "app-1",
		});
		await click("Allow this time");
		const mounted = frame();
		expect(frameSrc()).toEndWith(GRANT_A);

		await renderWidget(component({ props: tiles(TILE_A, TILE_B) }), {
			router: {},
			appId: "app-1",
		});
		await waitForDebounce();
		expect(calls.describe.at(-1)?.runtimeSources).toEqual([
			{ slot: SLOT.path, sources: [TILE_A, TILE_B] },
		]);
		expect(frame()).toBe(mounted);
		expect(findButton("Allow this time")).toBeNull();
		expect(host.textContent).toContain("Wants to load from 1 new site");

		await click("Review");
		expect(bodyText()).toContain("Widget wants to load from new sites");
		expect(bodyText()).toContain("b.tiles.example.org");
		expect(bodyText()).toContain("Already allowed: 2 sites");
		expect(bodyText()).toContain("Allowing reloads the widget.");
		expect(findButton("Stop asking for this widget")).not.toBeNull();
		await click("Allow this time");
		expect(calls.mint.at(-1)).toMatchObject({
			policyDigest: DIGEST_B,
			runtimeSources: [{ slot: SLOT.path, sources: [TILE_A, TILE_B] }],
		});
		expect(frameSrc()).toEndWith(GRANT_B);
		expect(host.textContent).not.toContain("Wants to load from");
	});

	test("an unchecked runtime part dims its chips and titles the dialog from the declared part", async () => {
		const calls = stubRuntimeRegistry({ level: "broad", kind: "shared-host" });
		await renderWidget(component({ props: tiles(TILE_A) }), {
			router: {},
			appId: "app-1",
		});
		const checkbox = () =>
			window.document.body.querySelector("[data-widget-runtime-checkbox]");
		const runtimeChip = () =>
			window.document.body.querySelector("[data-widget-source-runtime]");
		const toggle = async () => {
			await act(async () => {
				(checkbox() as unknown as HTMLElement).click();
			});
			await settle();
		};

		expect(bodyText()).toContain(
			"Also allow the 1 address provided while the app runs",
		);
		expect(checkbox()?.getAttribute("aria-checked")).toBe("false");
		expect(runtimeChip()?.getAttribute("data-widget-source-included")).toBe(
			"false",
		);
		expect(runtimeChip()?.className).toContain("opacity-50");
		expect(dialogTitle()?.textContent).toBe("Widget requests network access");
		expect(isPrimary("Allow this time")).toBe(true);

		await toggle();
		expect(checkbox()?.getAttribute("aria-checked")).toBe("true");
		expect(runtimeChip()?.getAttribute("data-widget-source-included")).toBe(
			"true",
		);
		expect(dialogTitle()?.textContent).toBe(
			"Widget requests broad network access",
		);
		expect(isPrimary("Don't allow")).toBe(true);

		await toggle();
		await click("Allow this time");
		expect(calls.mint).toHaveLength(1);
		expect(calls.mint[0].policyDigest).toBe(DIGEST_A);
		expect(calls.mint[0].runtimeSources).toBeUndefined();
	});

	test("an unchecked runtime box answers for every instance sharing the dialog", async () => {
		const calls = stubRuntimeRegistry();
		await renderWidget(
			[
				component({ props: tiles(TILE_A) }),
				component({ id: "copy", instanceId: "copy", props: tiles(TILE_A) }),
			],
			{ router: {}, appId: "app-1" },
		);
		expect(dialogs()).toBe(1);
		expect(bodyText()).not.toContain("1 of 2");

		await act(async () => {
			(
				window.document.body.querySelector(
					"[data-widget-runtime-checkbox]",
				) as unknown as HTMLElement
			).click();
		});
		await click("Allow this time");
		expect(dialogs()).toBe(0);
		expect(host.querySelectorAll("iframe")).toHaveLength(2);
		expect(calls.mint.map((call) => call.runtimeSources)).toEqual([
			undefined,
			undefined,
		]);
	});

	test("Always allow on a runtime review stores only the addresses it listed", async () => {
		const items = installLocalStorage();
		stubRuntimeRegistry();
		await renderWidget(component({ props: tiles(TILE_A) }), {
			router: {},
			appId: "app-1",
		});
		await click("Allow this time");
		expect([...items.keys()]).toEqual([]);

		await renderWidget(component({ props: tiles(TILE_A, TILE_B) }), {
			router: {},
			appId: "app-1",
		});
		await waitForDebounce();
		await click("Review");
		expect(bodyText()).toContain("Already allowed: 2 sites");
		await click("Always allow for this project");
		expect(frameSrc()).toEndWith(GRANT_B);
		const [stored, ...rest] = [...items.values()].map((value) =>
			JSON.parse(value),
		);
		expect(rest).toEqual([]);
		expect(stored.policy).toEqual({});
		expect(stored.levels).toEqual({});
		expect(stored.runtime.map((entry: { s: string }) => entry.s)).toEqual([
			TILE_B,
		]);
	});

	test("newer addresses never swap in silently; Show them does", async () => {
		stubRuntimeRegistry();
		await renderWidget(component({ props: tiles(TILE_A) }), {
			router: {},
			appId: "app-1",
		});
		expect(bodyText()).toContain("a.tiles.example.org");

		await renderWidget(component({ props: tiles(TILE_A, TILE_B) }), {
			router: {},
			appId: "app-1",
		});
		await waitForDebounce();
		expect(bodyText()).toContain(
			"The widget received different addresses while this was open.",
		);
		expect(bodyText()).not.toContain("b.tiles.example.org");
		await waitUntilActive(findButton("Allow this time"));

		await click("Show them");
		expect(bodyText()).toContain("b.tiles.example.org");
		expect(bodyText()).not.toContain("received different addresses");
		expect(window.document.activeElement === dialogTitle()).toBe(true);
		expect(isInert(findButton("Allow this time"))).toBe(true);
	});

	test("dismissing the banner keeps the frame and hides the request until its backoff timer fires", async () => {
		stubRuntimeRegistry();
		await renderWidget(component({ props: tiles(TILE_A) }), {
			router: {},
			appId: "app-1",
		});
		await click("Allow this time");
		const mounted = frame();

		await renderWidget(component({ props: tiles(TILE_A, TILE_B) }), {
			router: {},
			appId: "app-1",
		});
		await waitForDebounce();
		const banner = host.querySelector("[data-widget-runtime-request]");
		expect(banner?.textContent).toContain("Wants to load from 1 new site");
		expect(isInert(findButton("Review"))).toBe(true);

		await act(async () => {
			(
				banner?.querySelector('button[aria-label="Close"]') as unknown as
					| HTMLElement
					| undefined
			)?.click();
		});
		await settle();
		expect(host.querySelector("[data-widget-runtime-request]")).toBeNull();
		expect(frame()).toBe(mounted);
		expect(dialogs()).toBe(0);

		await runTimer(30_000);
		expect(
			host.querySelector("[data-widget-runtime-request]")?.textContent,
		).toContain("Wants to load from 1 new site");
		expect(frame()).toBe(mounted);
	});

	test("a frame past its grant's deadline gets a new grant with its new token while a new address waits for review", async () => {
		const elapse = shiftWallClock();
		const documents = recordDocuments();
		const { calls, accessCalls } = numberedTokens(undefined, () =>
			stubRuntimeRegistry({}, webGrants()),
		);
		await renderWidget(component({ props: tiles(TILE_A) }), {
			router: {},
			appId: "app-1",
		});
		await click("Allow this time");
		await renderWidget(component({ props: tiles(TILE_A, TILE_B) }), {
			router: {},
			appId: "app-1",
		});
		await waitForDebounce();
		const banner = () => host.querySelector("[data-widget-runtime-request]");
		expect(banner()?.textContent).toContain("Wants to load from 1 new site");
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}1`, `${WEB_GRANT}1`));

		elapse(12 * HOUR);
		await setVisibility("visible");
		expect(accessCalls).toHaveLength(2);
		expect(calls.mint).toHaveLength(2);
		expect(calls.mint[1].runtimeSources).toEqual([
			{ slot: SLOT.path, sources: [TILE_A] },
		]);
		expect(frameSrc()).toEndWith(sandboxPath(`${ACCESS}2`, `${WEB_GRANT}2`));
		expect(documents).toHaveLength(2);
		expect(banner()?.textContent).toContain("Wants to load from 1 new site");
	});

	test("skipped addresses list hosts and reasons, never URLs", async () => {
		stubRegistry((_call, request) => ({
			networkInputs: [SLOT] as WidgetPolicyDescriptor["networkInputs"],
			...described([
				{
					reason: "Loads map tiles given to it at runtime",
					sources: [TILE_SERVER],
				},
			]),
			runtime: {
				status: "none",
				declaredDigest: DIGEST_A,
				rejected: request.runtimeSources?.length
					? [
							{
								slot: SLOT.path,
								source: "https://10-0-0-1.sslip.io",
								code: "reserved-name",
							},
						]
					: [],
			},
		}));
		await renderWidget(
			component({
				props: {
					layers: [
						{ url: "https://10-0-0-1.sslip.io/private/tile.png?token=secret" },
						{ url: "http://insecure.example.com/tile.png" },
					],
				},
			}),
			{ router: {}, appId: "app-1" },
		);
		await click("Allow this time");
		const notice = host.querySelector("[data-widget-notice]");
		expect(notice?.getAttribute("data-widget-notice")).toBe("runtime-skipped");
		expect(notice?.textContent).toContain(
			"2 addresses given to this widget can't be allowed",
		);

		await click("Details");
		const details =
			window.document.body.querySelector("[data-widget-skipped-details]")
				?.textContent ?? "";
		expect(details).toContain(
			"10-0-0-1.sslip.io: points at Flow-Like itself or a local-only name",
		);
		expect(details).toContain("1 × not a valid https or wss address");
		for (const secret of ["://", "token", "private", "insecure"])
			expect(details).not.toContain(secret);
	});
});

describe("micro widget consent dialog", () => {
	const figures = (sources: SourceSpec[], capabilities = {}) =>
		stubRegistry(() =>
			described(
				[{ reason: "Loads sales figures from the tile server", sources }],
				capabilities,
			),
		);

	test("calm levels focus the title and allow for the project directly", async () => {
		installLocalStorage();
		const calls = figures([TILE_SERVER], { workers: true });
		await renderWidget(component(), { router: {}, appId: "app-1" });

		expect(dialogTitle()?.textContent).toBe("Widget requests network access");
		expect(window.document.activeElement === dialogTitle()).toBe(true);
		const labels = Array.from(
			window.document.body.querySelectorAll(
				"[data-widget-consent-footer] button",
			),
		).map((button) => button.textContent);
		expect(labels).toEqual([
			"Don't allow",
			"Allow this time",
			"Always allow for this project",
		]);
		expect(isPrimary("Allow this time")).toBe(true);
		expect(isPrimary("Don't allow")).toBe(false);
		const banner = window.document.body.querySelector(
			"[data-widget-consent-banner]",
		);
		expect(banner?.getAttribute("data-widget-consent-banner")).toBe("external");
		expect(banner?.textContent).toBe(
			"Flow-Like cannot verify who receives what this widget sends to example.com. Anything the widget can see, including values this app gives it, can leave your device.",
		);
		expect(bodyText()).toContain("External site");
		expect(bodyText()).toContain("Publisher: Loads sales figures");
		expect(bodyText()).toContain("Also asks for");
		expect(bodyText()).toContain("Background workers");

		await click("Always allow for this project");
		expect(calls.mint).toHaveLength(1);
		expect(frameSrc()).toEndWith(
			`/com.example.sales/${BUNDLE_HASH}/frame/chart/${GRANT_A}`,
		);
	});

	test("broad focuses Don't allow and confirms Always allow inline", async () => {
		const items = installLocalStorage();
		const calls = stubRegistry(() =>
			described([
				{
					reason: "Loads map layers from Amazon S3 buckets in Frankfurt",
					sources: [S3_BUCKETS],
				},
			]),
		);
		await renderWidget(component(), { router: {}, appId: "app-1" });

		expect(dialogTitle()?.textContent).toBe(
			"Widget requests broad network access",
		);
		expect(activeText()).toBe("Don't allow");
		expect(isPrimary("Don't allow")).toBe(true);
		expect(isPrimary("Allow this time")).toBe(false);
		const banner = window.document.body.querySelector(
			"[data-widget-consent-banner]",
		);
		expect(banner?.getAttribute("data-widget-consent-banner")).toBe("broad");
		expect(banner?.textContent).toContain(
			"Anyone who signs up with Amazon S3 could receive what this widget sends.",
		);
		expect(bodyText()).toContain("any address under");
		expect(bodyText()).toContain("Anyone can receive");

		await click("Always allow for this project…");
		expect(calls.mint).toEqual([]);
		expect(bodyText()).toContain(
			"anyone who signs up with Amazon S3 could receive what it sends",
		);
		expect(activeText()).toBe("Cancel");
		expect(isInert(findButton("Always allow"))).toBe(true);

		await pressEscape();
		expect(dialogs()).toBe(1);
		expect(activeText()).toContain("Always allow for this project");
		expect(calls.mint).toEqual([]);

		await click("Always allow for this project…");
		await click("Always allow");
		expect(calls.mint).toHaveLength(1);
		expect(items.size).toBe(1);
		expect(dialogs()).toBe(0);
	});

	test("Allow ignores activation during the inert window", async () => {
		const calls = figures([TILE_SERVER]);
		await renderWidget(component(), { router: {} });
		const allow = findButton("Allow this time");
		expect(isInert(allow)).toBe(true);
		await act(async () => {
			allow?.click();
		});
		await settle();
		expect(calls.mint).toEqual([]);

		await waitUntilActive(allow);
		await act(async () => {
			window.dispatchEvent(new window.Event("focus") as never);
		});
		expect(isInert(allow)).toBe(true);
		await act(async () => {
			allow?.click();
		});
		await settle();
		expect(calls.mint).toEqual([]);

		await click("Allow this time");
		expect(calls.mint).toHaveLength(1);
	});

	test("Escape means Don't allow and focus returns to Review", async () => {
		figures([TILE_SERVER]);
		await renderWidget(component(), { router: {} });
		await act(async () => {
			host.dispatchEvent(
				new window.PointerEvent("pointerdown", { bubbles: true }) as never,
			);
		});
		await settle();
		expect(dialogs()).toBe(1);

		await pressEscape();
		expect(dialogs()).toBe(0);
		expect(host.textContent).toContain("Blocked network access to 1 site");
		expect(host.textContent).toContain("External site");
		expect(activeText()).toBe("Review permissions");
	});

	test("a trailing wss address outside the catalog stays visible while Details is closed", async () => {
		const known = (source: string, provider: string): SourceSpec => ({
			source,
			level: "known",
			kind: "service",
			provider,
		});
		figures([
			known("https://a.tile.openstreetmap.org", "OpenStreetMap"),
			known("https://b.tile.openstreetmap.org", "OpenStreetMap"),
			known("https://c.tile.openstreetmap.org", "OpenStreetMap"),
			known("https://fonts.googleapis.com", "Google Fonts"),
			known("https://fonts.gstatic.com", "Google Fonts"),
			known("https://tile.openstreetmap.org", "OpenStreetMap"),
			{ source: "wss://relay.evil-attacker.net", level: "external" },
		]);
		await renderWidget(component(), { router: {} });

		const details = window.document.body.querySelector(
			"[data-widget-consent-details]",
		);
		expect(details?.hasAttribute("hidden")).toBe(true);
		expect(details?.textContent).toBe("");
		const chips = Array.from(
			window.document.body.querySelectorAll("[data-widget-source-chip]"),
		).map((chip) => chip.textContent);
		expect(chips).toContain("relay.evil-attacker.net");
		expect(chips).toHaveLength(4);
		expect(findButton("+3 more")?.getAttribute("aria-expanded")).toBe("false");
		expect(bodyText()).toContain(
			"Flow-Like cannot verify who receives what this widget sends to evil-attacker.net.",
		);

		await click("+3 more");
		expect(
			window.document.body.querySelectorAll("[data-widget-source-chip]"),
		).toHaveLength(7);
	});

	test("one dialog per page; the other widget waits its turn", async () => {
		const calls = figures([TILE_SERVER]);
		await renderWidget(
			[
				component(),
				component({ id: "legend", instanceId: "legend", widgetId: "legend" }),
			],
			{ router: {} },
		);
		expect(dialogs()).toBe(1);
		expect(bodyText()).toContain("1 of 2");
		expect(host.querySelectorAll("[data-widget-queued]")).toHaveLength(1);
		expect(host.textContent).toContain("Waiting for your permission");

		await click("Allow this time");
		expect(calls.mint).toHaveLength(1);
		expect(dialogs()).toBe(1);
		expect(bodyText()).not.toContain("1 of 2");
		expect(host.querySelectorAll("[data-widget-queued]")).toHaveLength(0);

		await click("Allow this time");
		expect(calls.mint).toHaveLength(2);
		expect(host.querySelectorAll("iframe")).toHaveLength(2);
	});
});
