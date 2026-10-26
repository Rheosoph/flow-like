import {
	afterAll,
	afterEach,
	beforeEach,
	expect,
	mock,
	setSystemTime,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import type { IProfile } from "../types";

const window = new Window({ url: "https://app.example.com" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	IS_REACT_ACT_ENVIRONMENT: true,
};
// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const actual = {
	backendState: { ...(await import("../state/backend-state")) },
	invoke: { ...(await import("./use-invoke")) },
};
let profile: Partial<IProfile> | undefined;
const backend = { userState: { getProfile: async () => profile } };
mock.module("../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => backend,
}));
mock.module("./use-invoke", () => ({
	...actual.invoke,
	useInvoke: () => ({ data: profile }),
}));

const originalFetch = globalThis.fetch;
const originalConsoleError = console.error;
const errors = mock(() => {});
console.error = errors;
const originalOverride = process.env.NEXT_PUBLIC_API_URL;
const originalRuntime = process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG;
const requests: {
	url: string;
	signal?: AbortSignal | null;
	init?: RequestInit;
	resolve: (response: Response) => void;
	reject: (error: Error) => void;
}[] = [];
globalThis.fetch = ((input: string | URL | Request, init?: RequestInit) =>
	new Promise<Response>((resolve, reject) => {
		requests.push({
			url: String(input),
			signal: init?.signal,
			init,
			resolve,
			reject,
		});
	})) as typeof fetch;

