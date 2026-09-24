import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";

let root: Root | undefined;
let restoreGlobals: (() => void) | undefined;

afterEach(async () => {
	await act(async () => root?.unmount());
	root = undefined;
	restoreGlobals?.();
	restoreGlobals = undefined;
});

async function renderSetting(noCache: boolean) {
	const window = new Window();
	// Bun does not populate this Happy DOM realm constructor used by selector parsing.
	Object.assign(window, { SyntaxError });
	const globals = {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.fromEntries(
		Object.keys(globals).map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	Object.assign(globalThis, globals);
	restoreGlobals = () => {
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	const { PageNoCacheSetting } = await import("./page-no-cache-setting");
	const changes: (true | undefined)[] = [];
	const container = window.document.createElement("div");
	window.document.body.append(container);
	root = createRoot(container as unknown as HTMLElement);
	await act(async () => {
		root?.render(
			<PageNoCacheSetting
				noCache={noCache}
				onChange={(value) => changes.push(value)}
			/>,
		);
	});
	const toggle = container.querySelector('[role="switch"]');
	if (!toggle) throw new Error("The No cache switch did not render");
	return { container, toggle, changes };
}

describe("PageNoCacheSetting", () => {
	test("is labelled and explained, and turning it on sets noCache", async () => {
		const { container, toggle, changes } = await renderSetting(false);
		expect(toggle.getAttribute("aria-checked")).toBe("false");
		const label = container.querySelector(`label[for="${toggle.id}"]`);
		expect(label?.textContent).toBe("No cache");
		const hint = container.ownerDocument.getElementById(
			toggle.getAttribute("aria-describedby") ?? "",
		);
		expect(hint?.textContent).toBe(
			"Only show fresh workflow output. Shows a loading screen until the onLoad workflow renders.",
		);

		await act(async () => (toggle as unknown as HTMLElement).click());
		expect(changes).toEqual([true]);
	});

	test("turning it off clears noCache instead of storing false", async () => {
		const { toggle, changes } = await renderSetting(true);
		expect(toggle.getAttribute("aria-checked")).toBe("true");

		await act(async () => (toggle as unknown as HTMLElement).click());
		expect(changes).toEqual([undefined]);
	});
});
