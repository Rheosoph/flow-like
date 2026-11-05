import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IBotComputerView } from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = { locales: { ...(await import("@flow-like/locales")) } };
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { ComputerDesktopOnly, ComputerStage, computerStageState } = await import(
	"./bot-computer-pane"
);

const active: IBotComputerView = {
	state: "active",
	kind: "browser",
	caption: "https://shop.example/",
	image: "data:image/jpeg;base64,AAAA",
	age_ms: 400,
};
const asleep: IBotComputerView = {
	state: "asleep",
	kind: null,
	caption: null,
	image: null,
	age_ms: null,
};

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => {
	installDomGlobals();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	restoreDomGlobals();
});

async function show(
	state: Parameters<typeof ComputerStage>[0]["state"],
	view: IBotComputerView | undefined,
	handlers: { onStop?: () => void; onRetry?: () => void } = {},
) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () => {
		root.render(
			createElement(ComputerStage, {
				state,
				view,
				name: "Scout",
				onStop: handlers.onStop ?? (() => {}),
				onRetry: handlers.onRetry ?? (() => {}),
			}),
		);
	});
}

async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

describe("the Computer tab's states", () => {
	test("no session reads as asleep", () => {
		expect(computerStageState(undefined, false)).toBe("asleep");
		expect(computerStageState(asleep, false)).toBe("asleep");
		expect(computerStageState({ ...active, image: null }, false)).toBe(
			"asleep",
		);
		expect(computerStageState(active, false)).toBe("active");
		expect(computerStageState(undefined, true)).toBe("failed");
		expect(computerStageState(active, true)).toBe("active");
	});

	test("asleep says so and offers no Stop", async () => {
		await show("asleep", asleep);
		expect(container.textContent).toContain("Computer is asleep");
		expect(container.querySelector("img")).toBeNull();
		expect(container.querySelector("button")).toBeNull();
		await hide();
	});

	test("an active session shows the latest picture, its page and a Stop button", async () => {
		const stop = mock(() => {});
		await show("active", active, { onStop: stop });
		const picture = container.querySelector("img");
		expect(picture?.getAttribute("src")).toBe(active.image);
		expect(container.textContent).toContain("https://shop.example/");
		const button = container.querySelector("button");
		expect(button?.textContent).toContain("Stop");
		await act(async () => button?.click());
		expect(stop).toHaveBeenCalledTimes(1);
		await hide();
	});

	test("a failed read says so and retries", async () => {
		const retry = mock(() => {});
		await show("failed", undefined, { onRetry: retry });
		expect(container.textContent).toContain("Couldn't reach");
		const button = container.querySelector("button");
		await act(async () => button?.click());
		expect(retry).toHaveBeenCalledTimes(1);
		await hide();
	});
});

const device: IBotComputerView = {
	...active,
	kind: "computer",
	caption: "Desktop",
	can_control: true,
	taken_over: false,
	width: 1280,
	height: 800,
};

async function showControls(
	view: IBotComputerView,
	handlers: {
		onTakeOver?: () => void;
		onHandBack?: () => void;
		onInput?: (event: unknown) => void;
	},
) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () => {
		root.render(
			createElement(ComputerStage, {
				state: "active",
				view,
				name: "Scout",
				onStop: () => {},
				onRetry: () => {},
				...handlers,
			} as never),
		);
	});
}

const labels = () =>
	[...container.querySelectorAll("button")].map((b) => b.textContent?.trim());

