import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type {
	IBlockedPackage,
	IForkPreviewResponse,
} from "../../../lib/schema/app/fork";
import type {
	RequestAccessResponse,
	WasmPurchaseResponse,
} from "../../../lib/schema/wasm";

/** The first dynamic import transpiles the component graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 60_000;
const BUYER_PERMISSION = 8;

const PAID: IBlockedPackage = {
	package_id: "chart-kit",
	name: "Chart Kit",
	block: "paid",
	price: 499,
	request_pending: false,
};
const ON_REQUEST: IBlockedPackage = {
	package_id: "team-utils",
	name: "Team Utils",
	block: "request_access",
	price: 0,
	request_pending: false,
};
const PRIVATE: IBlockedPackage = {
	package_id: "internal-nodes",
	name: "internal-nodes",
	block: "private",
	price: 0,
	request_pending: false,
};

interface RenderOptions {
	purchasePackage?: (packageId: string) => Promise<WasmPurchaseResponse>;
	requestAccess?: (packageId: string) => Promise<RequestAccessResponse>;
	/** Answers hub GETs by path; unanswered paths resolve to null. */
	apiGet?: (path: string) => unknown;
	apiPost?: (path: string) => unknown;
	hub?: "legacy" | "marketplace" | "pending" | "failing";
	/** How many hub config requests get an answer; later ones never resolve. */
	hubAnswers?: number;
	onAccessChanged?: () => void;
	onCheckoutPendingChange?: (
		packageId: string,
		pending: boolean,
		browserCheckout?: boolean,
	) => void;
	/** What the host remembers of each open purchase, by package id. */
	pendingCheckouts?: ReadonlyMap<string, { since?: number }>;
	context?: "fork" | "copy";
	disabled?: boolean;
	seed?: (client: QueryClient) => void;
}

let window: Window;
let root: Root;
let host: HTMLElement;
let client: QueryClient;
let restoreGlobals: () => void;
let restoreBackend: (() => void) | undefined;
let hubDown = false;

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/store" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		HTMLAnchorElement: window.HTMLAnchorElement,
		HTMLIFrameElement: window.HTMLIFrameElement,
		Element: window.Element,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		ShadowRoot: window.ShadowRoot,
		DOMRect: window.DOMRect,
		Range: window.Range,
		Selection: window.Selection,
		HTMLCollection: window.HTMLCollection,
		Node: window.Node,
		NodeList: window.NodeList,
		MutationObserver: window.MutationObserver,
		IntersectionObserver: window.IntersectionObserver,
		ResizeObserver: window.ResizeObserver,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		KeyboardEvent: window.KeyboardEvent,
		NodeFilter: window.NodeFilter,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
		fetch: globalThis.fetch,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	// A browser checkout opens its page with window.open; happy-dom would load it.
	Object.assign(window, { SyntaxError, TypeError, open: () => null });
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
	const { useAuthStatusStore } = await import("../../../state/backend-state");
	await act(() => root.unmount());
	useAuthStatusStore.setState({ signedIn: undefined });
	restoreBackend?.();
	restoreBackend = undefined;
	await window.happyDOM.abort();
	restoreGlobals();
});

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

/**
 * The hub root answers with its payment config. `pending` never answers,
 * `failing` answers 500 until a test flips `hubDown`, and only the first
 * `answers` requests get an answer at all.
 */
function stubHub(
	hub: RenderOptions["hub"] = "legacy",
	answers = Number.POSITIVE_INFINITY,
) {
	hubDown = hub === "failing";
	let asked = 0;
	globalThis.fetch = (async () => {
		asked += 1;
		if (hub === "pending" || asked > answers)
			return new Promise<Response>(() => {});
		if (hubDown) return new Response("", { status: 500 });
		return new Response(
			JSON.stringify({
				payments: { marketplace_enabled: hub === "marketplace" },
			}),
			{ status: 200, headers: { "content-type": "application/json" } },
		);
	}) as unknown as typeof fetch;
}

/** Installs the hub, the backend and the query client a test renders against. */
async function prepare(options: RenderOptions) {
	const { QueryClient } = await import("@tanstack/react-query");
	const { useBackendStore } = await import("../../../state/backend-state");

	stubHub(options.hub, options.hubAnswers);
	const previous = useBackendStore.getState().backend;
	restoreBackend = () => useBackendStore.setState({ backend: previous });
	useBackendStore.getState().setBackend({
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "hub" } }),
			getProfile: async () => ({ id: "profile", hub: "hub.test" }),
			getPATs: async () => [],
		},
		apiState: {
			get: async (_profile: unknown, path: string) =>
				options.apiGet?.(path) ?? null,
			post: async (_profile: unknown, path: string) =>
				options.apiPost?.(path) ?? null,
		},
		registryState: {
			purchasePackage: options.purchasePackage,
			requestAccess: options.requestAccess,
		},
	} as never);
	client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	options.seed?.(client);
}

