import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { IBlockedPackage } from "../../../lib/schema/app/fork";
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
	/** What `registry/package/{id}` answers while a checkout is polled. */
	registryEntry?: () => unknown;
	onAccessChanged?: () => void;
	onCheckoutPendingChange?: (packageId: string, pending: boolean) => void;
	seed?: (client: QueryClient) => void;
}

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/store" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		Element: window.Element,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		NodeList: window.NodeList,
		MutationObserver: window.MutationObserver,
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
	const { useAuthStatusStore } = await import("../../../state/backend-state");
	await act(() => root.unmount());
	useAuthStatusStore.setState({ signedIn: undefined });
	await window.happyDOM.abort();
	restoreGlobals();
});

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

async function render(
	packages: IBlockedPackage[],
	options: RenderOptions = {},
) {
	const { act } = await import("react");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { AuthContext } = await import("react-oidc-context");
	const { useBackendStore } = await import("../../../state/backend-state");
	const { ForkBlockedPackages } = await import("./fork-blocked-packages");

	useBackendStore.getState().setBackend({
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "hub" } }),
			getProfile: async () => null,
		},
		apiState: { get: async () => options.registryEntry?.() ?? null },
		registryState: {
			purchasePackage: options.purchasePackage,
			requestAccess: options.requestAccess,
		},
	} as never);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	options.seed?.(client);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<AuthContext.Provider
					value={{ isAuthenticated: true, user: null } as never}
				>
					<ForkBlockedPackages
						packages={packages}
						onAccessChanged={options.onAccessChanged ?? (() => {})}
						onCheckoutPendingChange={options.onCheckoutPendingChange}
					/>
				</AuthContext.Provider>
			</QueryClientProvider>,
		),
	);
	await flush();
}

function button(label: string) {
	return Array.from(host.querySelectorAll("button")).find((candidate) =>
		candidate.textContent?.includes(label),
	);
}

async function click(label: string) {
	const { act } = await import("react");
	const target = button(label);
	if (!target) throw new Error(`No "${label}" button`);
	await act(async () => {
		target.dispatchEvent(
			new window.MouseEvent("click", { bubbles: true }) as never,
		);
	});
	await flush();
}

/** Replaces the checkout poll's interval so a test fires its ticks by hand. */
function captureIntervals() {
	const ticks: (() => void)[] = [];
	const cleared: unknown[] = [];
	Object.assign(window, {
		setInterval: (tick: () => void) => ticks.push(tick),
		clearInterval: (id: unknown) => cleared.push(id),
		open: () => null,
	});
	return { ticks, cleared };
}

describe("ForkBlockedPackages", () => {
	test(
		"offers a purchase, an access request or says the package is left out",
		async () => {
			await render([PAID, ON_REQUEST, PRIVATE]);

			const text = host.textContent ?? "";
			expect(text).toContain("3 packages won't come with your copy");
			expect(button("Buy")?.textContent).toContain("4.99");
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
		const { act } = await import("react");
		const { ticks, cleared } = captureIntervals();
		const pending: [string, boolean][] = [];
		let permission = 0;
		let reloads = 0;
		await render([PAID], {
			purchasePackage: async (packageId) => ({
				alreadyHasAccess: false,
				checkoutUrl: "https://pay.example/session",
				packageId,
			}),
			registryEntry: () => ({
				visibility: "public",
				price: 499,
				currentUserPermission: permission,
			}),
			onAccessChanged: () => {
				reloads += 1;
			},
			onCheckoutPendingChange: (packageId, isPending) => {
				pending.push([packageId, isPending]);
			},
		});

		await click("Buy");

		expect(button("Waiting for payment")).toBeDefined();
		expect(pending).toEqual([["chart-kit", true]]);
		expect(ticks.length).toBeGreaterThan(0);

		const tick = async () => {
			await act(async () => ticks[ticks.length - 1]?.());
			await flush();
		};
		await tick();
		expect(reloads).toBe(0);

		permission = BUYER_PERMISSION;
		await tick();
		expect(reloads).toBe(1);
		expect(host.textContent).toContain("Purchased");
		expect(pending.at(-1)).toEqual(["chart-kit", false]);
		expect(cleared.length).toBeGreaterThan(0);

		await tick();
		expect(reloads).toBe(1);
	});

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
		expect(host.textContent).toContain(
			"Sign in to get these packages for your fork.",
		);
	});

	test("where purchasing is unavailable a paid package is left out, with no sign-in promise", async () => {
		const { useAuthStatusStore } = await import("../../../state/backend-state");
		useAuthStatusStore.setState({ signedIn: false });
		await render([PAID], {
			seed: (client) => client.setQueryData(["payment-distribution"], false),
		});

		const text = host.textContent ?? "";
		expect(button("Buy")).toBeUndefined();
		expect(text).toContain("purchasing isn't available in this app");
		expect(text).not.toContain("Sign in");
	});
});