describe("taking over a bot's screen", () => {
	test("a controllable screen offers Take over and Stop", async () => {
		const takeOver = mock(() => {});
		await showControls(device, { onTakeOver: takeOver, onHandBack: () => {} });
		expect(labels()).toEqual(["Take over", "Stop"]);
		await act(async () =>
			(container.querySelector("button") as HTMLElement).click(),
		);
		expect(takeOver).toHaveBeenCalledTimes(1);
		await hide();
	});

	test("a browser frame is watch only: no Take over", async () => {
		await showControls(
			{ ...device, kind: "browser", can_control: false },
			{ onTakeOver: () => {}, onHandBack: () => {} },
		);
		expect(labels()).toEqual(["Stop"]);
		expect(container.textContent).toContain("Watch only");
		await hide();
	});

	test("a local bot without can_control can still be paused", async () => {
		await showControls(
			{ ...active, kind: "computer", can_control: undefined },
			{ onTakeOver: () => {}, onHandBack: () => {} },
		);
		expect(labels()).toEqual(["Take over", "Stop"]);
		await hide();
	});

	test("while taken over it says you are in control and offers Hand back", async () => {
		const back = mock(() => {});
		await showControls(
			{ ...device, taken_over: true },
			{ onTakeOver: () => {}, onHandBack: back, onInput: () => {} },
		);
		expect(container.textContent).toContain("You're in control");
		expect(labels()).toEqual(["Hand back", "Stop"]);
		expect(container.querySelector('[role="application"]')).not.toBeNull();
		await act(async () =>
			(container.querySelector("button") as HTMLElement).click(),
		);
		expect(back).toHaveBeenCalledTimes(1);
		await hide();
	});

	test("a local bot taken over is paused, not captured", async () => {
		await showControls(
			{ ...active, kind: "computer", can_control: undefined, taken_over: true },
			{ onTakeOver: () => {}, onHandBack: () => {} },
		);
		expect(container.textContent).toContain("Scout is paused");
		expect(container.querySelector('[role="application"]')).toBeNull();
		await hide();
	});

	test("typing and shortcuts reach onInput while driving", async () => {
		const sent: unknown[] = [];
		await showControls(
			{ ...device, taken_over: true },
			{ onHandBack: () => {}, onInput: (e) => sent.push(e) },
		);
		const surface = container.querySelector(
			'[role="application"]',
		) as HTMLElement;
		await act(async () => {
			surface.dispatchEvent(
				new window.KeyboardEvent("keydown", {
					key: "h",
					bubbles: true,
				}) as never,
			);
			surface.dispatchEvent(
				new window.KeyboardEvent("keydown", {
					key: "c",
					ctrlKey: true,
					bubbles: true,
				}) as never,
			);
		});
		expect(sent).toEqual([
			{ type: "text", text: "h" },
			{ type: "key", chord: "Control+c" },
		]);
		await hide();
	});

	test("a tap on Escape is sent, holding it hands back", async () => {
		const sent: unknown[] = [];
		const back = mock(() => {});
		await showControls(
			{ ...device, taken_over: true },
			{ onHandBack: back, onInput: (e) => sent.push(e) },
		);
		const surface = container.querySelector(
			'[role="application"]',
		) as HTMLElement;
		const press = (type: string) =>
			surface.dispatchEvent(
				new window.KeyboardEvent(type, {
					key: "Escape",
					bubbles: true,
				}) as never,
			);
		await act(async () => {
			press("keydown");
			press("keyup");
		});
		expect(sent).toEqual([{ type: "key", chord: "Escape" }]);
		expect(back).not.toHaveBeenCalled();
		await act(async () => {
			press("keydown");
			await new Promise((resolve) => setTimeout(resolve, 1100));
		});
		expect(back).toHaveBeenCalledTimes(1);
		await hide();
	});

	test("an asleep screen that is still taken over can be handed back", async () => {
		const back = mock(() => {});
		container = window.document.createElement("div") as unknown as HTMLElement;
		window.document.body.appendChild(container as never);
		root = createRoot(container);
		await act(async () => {
			root.render(
				createElement(ComputerStage, {
					state: "asleep",
					view: { ...asleep, taken_over: true },
					name: "Scout",
					onStop: () => {},
					onRetry: () => {},
					onHandBack: back,
				}),
			);
		});
		expect(labels()).toEqual(["Hand back"]);
		await hide();
	});
});

describe("the web app", () => {
	test("a device bot's screen points to the desktop app", async () => {
		container = window.document.createElement("div") as unknown as HTMLElement;
		window.document.body.appendChild(container as never);
		root = createRoot(container);
		await act(async () =>
			root.render(createElement(ComputerDesktopOnly, { name: "Scout" })),
		);
		expect(container.textContent).toContain(
			"Watch and take over in the desktop app",
		);
		expect(container.querySelector("button")).toBeNull();
		await hide();
	});
});