/** Renders `element` inside the providers the package rows need. */
async function show(element: ReactNode) {
	const { act } = await import("react");
	const { QueryClientProvider } = await import("@tanstack/react-query");
	const { AuthContext } = await import("react-oidc-context");
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<AuthContext.Provider
					value={{ isAuthenticated: true, user: null } as never}
				>
					{element}
				</AuthContext.Provider>
			</QueryClientProvider>,
		),
	);
	await flush(40);
}

async function render(
	packages: IBlockedPackage[],
	options: RenderOptions = {},
) {
	const { ForkBlockedPackages } = await import("./fork-blocked-packages");
	await prepare(options);
	await show(
		<ForkBlockedPackages
			packages={packages}
			onAccessChanged={options.onAccessChanged ?? (() => {})}
			onCheckoutPendingChange={options.onCheckoutPendingChange}
			pendingCheckouts={options.pendingCheckouts}
			context={options.context}
			disabled={options.disabled}
		/>,
	);
}

const PREVIEW: IForkPreviewResponse = {
	source_app_id: "source",
	total_size_bytes: 2048,
	total_object_count: 4,
	max_size_bytes: 1_000_000,
	max_file_count: 1000,
	within_limits: true,
	fork_policy: {
		flows: true,
		files: true,
		databases: "with_data",
		roles: true,
		widgets: true,
		templates: true,
	},
	size_breakdown: {
		always: { bytes: 1024, objects: 2 },
		flows: { bytes: 1024, objects: 2 },
		files: { bytes: 0, objects: 0 },
		databases: { bytes: 0, objects: 0 },
		widgets: { bytes: 0, objects: 0 },
		templates: { bytes: 0, objects: 0 },
	},
	selected_size_bytes: 2048,
	selected_object_count: 4,
	requires_token: false,
	remote_token_sites: [],
	allow_forking: true,
	user_can_fork: true,
	disallow_reason: "",
};

/** The fork dialog for a source app whose preview `loadPreview` answers. */
async function showDialog(
	open: boolean,
	loadPreview: () => Promise<IForkPreviewResponse>,
) {
	const { ForkAppDialog } = await import("./fork-app-dialog");
	await show(
		<ForkAppDialog
			appId="source"
			appName="CRM"
			open={open}
			onOpenChange={() => {}}
			target="online"
			loadPreview={loadPreview}
			beginFork={() => Promise.reject(new Error("not under test"))}
		/>,
	);
}

/** Buttons of the row list and of dialogs portalled into the body. */
function button(label: string) {
	return Array.from(window.document.body.querySelectorAll("button")).find(
		(candidate) => candidate.textContent?.includes(label),
	) as unknown as HTMLButtonElement | undefined;
}

async function clickElement(target: unknown) {
	const { act } = await import("react");
	await act(async () => {
		(target as HTMLElement).dispatchEvent(
			new window.MouseEvent("click", { bubbles: true }) as never,
		);
	});
	await flush();
}

async function click(label: string) {
	const target = button(label);
	if (!target) throw new Error(`No "${label}" button`);
	if (target.hasAttribute("disabled"))
		throw new Error(`The "${label}" button is disabled`);
	await clickElement(target);
}

/** The browser checkouts a host remembers, each started `minutesAgo`. */
function remembered(minutesAgo: number, ...packageIds: string[]) {
	const since = Date.now() - minutesAgo * 60_000;
	return new Map(packageIds.map((id) => [id, { since }]));
}

/** The marketplace orders a host remembers. They carry no start time. */
function rememberedOrders(...packageIds: string[]) {
	return new Map(packageIds.map((id) => [id, {}]));
}

/** Collects the success toasts shown until `restore` is called. */
async function captureSuccessToasts() {
	const { toast } = await import("sonner");
	const messages: string[] = [];
	const original = toast.success;
	toast.success = ((message: string) => {
		messages.push(message);
		return 0;
	}) as typeof toast.success;
	return {
		messages,
		restore: () => {
			toast.success = original;
		},
	};
}

/**
 * Replaces the window's intervals so a test fires their ticks by hand. Ids are
 * handed out from 1, so `cleared` tells which interval was stopped.
 */
