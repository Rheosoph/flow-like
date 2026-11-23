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

let pathname = "/";
const pushed: string[] = [];
const router = {
	push: (href: string) => pushed.push(href),
	replace: (href: string) => pushed.push(href),
	back() {},
	forward() {},
	refresh() {},
	prefetch() {},
};

mock.module("next/navigation", () => ({
	usePathname: () => pathname,
	useRouter: () => router,
	useSearchParams: () => new URLSearchParams(),
}));

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { GuideProvider } = await import("./guide-provider");
const { useGuideStore } = await import("../../state/guide-state");
const { emitGuideEvent } = await import("../../lib/guide/guide-events");
const { useBackendStore } = await import("../../state/backend-state");
const { useDeveloperMode } = await import("../../hooks/use-developer-mode");

let serverDevMode = false;
let tutorialCompleted = false;
let profileApps: string[] = [];
const userUpdates: { dev_mode?: boolean }[] = [];
const previousBackend = useBackendStore.getState().backend;
useBackendStore.setState({
	backend: {
		userState: {
			getInfo: async () => ({
				dev_mode: serverDevMode,
				tutorial_completed: tutorialCompleted,
			}),
			getProfile: async () => ({ apps: profileApps }),
			updateUser: async (update: { dev_mode?: boolean }) => {
				userUpdates.push(update);
				serverDevMode = update.dev_mode ?? serverDevMode;
			},
		},
	} as never,
});

const developer = { on: false, set: (_value: boolean) => {} };
function DeveloperModeProbe() {
	const { developerMode, setDeveloperMode } = useDeveloperMode();
	developer.on = developerMode;
	developer.set = setDeveloperMode;
	return null;
}

const body = () => window.document.body;
const text = () => body().textContent ?? "";

function renderApp(root: ReturnType<typeof createRoot>) {
	const client = new QueryClient();
	return act(async () => {
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement("a", { href: "/bots", "data-guide": "nav-bots" }, "Bots"),
				createElement(DeveloperModeProbe),
				createElement(GuideProvider),
			),
		);
	});
}

function giveTargetASize() {
	const target = body().querySelector('[data-guide="nav-bots"]');
	if (!target) throw new Error("the sidebar link did not render");
	Object.defineProperty(target, "getBoundingClientRect", {
		value: () => ({ top: 20, left: 10, width: 120, height: 32 }),
	});
}

async function mount() {
	const host = window.document.createElement("div");
	body().appendChild(host);
	const root = createRoot(host as unknown as HTMLElement);
	await renderApp(root);
	giveTargetASize();
	await settle(50);
	return root;
}

function clickButton(label: string) {
	const button = [...body().querySelectorAll("button")].find(
		(candidate) => candidate.textContent?.trim() === label,
	);
	if (!button) throw new Error(`no button "${label}"`);
	return act(async () => {
		button.click();
	});
}

beforeAll(installDomGlobals);

beforeEach(() => {
	pathname = "/";
	pushed.length = 0;
	tutorialCompleted = false;
	profileApps = [];
	window.localStorage.removeItem("tutorial-finished-new");
	body().innerHTML = "";
	useGuideStore.setState({
		welcome: "done",
		run: null,
		finished: null,
		completed: [],
	});
});

afterAll(() => {
	useBackendStore.setState({ backend: previousBackend });
	restoreDomGlobals();
});

describe("GuideProvider", () => {
	test("rings the real control and follows what the user does", async () => {
		useGuideStore.getState().startTour("use");
		const root = await mount();

		expect(text()).toContain("Click Bots in the sidebar.");
		expect(text()).toContain("1 of 7");
		const tags = [...body().querySelectorAll("div")].filter(
			(node) => node.textContent === "Click",
		);
		expect(tags.length).toBeGreaterThan(0);

		pathname = "/bots";
		await renderApp(root);
		await settle();
		expect(text()).toContain("Pick a bot from your crew.");

		await act(async () => emitGuideEvent("bots:message-sent"));
		await settle();
		expect(text()).toContain("Watch it work.");

		await act(async () => emitGuideEvent("bots:reply-finished"));
		await settle();
		expect(text()).toContain("Click the bot's name above the chat.");

		await clickButton("End tour");
		expect(useGuideStore.getState().run).toBeNull();
		expect(text()).not.toContain("Guide");
		await act(async () => root.unmount());
	});

	test("Do it for me clicks the ringed control", async () => {
		useGuideStore.getState().startTour("use");
		const root = await mount();
		let clicked = 0;
		body()
			.querySelector('[data-guide="nav-bots"]')
			?.addEventListener("click", (event) => {
				event.preventDefault();
				clicked += 1;
			});
		await clickButton("Do it for me");
		expect(clicked).toBe(1);
		await act(async () => root.unmount());
	});

	test("a finished tour offers the other one", async () => {
		useGuideStore.setState({ finished: "use" });
		const root = await mount();
		expect(text()).toContain("You're set.");
		await clickButton("Take the builder tour");
		expect(useGuideStore.getState().run?.tour).toBe("build");
		await act(async () => root.unmount());
	});

	test("sends a new user from Home to the welcome page, once", async () => {
		useGuideStore.setState({ welcome: "unknown" });
		const root = await mount();
		await settle(50);
		expect(pushed).toEqual(["/onboarding/welcome"]);
		expect(useGuideStore.getState().welcome).toBe("done");

		await renderApp(root);
		await settle(50);
		expect(pushed).toEqual(["/onboarding/welcome"]);
		await act(async () => root.unmount());
	});

	test("leaves people who have apps or finished the intro on Home", async () => {
		profileApps = ["app-1"];
		useGuideStore.setState({ welcome: "unknown" });
		let root = await mount();
		await settle(50);
		expect(pushed).toEqual([]);
		expect(useGuideStore.getState().welcome).toBe("done");
		await act(async () => root.unmount());

		profileApps = [];
		tutorialCompleted = true;
		useGuideStore.setState({ welcome: "unknown" });
		body().innerHTML = "";
		root = await mount();
		await settle(50);
		expect(pushed).toEqual([]);
		await act(async () => root.unmount());
	});

	test("the builder tour turns developer mode on, the bots tour does not", async () => {
		const root = await mount();
		await act(async () => developer.set(false));
		await settle(50);
		expect(developer.on).toBe(false);
		expect(serverDevMode).toBe(false);
		userUpdates.length = 0;

		await act(async () => useGuideStore.getState().startTour("use"));
		await settle(50);
		expect(userUpdates).toEqual([]);

		await act(async () => useGuideStore.getState().startTour("build"));
		await settle(50);
		expect(serverDevMode).toBe(true);
		expect(developer.on).toBe(true);
		await act(async () => root.unmount());
	});

	test("stays out of the way during onboarding", async () => {
		pathname = "/onboarding/welcome";
		useGuideStore.getState().startTour("use");
		const root = await mount();
		expect(text()).not.toContain("Click Bots in the sidebar.");
		await act(async () => root.unmount());
	});
});
