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
const { ComputerStage, computerStageState } = await import(
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
	container = window.document.createElement("div");
	window.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
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