function captureIntervals() {
	const intervals: { id: number; ms: number; tick: () => void }[] = [];
	const cleared: number[] = [];
	Object.assign(window, {
		setInterval: (tick: () => void, ms: number) => {
			const id = intervals.length + 1;
			intervals.push({ id, ms, tick });
			return id;
		},
		clearInterval: (id: number) => cleared.push(id),
		open: () => null,
	});
	/** The newest interval armed with that delay. */
	const latest = (ms: number) => {
		const interval = intervals
			.filter((candidate) => candidate.ms === ms)
			.at(-1);
		if (!interval) throw new Error(`No ${ms} ms interval was armed`);
		return interval;
	};
	return { latest, cleared };
}

/** Records the delay of every timeout the window is asked for. They still run. */
function captureTimeouts() {
	const delays: number[] = [];
	const schedule = window.setTimeout.bind(window);
	Object.assign(window, {
		setTimeout: (handler: () => void, ms = 0) => {
			delays.push(ms);
			return schedule(handler, ms);
		},
	});
	return delays;
}

async function fire(interval: { tick: () => void }) {
	const { act } = await import("react");
	await act(async () => interval.tick());
	await flush();
}

function openOrder() {
	return {
		orderId: "order-1",
		appId: "",
		itemKind: "PACKAGE",
		itemId: "chart-kit",
		itemName: "Chart Kit",
		status: "OPEN",
		amount: 499,
		currency: "eur",
		checkoutUrl: "https://pay.example/session",
		refundedAmount: 0,
		pendingRefundAmount: 0,
		withdrawable: false,
	};
}

/** Hub GETs of a marketplace checkout: terms, the order as it is now, the registry entry. */
function marketplaceGet(order: { status: string }, permission = () => 0) {
	return (path: string) => {
		if (path.startsWith("payments/terms"))
			return {
				kind: "PURCHASE_TERMS",
				version: "1",
				locale: "en",
				text: "Terms",
				hash: "terms-hash",
			};
		if (path.startsWith("user/purchases/")) return { ...order };
		if (path.startsWith("registry/package/"))
			return {
				visibility: "public",
				price: 499,
				currentUserPermission: permission(),
			};
		return null;
	};
}

function checkoutDialog() {
	return window.document.body.querySelector('[role="dialog"]');
}

async function prepareCheckout() {
	await clickElement(window.document.body.querySelector('[role="checkbox"]'));
	await click("Prepare checkout");
}

async function closeCheckout() {
	await clickElement(
		window.document.body.querySelector('[data-slot="dialog-close"]'),
	);
}

/**
 * Runs one tick of the checkout dialog's order poll. Refuses to stand in for a
 * poll the dialog has not armed.
 */
async function pollOrder() {
	const { act } = await import("react");
	const polled = client
		.getQueryCache()
		.findAll({ queryKey: ["payments"] })
		.filter(
			(query) =>
				query.queryKey.at(-1) === "user/purchases/order-1" &&
				query.observers.some(
					(observer) =>
						observer.options.enabled === true &&
						!!observer.options.refetchInterval,
				),
		);
	if (polled.length === 0)
		throw new Error("The checkout dialog is not polling its order");
	await act(async () => {
		await Promise.all(polled.map((query) => query.fetch()));
	});
	await flush();
}

/** A checkout the hub refuses with that code. */
async function refusedCheckout(status: number, code: string) {
	const { ApiResponseError } = await import("../../../lib/api-error");
	return () => {
		throw new ApiResponseError({ status, code, message: `Refused: ${code}` });
	};
}

