import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type { DockSelection } from "./dock-selection";
import {
	installDomGlobals,
	restoreDomGlobals,
	domWindow as window,
} from "./testing/dom-harness";

const actualNavigation = { ...(await import("next/navigation")) };
let params = new URLSearchParams();
const router = {
	replace: mock((href: string) => {
		window.history.replaceState(null, "", href);
		params = new URLSearchParams(window.location.search);
	}),
};
mock.module("next/navigation", () => ({
	...actualNavigation,
	useRouter: () => router,
	usePathname: () => "/bots",
	useSearchParams: () => params,
}));

const { createRoot } = await import("react-dom/client");
const { useDockSelection } = await import("./use-dock-selection");
let dock: ReturnType<typeof useDockSelection>;
let root: ReturnType<typeof createRoot>;
let container: HTMLElement;

function Harness() {
	dock = useDockSelection();
	return null;
}

async function render() {
	await act(async () => root.render(createElement(Harness)));
}

function setUrl(href: string) {
	window.history.replaceState(null, "", href);
	params = new URLSearchParams(window.location.search);
}

beforeEach(() => {
	installDomGlobals();
	setUrl("/bots");
	router.replace.mockClear();
	container = window.document.createElement("div") as unknown as HTMLElement;
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
});

afterAll(() => {
	mock.module("next/navigation", () => actualNavigation);
	restoreDomGlobals();
});

describe("bookmarked dock conversations", () => {
	const targets: [string, DockSelection, string][] = [
		["a bot", { type: "bot", botId: "bot_scout" }, "bot=bot_scout"],
		[
			"a group",
			{ type: "conversation", conversationId: "room-1" },
			"conversation=room-1",
		],
		[
			"a person",
			{ type: "conversation", conversationId: "dm-1" },
			"conversation=dm-1",
		],
		["Flux", { type: "system", system: "flux" }, "bot=flux"],
		[
			"an encoded ID",
			{ type: "conversation", conversationId: "dm/a+b&c" },
			"conversation=dm%2Fa%2Bb%26c",
		],
	];

	for (const [name, selection, query] of targets) {
		test(`opening ${name} updates the URL and a fresh mount opens it again`, async () => {
			await render();
			await act(async () => dock.select(selection));
			expect(window.location.search).toBe(`?${query}`);
			expect(dock.selection).toEqual(selection);

			const bookmark = window.location.href;
			await act(async () => root.unmount());
			setUrl(bookmark);
			root = createRoot(container);
			await render();
			expect(dock.selection).toEqual(selection);
			expect(dock.dockOpen).toBe(true);
		});
	}

	test("an external link replaces a closed dock's previous conversation", async () => {
		setUrl("/bots?bot=bot_scout");
		await render();
		await act(async () => dock.closeDock());
		setUrl("/bots?conversation=dm-2");
		await render();
		expect(dock.selection).toEqual({
			type: "conversation",
			conversationId: "dm-2",
		});
		expect(dock.dockOpen).toBe(true);
	});

	test("an external Flux session link reopens the dock even when the bot is unchanged", async () => {
		setUrl("/bots?bot=flux&session=chat-1");
		await render();
		await act(async () => dock.closeDock());
		setUrl("/bots?bot=flux&session=chat-2");
		await render();
		expect(dock.selection).toEqual({ type: "system", system: "flux" });
		expect(dock.dockOpen).toBe(true);
	});

	test("a selection keeps unrelated parameters and the hash while clearing the old session", async () => {
		setUrl("/bots?bot=flux&session=old-chat&panel=memory&voice=1#latest");
		await render();
		await act(async () =>
			dock.select({ type: "conversation", conversationId: "room-2" }),
		);
		expect(window.location.search).toBe("?voice=1&conversation=room-2");
		expect(window.location.hash).toBe("#latest");
		expect(dock.panel).toBeNull();
	});
});