const { createRoot } = await import("react-dom/client");
const { useHub } = await import("./use-hub");
const container = window.document.createElement("div");
window.document.body.appendChild(container);
const root = createRoot(container as unknown as HTMLElement);
const observations: { id: string; account: string; hub?: string }[] = [];
const refetches = new Map<string, () => Promise<void>>();
function Probe({ account, id = "a" }: { account: string; id?: string }) {
	const { hub, refetch } = useHub([account]);
	refetches.set(id, refetch);
	observations.push({ id, account, hub: hub?.name });
	return <output data-id={id}>{hub?.name ?? "unavailable"}</output>;
}
async function render(children: ReactNode) {
	await act(async () => {
		root.render(children);
	});
}
async function resolve(index: number, name: string) {
	await act(async () => {
		requests[index].resolve(Response.json({ name }));
	});
}
function text(id = "a") {
	return container.querySelector(`[data-id="${id}"]`)?.textContent;
}
function pair(accountA = "account-a", accountB = accountA) {
	return (
		<>
			<Probe key="a" id="a" account={accountA} />
			<Probe key="b" id="b" account={accountB} />
		</>
	);
}
beforeEach(() => {
	profile = { id: "profile-a", hub: "profile.example.com", secure: true };
	process.env.NEXT_PUBLIC_API_URL = "https://override.example.com/";
	Reflect.deleteProperty(process.env, "NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG");
	requests.length = 0;
	observations.length = 0;
	refetches.clear();
	errors.mockClear();
});
afterEach(async () => {
	await render(null);
	setSystemTime();
});
afterAll(async () => {
	await act(async () => root.unmount());
	globalThis.fetch = originalFetch;
	console.error = originalConsoleError;
	if (originalOverride === undefined)
		Reflect.deleteProperty(process.env, "NEXT_PUBLIC_API_URL");
	else process.env.NEXT_PUBLIC_API_URL = originalOverride;
	if (originalRuntime === undefined)
		Reflect.deleteProperty(process.env, "NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG");
	else process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = originalRuntime;
	mock.restore();
	mock.module("../state/backend-state", () => actual.backendState);
	mock.module("./use-invoke", () => actual.invoke);
	await window.happyDOM.close();
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

test("mounted consumers share one uncached request and one result", async () => {
	await render(pair());
	expect(requests).toHaveLength(1);
	expect(requests[0].url).toBe("https://override.example.com/api/v1");
	expect(requests[0].init?.cache).toBe("no-store");
	expect(requests[0].init?.headers).toBeUndefined();
	await resolve(0, "Shared hub");
	expect(text("a")).toBe("Shared hub");
	expect(text("b")).toBe("Shared hub");
	await render(<Probe key="a" account="account-a" />);
	await render(pair());
	expect(requests).toHaveLength(1);
	expect(text("b")).toBe("Shared hub");

	// The final unsubscribe discards availability, even after a successful load.
	await render(null);
	await render(pair());
	expect(requests).toHaveLength(2);
	expect(text("a")).toBe("unavailable");
	expect(text("b")).toBe("unavailable");
});

test("one consumer leaving does not cancel another consumer's request", async () => {
	await render(pair());
	await render(<Probe key="b" id="b" account="account-a" />);
	expect(requests).toHaveLength(1);
	expect(requests[0].signal?.aborted).toBe(false);
	await resolve(0, "Remaining consumer");
	expect(text("b")).toBe("Remaining consumer");
});

test("later consumers revalidate expired availability once and failures do not extend freshness", async () => {
	const start = new Date("2026-10-26T10:00:00Z").getTime();
	const consumers = (ids: string[]) =>
		ids.map((id) => <Probe key={id} id={id} account="account-a" />);
	setSystemTime(start);
	await render(consumers(["a"]));
	await resolve(0, "Initial hub");
	setSystemTime(start + 29_999);
	await render(consumers(["a", "b"]));
	expect(requests).toHaveLength(1);
	expect(text("b")).toBe("Initial hub");

	setSystemTime(start + 30_000);
	await render(consumers(["a", "b", "c", "d"]));
	expect(requests).toHaveLength(2);
	for (const id of ["a", "b", "c", "d"]) expect(text(id)).toBe("Initial hub");
	await act(async () =>
		requests[1].resolve(new Response("Unavailable", { status: 503 })),
	);

	setSystemTime(start + 31_000);
	await render(consumers(["a", "b", "c", "d", "e"]));
	expect(requests).toHaveLength(3);
	expect(text("e")).toBe("Initial hub");
	await resolve(2, "Updated hub");
	for (const id of ["a", "b", "c", "d", "e"])
		expect(text(id)).toBe("Updated hub");

	setSystemTime(start + 60_999);
	await render(consumers(["a", "b", "c", "d", "e", "f"]));
	expect(requests).toHaveLength(3);
	expect(text("f")).toBe("Updated hub");
});

test("a consumer switching account scopes leaves another consumer's pending request intact", async () => {
	await render(pair());
	await render(pair("account-b", "account-a"));
	expect(requests).toHaveLength(2);
	expect(requests[0].signal?.aborted).toBe(false);
	await resolve(0, "Original scope");
	expect(text("a")).toBe("unavailable");
	expect(text("b")).toBe("Original scope");
	await resolve(1, "New scope");
	expect(text("a")).toBe("New scope");
	expect(text("b")).toBe("Original scope");
});

test("the last unsubscribe aborts pending work and a late result cannot seed a remount", async () => {
	await render(pair());
	const oldRefetch = refetches.get("a");
	await render(null);
	expect(requests[0].signal?.aborted).toBe(true);
	await oldRefetch?.();
	expect(requests).toHaveLength(1);
	await render(pair());
	expect(requests).toHaveLength(2);
	await resolve(0, "Aborted old hub");
	expect(text("a")).toBe("unavailable");
	await resolve(1, "New hub");
	expect(text("a")).toBe("New hub");
});

test("concurrent account scopes have separate availability and requests", async () => {
	await render(pair("account-a", "account-b"));
	expect(requests).toHaveLength(2);
	await resolve(0, "Account A hub");
	expect(text("a")).toBe("Account A hub");
	expect(text("b")).toBe("unavailable");
	await resolve(1, "Account B hub");
	expect(text("b")).toBe("Account B hub");
	observations.length = 0;
	await render(pair("account-b", "account-b"));
	expect(requests).toHaveLength(2);
	expect(observations.find(({ id }) => id === "a")?.hub).toBeUndefined();
	expect(text("a")).toBe("Account B hub");
});

test("hub ignores responses superseded by account, profile, or origin changes", async () => {
	await render(<Probe account="account-a" />);
	await resolve(0, "Hub A");
	expect(text()).toBe("Hub A");
	await render(<Probe account="account-b" />);
	expect(
		observations.find((value) => value.account === "account-b")?.hub,
	).toBeUndefined();
	expect(text()).toBe("unavailable");
	profile = { ...profile, id: "profile-b" };
	await render(<Probe account="account-b" />);
	expect(requests[1].signal?.aborted).toBe(true);
	// This mock deliberately completes an aborted request to exercise stale-response rejection.
	await resolve(1, "Old account response");
	expect(text()).toBe("unavailable");
	await resolve(2, "Hub B");
	expect(text()).toBe("Hub B");
	Reflect.deleteProperty(process.env, "NEXT_PUBLIC_API_URL");
	profile = { ...profile, hub: "local.example.com", secure: false };
	await render(<Probe account="account-b" />);
	expect(requests[3].url).toBe("http://local.example.com/api/v1");
	expect(text()).toBe("unavailable");
	await resolve(3, "Local hub");
	expect(text()).toBe("Local hub");
	profile = undefined;
	await render(<Probe account="account-b" />);
	expect(text()).toBe("unavailable");
	await refetches.get("a")?.();
	expect(requests).toHaveLength(4);
});

test("explicit refetch supersedes pending work and publishes to every same-scope consumer", async () => {
	await render(pair());
	await resolve(0, "Initial hub");
	const first = refetches.get("a")?.();
	expect(requests).toHaveLength(2);
	const latest = refetches.get("b")?.();
	expect(requests).toHaveLength(3);
	expect(requests[1].signal?.aborted).toBe(true);
	await resolve(1, "Superseded hub");
	await first;
	expect(text("a")).toBe("Initial hub");
	expect(text("b")).toBe("Initial hub");
	await resolve(2, "Updated hub");
	await latest;
	expect(text("a")).toBe("Updated hub");
	expect(text("b")).toBe("Updated hub");
});

test("a response still parsing when superseded cannot publish stale availability", async () => {
	await render(pair());
	let finishBody: ((hub: unknown) => void) | undefined;
	const response = Response.json({});
	response.json = () =>
		new Promise((resolve) => {
			finishBody = resolve;
		});
	await act(async () => requests[0].resolve(response));
	const latest = refetches.get("a")?.();
	await act(async () => finishBody?.({ name: "Late body" }));
	expect(text("a")).toBe("unavailable");
	await resolve(1, "Latest hub");
	await latest;
	expect(text("a")).toBe("Latest hub");
	expect(text("b")).toBe("Latest hub");
});

test.each([
	["HTTP failure", () => new Response("unavailable", { status: 503 })],
	["invalid JSON", () => new Response("invalid json")],
	["invalid record", () => Response.json([])],
	[
		"upstream failure",
		() =>
			Response.json({ errorType: "Runtime.ExitError", errorMessage: "failed" }),
	],
])(
	"a shared %s allows refetch and preserves the last valid result",
	async (_name, response) => {
		await render(pair());
		await act(async () => requests[0].resolve(response()));
		expect(text("a")).toBe("unavailable");
		expect(text("b")).toBe("unavailable");
		expect(errors).toHaveBeenCalledTimes(1);
		const retry = refetches.get("a")?.();
		await resolve(1, "Recovered hub");
		await retry;
		expect(text("a")).toBe("Recovered hub");
		expect(text("b")).toBe("Recovered hub");
		const failedRefresh = refetches.get("b")?.();
		await act(async () => requests[2].resolve(response()));
		await failedRefresh;
		expect(text("a")).toBe("Recovered hub");
		expect(text("b")).toBe("Recovered hub");
	},
);

test("a later consumer retries a failed request instead of inheriting a cached failure", async () => {
	await render(<Probe key="a" account="account-a" />);
	await act(async () => requests[0].reject(new TypeError("Failed to fetch")));
	expect(text()).toBe("unavailable");
	expect(errors).toHaveBeenCalledTimes(1);
	await render(pair());
	expect(requests).toHaveLength(2);
	await resolve(1, "Recovered hub");
	expect(text("a")).toBe("Recovered hub");
	expect(text("b")).toBe("Recovered hub");
});
