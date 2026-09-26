import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";

let pathname = "/apps/chat";
let search = "page=one";
const stop = mock(() => {});
// bun keeps a module mock for every later file in the process, so each mocked module is
// captured first and put back in afterAll.
const actual = {
	nextNavigation: { ...(await import("next/navigation")) },
	frontendAudio: { ...(await import("../lib/frontend-audio")) },
};
mock.module("next/navigation", () => ({
	...actual.nextNavigation,
	usePathname: () => pathname,
	useSearchParams: () => new URLSearchParams(search),
}));
mock.module("../lib/frontend-audio", () => ({
	...actual.frontendAudio,
	stopAllFrontendAudio: stop,
}));
afterAll(() => {
	mock.module("next/navigation", () => actual.nextNavigation);
	mock.module("../lib/frontend-audio", () => actual.frontendAudio);
});
const { FrontendAudioLifecycle } = await import("./frontend-audio-lifecycle");

let browser: Window;
let root: Root;
const globals = [
	"window",
	"document",
	"navigator",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;
const originals = new Map(
	globals.map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]),
);

beforeEach(() => {
	pathname = "/apps/chat";
	search = "page=one";
	stop.mockClear();
	browser = new Window({ url: "https://flow-like.test/apps/chat?page=one" });
	for (const [key, value] of Object.entries({
		window: browser,
		document: browser.document,
		navigator: browser.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	}))
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value,
		});
	const container = browser.document.createElement("div");
	browser.document.body.append(container);
	root = createRoot(container as unknown as HTMLElement);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await browser.happyDOM.close();
	for (const key of globals) {
		const descriptor = originals.get(key);
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

test("route, page query and account changes invalidate old playback, ordinary renders do not", async () => {
	const render = (scope = "account-one") =>
		act(async () => {
			root.render(<FrontendAudioLifecycle scope={scope} />);
		});
	await render();
	await render();
	expect(stop).not.toHaveBeenCalled();
	search = "page=two";
	await render();
	expect(stop).toHaveBeenCalledTimes(1);
	pathname = "/chat";
	await render();
	expect(stop).toHaveBeenCalledTimes(2);
	await render("account-two");
	expect(stop).toHaveBeenCalledTimes(3);
	await render("account-two");
	expect(stop).toHaveBeenCalledTimes(3);
});

test("provider unmount invalidates playback even without a route change", async () => {
	await act(async () =>
		root.render(<FrontendAudioLifecycle scope="account" />),
	);
	await act(async () => root.render(null));
	expect(stop).toHaveBeenCalledTimes(1);
});

test("backgrounding invalidates pending Events before they request audio", async () => {
	await act(async () =>
		root.render(<FrontendAudioLifecycle scope="account" />),
	);
	Object.defineProperty(browser.document, "visibilityState", {
		configurable: true,
		value: "hidden",
	});
	browser.document.dispatchEvent(new browser.Event("visibilitychange"));
	expect(stop).toHaveBeenCalledTimes(1);
	browser.dispatchEvent(new browser.Event("flow-like:device-inactive"));
	expect(stop).toHaveBeenCalledTimes(2);
	browser.dispatchEvent(new browser.Event("pagehide"));
	expect(stop).toHaveBeenCalledTimes(3);
});
