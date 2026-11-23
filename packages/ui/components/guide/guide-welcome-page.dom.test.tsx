import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "../bots-tab/testing/dom-harness";

let search = "";
const navigated: string[] = [];
const router = {
	push: (href: string) => navigated.push(href),
	replace: (href: string) => navigated.push(href),
	back() {},
	forward() {},
	refresh() {},
	prefetch() {},
};

mock.module("next/navigation", () => ({
	usePathname: () => "/onboarding/welcome",
	useRouter: () => router,
	useSearchParams: () => new URLSearchParams(search),
}));

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { GuideWelcomePage } = await import("./guide-welcome-page");
const { useGuideStore } = await import("../../state/guide-state");
const { useBackendStore } = await import("../../state/backend-state");

const userUpdates: unknown[] = [];
const previousBackend = useBackendStore.getState().backend;
useBackendStore.setState({
	backend: {
		userState: {
			updateUser: async (update: unknown) => {
				userUpdates.push(update);
			},
		},
	} as never,
});

/** Reduced motion makes the chat answer at once instead of after a pause. */
const reducedMotion = () => ({
	matches: true,
	addEventListener() {},
	removeEventListener() {},
});
const previousMatchMedia = window.matchMedia;

const body = () => window.document.body;
const text = () => body().textContent ?? "";

function findButton(label: string) {
	const buttons = [...body().querySelectorAll("button")];
	const found =
		buttons.find((button) => button.textContent?.trim() === label) ??
		buttons.find((button) => button.textContent?.includes(label));
	if (!found) throw new Error(`no button "${label}"`);
	return found;
}

async function click(label: string) {
	const button = findButton(label);
	await act(async () => button.click());
	await settle();
}

async function press(key: string) {
	await act(async () => {
		window.dispatchEvent(new window.KeyboardEvent("keydown", { key }));
	});
	await settle();
}

async function mount() {
	const host = window.document.createElement("div");
	body().appendChild(host);
	const root = createRoot(host as unknown as HTMLElement);
	await act(async () => root.render(createElement(GuideWelcomePage)));
	await settle();
	return root;
}

beforeAll(() => {
	installDomGlobals();
	Object.assign(window, { matchMedia: reducedMotion });
	Object.assign(globalThis, { matchMedia: reducedMotion });
});

beforeEach(() => {
	search = "";
	navigated.length = 0;
	userUpdates.length = 0;
	body().innerHTML = "";
	useGuideStore.setState({
		welcome: "unknown",
		role: null,
		run: null,
		finished: null,
		completed: [],
	});
});

afterAll(() => {
	useBackendStore.setState({ backend: previousBackend });
	Object.assign(window, { matchMedia: previousMatchMedia });
	restoreDomGlobals();
});

describe("GuideWelcomePage", () => {
	test("tells one job from the request down to the flow, then starts the chosen tour", async () => {
		const root = await mount();
		expect(text()).toContain("It starts with a sentence.");
		expect(text()).toContain("1 of 4 · You ask");

		await click("Send");
		expect(text()).toContain("Let Post read the Finance inbox?");
		await click("Allow");
		await settle();
		expect(text()).toContain("Found 4 new invoices.");

		await click("Next");
		expect(text()).toContain("Post did it with an app.");
		expect(text()).toContain("Invoice Inbox");

		await click("Look inside the app");
		expect(text()).toContain("Inside every app is a flow.");
		expect(text()).toContain("Ran in the cloud");
		await click("Your servers");
		expect(text()).toContain("Ran on your servers");

		await click("Next");
		expect(text()).toContain("Where do you want to start?");
		expect(text()).toContain("Stay up here");

		await click("Build my own");
		const state = useGuideStore.getState();
		expect(state.welcome).toBe("done");
		expect(state.run?.tour).toBe("build");
		expect(userUpdates).toEqual([{ tutorial_completed: true }]);
		expect(navigated).toEqual(["/"]);
		await act(async () => root.unmount());
	});

	test("Not now is respected and the story carries on with a sample", async () => {
		const root = await mount();
		await click("Send");
		await click("Not now");
		expect(text()).toContain("Not allowed");
		expect(text()).toContain("Here's a sample in Invoice Inbox instead.");
		await act(async () => root.unmount());
	});

	test("Show me around opens on the question, with the story one click away", async () => {
		search = "start=question";
		const root = await mount();
		expect(text()).toContain("What brings you here?");
		expect(text()).not.toContain("Stay up here");
		expect(text()).not.toContain("Skip to the question");

		await click("Replay intro");
		expect(text()).toContain("It starts with a sentence.");
		await press("ArrowRight");
		expect(text()).toContain("Post did it with an app.");
		await press("Escape");
		expect(text()).toContain("What brings you here?");

		await click("I'll look around myself");
		expect(useGuideStore.getState().run).toBeNull();
		expect(useGuideStore.getState().welcome).toBe("done");
		expect(navigated).toEqual(["/"]);
		await act(async () => root.unmount());
	});
});