describe("ForkBlockedPackages", () => {
	test(
		"offers a purchase, an access request or says the package is left out",
		async () => {
			await render([PAID, ON_REQUEST, PRIVATE]);

			const text = host.textContent ?? "";
			expect(text).toContain("3 packages won't come with your copy");
			expect(button("Buy")?.textContent).toContain("4.99");
			expect(button("Buy")?.hasAttribute("disabled")).toBe(false);
			expect(button("Request access")).toBeDefined();
			expect(text).toContain("This package is private");
			expect(text).toContain("Left out");
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test("a queued access request marks the package requested", async () => {
		const requested: string[] = [];
		let reloads = 0;
		await render([ON_REQUEST], {
			requestAccess: async (packageId) => {
				requested.push(packageId);
				return {
					granted: false,
					queued: true,
					requiresPurchase: false,
					packageId,
				};
			},
			onAccessChanged: () => {
				reloads += 1;
			},
		});

		await click("Request access");

		expect(requested).toEqual(["team-utils"]);
		expect(button("Request access")).toBeUndefined();
		expect(host.textContent).toContain("Requested");
		expect(reloads).toBe(0);
	});

	test("granted access reloads the preview", async () => {
		let reloads = 0;
		await render([ON_REQUEST], {
			requestAccess: async (packageId) => ({
				granted: true,
				queued: false,
				requiresPurchase: false,
				packageId,
			}),
			onAccessChanged: () => {
				reloads += 1;
			},
		});

		await click("Request access");

		expect(reloads).toBe(1);
	});

	test("buying a package the forker already holds reloads the preview", async () => {
		const purchased: string[] = [];
		let reloads = 0;
		await render([PAID], {
			purchasePackage: async (packageId) => {
				purchased.push(packageId);
				return { alreadyHasAccess: true, packageId };
			},
			onAccessChanged: () => {
				reloads += 1;
			},
		});

		await click("Buy");

		expect(purchased).toEqual(["chart-kit"]);
		expect(reloads).toBe(1);
		expect(button("Buy")).toBeUndefined();
		expect(host.textContent).toContain("Purchased");
	});

	test("a browser checkout polls until access lands, then reloads the preview once", async () => {
		const { CHECKOUT_POLL_MS } = await import(
			"../../store/use-package-store-data"
		);
		const { latest, cleared } = captureIntervals();
		const pending: [string, boolean][] = [];
		const inBrowser: (boolean | undefined)[] = [];
		let permission = 0;
		let reloads = 0;
		await render([PAID], {
			purchasePackage: async (packageId) => ({
				alreadyHasAccess: false,
				checkoutUrl: "https://pay.example/session",
				packageId,
			}),
			apiGet: () => ({
				visibility: "public",
				price: 499,
				currentUserPermission: permission,
			}),
			onAccessChanged: () => {
				reloads += 1;
			},
			onCheckoutPendingChange: (packageId, isPending, browserCheckout) => {
				pending.push([packageId, isPending]);
				if (isPending) inBrowser.push(browserCheckout);
			},
		});

		await click("Buy");

		expect(button("Waiting for payment")).toBeDefined();
		expect(pending.at(-1)).toEqual(["chart-kit", true]);
		expect(inBrowser).toEqual([true]);
		const poll = latest(CHECKOUT_POLL_MS);

		await fire(poll);
		expect(reloads).toBe(0);

		permission = BUYER_PERMISSION;
		await fire(poll);
		expect(reloads).toBe(1);
		expect(host.textContent).toContain("Purchased");
		expect(pending.at(-1)).toEqual(["chart-kit", false]);
		expect(cleared).toContain(poll.id);

		// A tick that was already in flight must not reload a second time.
		await fire(poll);
		expect(reloads).toBe(1);
	});

	test("a checkout the host still remembers resumes after the rows remount", async () => {
		const { CHECKOUT_POLL_MS } = await import(
			"../../store/use-package-store-data"
		);
		const { latest, cleared } = captureIntervals();
		await render([PAID], { pendingCheckouts: remembered(2, "chart-kit") });

		expect(button("Buy")).toBeUndefined();
		expect(button("Waiting for payment")).toBeDefined();
		// The poll is re-armed as its inputs load; the newest one keeps running.
		expect(cleared).not.toContain(latest(CHECKOUT_POLL_MS).id);
	});

	test("a remembered browser checkout is followed for the time it has left", async () => {
		const { checkoutWindowLeft } = await import(
			"../../store/use-package-store-data"
		);
		const wholeWindow = checkoutWindowLeft();
		const delays = captureTimeouts();
		const started = Date.now();
		await render([PAID], { pendingCheckouts: remembered(2, "chart-kit") });
		// The row armed its timer somewhere between the start and now.
		const elapsed = Date.now() - started;

		expect(button("Waiting for payment")).toBeDefined();
		const left = wholeWindow - 2 * 60_000;
		expect(delays.some((ms) => ms >= left - elapsed && ms <= left)).toBe(true);
		expect(delays).not.toContain(wholeWindow);
	});

	test("a remembered order is followed for the whole time again", async () => {
		const { checkoutWindowLeft } = await import(
			"../../store/use-package-store-data"
		);
		const delays = captureTimeouts();
		await render([PAID], {
			hub: "marketplace",
			pendingCheckouts: rememberedOrders("chart-kit"),
			apiGet: marketplaceGet(openOrder()),
		});

		expect(button("Continue checkout")).toBeDefined();
		expect(delays).toContain(checkoutWindowLeft());
	});

	test("a remembered checkout whose time ran out is offered again, not followed afresh", async () => {
		const pending: [string, boolean][] = [];
		await render([PAID], {
			pendingCheckouts: remembered(11, "chart-kit"),
			onCheckoutPendingChange: (packageId, isPending) => {
				pending.push([packageId, isPending]);
			},
		});

		expect(button("Waiting for payment")).toBeUndefined();
		expect(button("Buy")?.hasAttribute("disabled")).toBe(false);
		expect(pending.at(-1)).toEqual(["chart-kit", false]);
	});

	test("on a marketplace hub a remembered order can be continued", async () => {
		await render([PAID], {
			hub: "marketplace",
			pendingCheckouts: rememberedOrders("chart-kit"),
			apiGet: marketplaceGet(openOrder()),
		});
		expect(button("Buy")).toBeUndefined();

		await click("Continue checkout");

		expect(checkoutDialog()).not.toBeNull();
		expect(button("Prepare checkout")).toBeDefined();
	});

	test("a remembered order shows its own action while the hub's payment config is still loading", async () => {
		await render([PAID], {
			hub: "pending",
			pendingCheckouts: rememberedOrders("chart-kit"),
		});

		expect(button("Waiting for payment")).toBeUndefined();
		expect(button("Continue checkout")?.hasAttribute("disabled")).toBe(true);
	});

	test("the host times a browser checkout, and leaves an order to the hub", async () => {
		const { act } = await import("react");
		const { usePendingCheckouts } = await import("./fork-blocked-packages");
		let kept: ReturnType<typeof usePendingCheckouts> | undefined;
		function Keeper() {
			kept = usePendingCheckouts();
			return null;
		}
		await act(async () => root.render(<Keeper />));
		const before = Date.now();

		await act(async () => {
			kept?.handleCheckoutPendingChange("browser", true, true);
			kept?.handleCheckoutPendingChange("order", true, false);
		});

		const since = kept?.pendingCheckouts.get("browser")?.since;
		expect(since).toBeGreaterThanOrEqual(before);
		expect(kept?.pendingCheckouts.get("order")).toEqual({ since: undefined });

		// A row that is remounted reports again, later; the first report stands.
		await flush();
		await act(async () => {
			kept?.handleCheckoutPendingChange("browser", true, true);
			kept?.handleCheckoutPendingChange("order", false);
		});

		expect(kept?.pendingCheckouts.get("browser")?.since).toBe(since);
		expect(kept?.pendingCheckouts.has("order")).toBe(false);
	});

	test("Buy stays disabled until the hub's payment config is known", async () => {
		await render([PAID], { hub: "pending" });

		expect(button("Buy")?.hasAttribute("disabled")).toBe(true);
	});

	test("a failed payment config request is repeated until Buy unlocks", async () => {
		const { CONFIG_RETRY_MS } = await import("./fork-blocked-packages");
		const { latest, cleared } = captureIntervals();
		await render([PAID], { hub: "failing" });
		expect(button("Buy")?.hasAttribute("disabled")).toBe(true);
		const retry = latest(CONFIG_RETRY_MS);

		hubDown = false;
		await fire(retry);
		await flush(40);

		expect(button("Buy")?.hasAttribute("disabled")).toBe(false);
		expect(cleared).toContain(retry.id);
	});

	test("the checkout dialog does not wait for a hub answer of its own", async () => {
		// The list asks the hub first and gets the only answer; the dialog, which
		// mounts after it, never hears back.
		await render([PAID], {
			hub: "marketplace",
			hubAnswers: 1,
			apiGet: marketplaceGet(openOrder()),
		});

		await click("Buy");

		expect(button("Prepare checkout")).toBeDefined();
		expect(checkoutDialog()?.textContent).not.toContain(
			"Purchasing is unavailable",
		);
	});

	test("a marketplace order stays tracked after its checkout dialog is closed", async () => {
		const pending: [string, boolean][] = [];
		const inBrowser: (boolean | undefined)[] = [];
		const legacy: string[] = [];
		const order = openOrder();
		let reloads = 0;
		await render([PAID], {
			hub: "marketplace",
			purchasePackage: async (packageId) => {
				legacy.push(packageId);
				return { alreadyHasAccess: false, packageId };
			},
			apiGet: marketplaceGet(order),
			apiPost: () => ({ ...order }),
			onAccessChanged: () => {
				reloads += 1;
			},
			onCheckoutPendingChange: (packageId, isPending, browserCheckout) => {
				pending.push([packageId, isPending]);
				if (isPending) inBrowser.push(browserCheckout);
			},
		});

		await click("Buy");
		expect(legacy).toEqual([]);
		await prepareCheckout();

		expect(pending.at(-1)).toEqual(["chart-kit", true]);
		expect(inBrowser).toEqual([false]);
		expect(button("Continue checkout")).toBeDefined();

		await closeCheckout();
		expect(checkoutDialog()).toBeNull();
		expect(button("Continue checkout")).toBeDefined();
		expect(button("Buy")).toBeUndefined();

		order.status = "PAID";
		await pollOrder();

		expect(reloads).toBe(1);
		expect(host.textContent).toContain("Purchased");
		expect(pending.at(-1)).toEqual(["chart-kit", false]);
	});

	test("an order that ends unpaid can be retried with a fresh checkout", async () => {
		const pending: [string, boolean][] = [];
		const order = openOrder();
		await render([PAID], {
			hub: "marketplace",
			apiGet: marketplaceGet(order),
			apiPost: () => ({ ...order }),
			onCheckoutPendingChange: (packageId, isPending) => {
				pending.push([packageId, isPending]);
			},
		});
		await click("Buy");
		await prepareCheckout();
		await closeCheckout();
		expect(button("Continue checkout")).toBeDefined();

		order.status = "CANCELED";
		await pollOrder();

		expect(pending.at(-1)).toEqual(["chart-kit", false]);
		expect(button("Continue checkout")).toBeUndefined();

		await click("Buy");

		// The dialog that held the cancelled order was replaced while closed.
		expect(checkoutDialog()?.textContent).not.toContain("order-1");
		expect(button("Prepare checkout")).toBeDefined();
	});

	test("a checkout the hub refuses as already owned reloads the preview", async () => {
		let reloads = 0;
		await render([PAID], {
			hub: "marketplace",
			apiGet: marketplaceGet(openOrder()),
			apiPost: await refusedCheckout(409, "ALREADY_OWNED"),
			onAccessChanged: () => {
				reloads += 1;
			},
		});

		await click("Buy");
		await prepareCheckout();

		// The registry still says no, so the preview has to decide.
		expect(reloads).toBe(1);
		expect(host.textContent).not.toContain("Purchased");
	});

	test("an already owned package the registry confirms settles as purchased", async () => {
		let permission = 0;
		let reloads = 0;
		await render([PAID], {
			hub: "marketplace",
			apiGet: marketplaceGet(openOrder(), () => permission),
			apiPost: await refusedCheckout(409, "ALREADY_OWNED"),
			onAccessChanged: () => {
				reloads += 1;
			},
		});

		await click("Buy");
		permission = BUYER_PERMISSION;
		await prepareCheckout();

		expect(host.textContent).toContain("Purchased");
		expect(checkoutDialog()).toBeNull();
		expect(reloads).toBe(1);
	});

	// The package was withdrawn from sale or deleted after the preview loaded.
	for (const [status, code] of [
		[409, "LISTING_UNAVAILABLE"],
		[404, "NOT_FOUND"],
	] as const) {
		test(`a checkout refused with ${code} reloads the preview`, async () => {
			let reloads = 0;
			await render([PAID], {
				hub: "marketplace",
				apiGet: marketplaceGet(openOrder()),
				apiPost: await refusedCheckout(status, code),
				onAccessChanged: () => {
					reloads += 1;
				},
			});

			await click("Buy");
			await prepareCheckout();

			expect(reloads).toBe(1);
		});
	}

	test("a checkout that fails for another reason leaves the preview alone", async () => {
		let reloads = 0;
		await render([PAID], {
			hub: "marketplace",
			apiGet: marketplaceGet(openOrder()),
			apiPost: await refusedCheckout(409, "PAYMENT_TERMS_REQUIRED"),
			onAccessChanged: () => {
				reloads += 1;
			},
		});

		await click("Buy");
		await prepareCheckout();

		expect(reloads).toBe(0);
		expect(button("Prepare checkout")).toBeDefined();
	});

	test("a package bought elsewhere settles as soon as checkout is opened", async () => {
		const posted: string[] = [];
		let reloads = 0;
		await render([PAID], {
			hub: "marketplace",
			apiGet: marketplaceGet(openOrder(), () => BUYER_PERMISSION),
			apiPost: (path) => {
				posted.push(path);
				return null;
			},
			onAccessChanged: () => {
				reloads += 1;
			},
		});

		await click("Buy");

		expect(host.textContent).toContain("Purchased");
		expect(checkoutDialog()).toBeNull();
		expect(reloads).toBe(1);
		expect(posted).toEqual([]);
	});

	test("a payment confirmed while the fork is being created makes no promise about that fork", async () => {
		let reloads = 0;
		await render([PAID], {
			disabled: true,
			pendingCheckouts: remembered(2, "chart-kit"),
			apiGet: () => ({
				visibility: "public",
				price: 499,
				currentUserPermission: BUYER_PERMISSION,
			}),
			onAccessChanged: () => {
				reloads += 1;
			},
		});
		const { act } = await import("react");
		const { messages, restore } = await captureSuccessToasts();
		try {
			// Let the real poll interval elapse once.
			await act(async () => {
				await new Promise((resolve) => setTimeout(resolve, 4200));
			});
			await flush();
		} finally {
			restore();
		}

		expect(reloads).toBe(1);
		expect(messages).toHaveLength(1);
		expect(messages[0]).toContain("Chart Kit is yours.");
		expect(messages[0]).not.toContain("comes with your fork");
		expect(host.textContent).toContain("You own this package");
	}, 15_000);

	test("a pending request shows as requested without a button", async () => {
		await render([{ ...ON_REQUEST, request_pending: true }]);

		expect(button("Request access")).toBeUndefined();
		expect(host.textContent).toContain("Requested");
	});

	test("unavailable and revoked packages are left out without an action", async () => {
		await render([
			{ ...PAID, package_id: "retired", name: "Retired", block: "unavailable" },
			{ ...PAID, package_id: "lost", name: "Lost", block: "revoked" },
		]);

		const text = host.textContent ?? "";
		expect(host.querySelectorAll("button").length).toBe(0);
		expect(text).toContain("isn't available in the registry right now");
		expect(text).toContain("Your access to this package was revoked");
	});

	test("signed-out viewers get disabled actions and a sign-in hint", async () => {
		const { useAuthStatusStore } = await import("../../../state/backend-state");
		useAuthStatusStore.setState({ signedIn: false });
		await render([PAID, ON_REQUEST]);

		expect(button("Buy")?.hasAttribute("disabled")).toBe(true);
		expect(button("Request access")?.hasAttribute("disabled")).toBe(true);
		expect(host.textContent).toContain("Sign in to get these packages.");
	});

	test("for a copy made earlier the rows offer to add the package to it", async () => {
		await render([PAID, ON_REQUEST], { context: "copy" });

		const text = host.textContent ?? "";
		expect(text).toContain("2 packages are missing from your copy");
		expect(text).toContain("Buy it to add it to your copy.");
		expect(text).toContain("Request access to add it to your copy.");
		expect(text).not.toContain("before forking");
		expect(button("Buy")?.hasAttribute("disabled")).toBe(false);
	});

	test("a package bought for a copy made earlier is announced as being added to it", async () => {
		const { messages, restore } = await captureSuccessToasts();
		try {
			await render([PAID], {
				context: "copy",
				purchasePackage: async (packageId) => ({
					alreadyHasAccess: true,
					packageId,
				}),
			});
			await click("Buy");
		} finally {
			restore();
		}

		expect(messages).toEqual([
			"Chart Kit is yours and is being added to your copy.",
		]);
	});

	test("access granted for a copy made earlier is announced as being added to it", async () => {
		let reloads = 0;
		const { messages, restore } = await captureSuccessToasts();
		try {
			await render([ON_REQUEST], {
				context: "copy",
				requestAccess: async (packageId) => ({
					granted: true,
					queued: false,
					requiresPurchase: false,
					packageId,
				}),
				onAccessChanged: () => {
					reloads += 1;
				},
			});
			await click("Request access");
		} finally {
			restore();
		}

		expect(messages).toEqual([
			"Access granted. Team Utils is being added to your copy.",
		]);
		expect(reloads).toBe(1);
	});

	test("a request sent for a copy made earlier promises the package once approved", async () => {
		await render([{ ...ON_REQUEST, request_pending: true }], {
			context: "copy",
		});

		expect(host.textContent).toContain(
			"Once the author approves it, the package is added to your copy.",
		);
	});

	test("a checkout still open for a copy made earlier promises the package once paid", async () => {
		await render([PAID], {
			context: "copy",
			pendingCheckouts: remembered(2, "chart-kit"),
		});

		const text = host.textContent ?? "";
		expect(button("Waiting for payment")).toBeDefined();
		expect(text).toContain(
			"Once the payment is confirmed, the package is added to your copy.",
		);
		expect(text).not.toContain("If you fork before that");
	});

	test("where purchasing is unavailable a paid package is left out, with no sign-in promise", async () => {
		const { useAuthStatusStore } = await import("../../../state/backend-state");
		useAuthStatusStore.setState({ signedIn: false });
		await render([PAID], {
			seed: (queryClient) =>
				queryClient.setQueryData(["payment-distribution"], false),
		});

		const text = host.textContent ?? "";
		expect(button("Buy")).toBeUndefined();
		expect(text).toContain("purchasing isn't available in this app");
		expect(text).not.toContain("Sign in");
	});
});

describe("ForkRepinnedPackages", () => {
	test("names the version the copy uses and the one it replaces, with nothing to act on", async () => {
		const { act } = await import("react");
		const { ForkRepinnedPackages } = await import("./fork-blocked-packages");
		await act(async () =>
			root.render(
				<ForkRepinnedPackages
					packages={[
						{
							package_id: "chart-kit",
							name: "Chart Kit",
							pinned_version: "2.0.0",
							version: "1.4.0",
						},
					]}
				/>,
			),
		);

		const text = host.textContent ?? "";
		expect(text).toContain("1 package comes in another version");
		expect(text).toContain("Chart Kit");
		expect(text).toContain("Your copy uses 1.4.0 instead of 2.0.0.");
		expect(host.querySelectorAll("button").length).toBe(0);
	});
});

describe("ForkAppDialog", () => {
	const REPINNED = {
		package_id: "pdf-tools",
		name: "PDF Tools",
		pinned_version: "2.0.0",
		version: "1.4.0",
	};
	const dialogText = () => window.document.body.textContent ?? "";

	test(
		"lists what the fork leaves out and what it carries in another version",
		async () => {
			await prepare({});
			await showDialog(true, async () => ({
				...PREVIEW,
				blocked_packages: [PRIVATE],
				repinned_packages: [REPINNED],
			}));

			expect(dialogText()).toContain("1 package won't come with your copy");
			expect(dialogText()).toContain("1 package comes in another version");
			expect(dialogText()).toContain("Your copy uses 1.4.0 instead of 2.0.0.");
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	const refusals: [string, Partial<IForkPreviewResponse>][] = [
		[
			"the caller may not fork",
			{ user_can_fork: false, disallow_reason: "Forking is not enabled" },
		],
		["the fork exceeds the limits", { within_limits: false }],
	];
	for (const [reason, refusal] of refusals) {
		test(`offers no package when ${reason}`, async () => {
			await prepare({});
			await showDialog(true, async () => ({
				...PREVIEW,
				...refusal,
				blocked_packages: [PAID],
				repinned_packages: [REPINNED],
			}));

			expect(dialogText()).toContain("Fork CRM");
			expect(dialogText()).not.toContain("won't come with your copy");
			expect(dialogText()).not.toContain("comes in another version");
			expect(button("Buy")).toBeUndefined();
		});
	}

	test("closing the dialog forgets a purchase that was still open", async () => {
		await prepare({
			purchasePackage: async (packageId) => ({
				alreadyHasAccess: false,
				checkoutUrl: "https://pay.example/session",
				packageId,
			}),
		});
		const loadPreview = async () => ({ ...PREVIEW, blocked_packages: [PAID] });
		await showDialog(true, loadPreview);
		await click("Buy");
		expect(dialogText()).toContain("Payment for Chart Kit isn't confirmed yet");

		await showDialog(false, loadPreview);
		await showDialog(true, loadPreview);

		expect(dialogText()).not.toContain("isn't confirmed yet");
		expect(button("Buy")?.hasAttribute("disabled")).toBe(false);
	});

	const browserCheckout: RenderOptions = {
		purchasePackage: async (packageId) => ({
			alreadyHasAccess: false,
			checkoutUrl: "https://pay.example/session",
			packageId,
		}),
	};

	test("a purchase that is still open survives a reload of the preview", async () => {
		await prepare(browserCheckout);
		await showDialog(true, async () => ({
			...PREVIEW,
			blocked_packages: [PAID],
		}));
		await click("Buy");
		expect(button("Waiting for payment")).toBeDefined();

		// Another loader reloads the preview from scratch, which rebuilds the rows.
		await showDialog(true, async () => ({
			...PREVIEW,
			blocked_packages: [PAID],
		}));

		expect(button("Waiting for payment")).toBeDefined();
		expect(button("Buy")).toBeUndefined();
		expect(dialogText()).toContain("Payment for Chart Kit isn't confirmed yet");
	});

	test("a package that is no longer for sale stops counting as an open payment", async () => {
		await prepare(browserCheckout);
		await showDialog(true, async () => ({
			...PREVIEW,
			blocked_packages: [PAID],
		}));
		await click("Buy");
		expect(dialogText()).toContain("Payment for Chart Kit isn't confirmed yet");

		await showDialog(true, async () => ({
			...PREVIEW,
			blocked_packages: [{ ...PAID, block: "unavailable" }],
		}));

		expect(dialogText()).toContain("isn't available in the registry right now");
		expect(dialogText()).not.toContain("isn't confirmed yet");
	});
});
