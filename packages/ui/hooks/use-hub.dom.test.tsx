import { afterAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
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
let profile: Partial<IProfile> = {
	id: "profile-a",
	hub: "profile.example.com",
	secure: true,
};
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
const originalOverride = process.env.NEXT_PUBLIC_API_URL;
const originalRuntime = process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG;
const requests: {
	url: string;
	signal?: AbortSignal | null;
	resolve: (response: Response) => void;
}[] = [];
globalThis.fetch = ((input: string | URL | Request, init?: RequestInit) =>
	new Promise<Response>((resolve) => {
		requests.push({ url: String(input), signal: init?.signal, resolve });
	})) as typeof fetch;
process.env.NEXT_PUBLIC_API_URL = "https://override.example.com/";
Reflect.deleteProperty(process.env, "NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG");

const { createRoot } = await import("react-dom/client");
const { useHub } = await import("./use-hub");
const container = window.document.createElement("div");
window.document.body.appendChild(container);
const root = createRoot(container as unknown as HTMLElement);
const observations: { account: string; hub?: string }[] = [];
function Probe({ account }: { account: string }) {
	const { hub } = useHub([account]);
	observations.push({ account, hub: hub?.name });
	return <output>{hub?.name ?? "unavailable"}</output>;
}
async function render(account: string) {
	await act(async () => {
		root.render(<Probe account={account} />);
	});
}
async function resolve(index: number, name: string) {
	await act(async () => {
		requests[index].resolve(Response.json({ name }));
	});
}
afterAll(async () => {
	await act(async () => root.unmount());
	globalThis.fetch = originalFetch;
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

test("hub uses the API override and ignores responses superseded by account, profile, or origin changes", async () => {
	await render("account-a");
	expect(requests[0].url).toBe("https://override.example.com/api/v1");
	await resolve(0, "Hub A");
	expect(container.textContent).toBe("Hub A");
	await render("account-b");
	expect(requests[0].signal?.aborted).toBe(true);
	expect(
		observations.find((value) => value.account === "account-b")?.hub,
	).toBeUndefined();
	expect(container.textContent).toBe("unavailable");
	profile = { ...profile, id: "profile-b" };
	await render("account-b");
	expect(requests[1].signal?.aborted).toBe(true);
	// This mock deliberately completes an aborted request to exercise stale-response rejection.
	await resolve(1, "Old account response");
	expect(container.textContent).toBe("unavailable");
	await resolve(2, "Hub B");
	expect(container.textContent).toBe("Hub B");
	Reflect.deleteProperty(process.env, "NEXT_PUBLIC_API_URL");
	profile = { ...profile, hub: "local.example.com", secure: false };
	await render("account-b");
	expect(requests[3].url).toBe("http://local.example.com/api/v1");
	expect(container.textContent).toBe("unavailable");
	await resolve(3, "Local hub");
	expect(container.textContent).toBe("Local hub");
});
