import { afterAll, afterEach, beforeEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";

/** The first render transpiles the backend-state graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 30_000;

const browser = new Window({ url: "https://app.flow-like.com/store/explore" });
const globalKeys = {
	window: browser,
	document: browser.document,
	navigator: browser.navigator,
	localStorage: browser.localStorage,
	HTMLElement: browser.HTMLElement,
	Element: browser.Element,
	Text: browser.Text,
	DocumentFragment: browser.DocumentFragment,
	Node: browser.Node,
	MutationObserver: browser.MutationObserver,
	Event: browser.Event,
	CustomEvent: browser.CustomEvent,
	requestAnimationFrame: browser.requestAnimationFrame.bind(browser),
	cancelAnimationFrame: browser.cancelAnimationFrame.bind(browser),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globalKeys).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globalKeys);
Object.assign(browser, { SyntaxError, TypeError });

afterAll(() => {
	for (const [key, descriptor] of descriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { AuthContext } = await import("react-oidc-context");
const { exploreFixture } = await import("./explore-fixture");
const { useAuthStatusStore, useBackendStore } = await import(
	"../../../state/backend-state"
);
const { EXPLORE_AUTH_STATE_GRACE_MS, useExplore, useExploreViewer } =
	await import("./use-explore");

let root: Root;
let host: HTMLElement;
let calls = 0;

function Probe() {
	const { signedIn } = useExploreViewer();
	useExplore();
	return <output data-signed-in={String(signedIn)} />;
}

function signedInShown(): string | null | undefined {
	return host.querySelector("output")?.getAttribute("data-signed-in");
}

async function flush(ms = 20) {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

async function until(check: () => boolean, label: string, attempts = 100) {
	for (let attempt = 0; attempt < attempts; attempt += 1) {
		if (check()) return;
		await flush(10);
	}
	throw new Error(`Timed out waiting for ${label}`);
}

function withAuth(node: ReactNode, isLoading: boolean | undefined) {
	if (isLoading === undefined) return node;
	return (
		<AuthContext.Provider value={{ isLoading } as AuthContextProps}>
			{node}
		</AuthContext.Provider>
	);
}

function mount() {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	return (authLoading?: boolean) =>
		act(async () =>
			root.render(
				<QueryClientProvider client={client}>
					{withAuth(<Probe />, authLoading)}
				</QueryClientProvider>,
			),
		);
}

async function push(signedIn: boolean | undefined) {
	await act(async () => useAuthStatusStore.setState({ signedIn }));
}

async function remount() {
	await act(() => root.unmount());
	root = createRoot(host);
}

beforeEach(async () => {
	calls = 0;
	useBackendStore.getState().setBackend({
		profile: undefined,
		appState: {
			getExplore: async () => {
				calls += 1;
				return exploreFixture({ dev: false });
			},
		},
		userState: {
			getInfo: async () => ({ dev_mode: false, permission: 0 }),
			updateUser: async () => {},
		},
	} as never);
	host = browser.document.createElement("div") as unknown as HTMLElement;
	browser.document.body.appendChild(host as never);
	root = createRoot(host);
	// A settled viewer ends any app-wide wait an earlier test left behind.
	await push(false);
	await mount()(false);
	await flush();
	await remount();
	calls = 0;
});

afterEach(async () => {
	await act(() => root.unmount());
	host.remove();
});

test(
	"a signed-out push while OIDC loads fetches nothing; the settled push does, and a sign-in round trip keeps it",
	async () => {
		const render = mount();
		await push(false);
		await render(true);
		await flush(50);
		expect(calls).toBe(0);
		expect(signedInShown()).toBe("undefined");

		await push(true);
		await render(false);
		await until(() => calls === 1, "the signed-in request");
		expect(signedInShown()).toBe("true");

		await render(true);
		await flush(50);
		expect(signedInShown()).toBe("true");
		expect(calls).toBe(1);

		await push(false);
		await render(false);
		await until(() => calls === 2, "the signed-out request");
		expect(signedInShown()).toBe("false");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a host that never pushes its auth state is asked as signed out after the grace period, and only once",
	async () => {
		await push(undefined);
		await mount()(false);
		await flush(100);
		expect(calls).toBe(0);
		expect(signedInShown()).toBe("undefined");

		await until(
			() => calls === 1,
			"the signed-out request after the grace period",
			EXPLORE_AUTH_STATE_GRACE_MS / 5,
		);
		expect(signedInShown()).toBe("false");

		await remount();
		await mount()(false);
		await until(() => calls === 2, "a later page asking without waiting", 5);
		expect(signedInShown()).toBe("false");

		await push(true);
		await until(() => calls === 3, "the late push taking over");
		expect(signedInShown()).toBe("true");
	},
	EXPLORE_AUTH_STATE_GRACE_MS * 4,
);
