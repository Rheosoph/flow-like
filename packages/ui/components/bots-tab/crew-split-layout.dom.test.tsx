import { afterAll, beforeAll, expect, test } from "bun:test";
import { act } from "react";
import {
	installDomGlobals,
	restoreDomGlobals,
	domWindow as window,
} from "./testing/dom-harness";

const { createRoot } = await import("react-dom/client");
const { CrewSplitLayout, useDockPanelLayout } = await import(
	"./crew-split-layout"
);
let measure: () => void;
let outerWidth = 1200;
let root: ReturnType<typeof createRoot>;
let host: HTMLElement;
// Read after installDomGlobals: an earlier file in the run may have restored the globals.
let originalRect: HTMLElement["getBoundingClientRect"];

beforeAll(async () => {
	installDomGlobals();
	originalRect = HTMLElement.prototype.getBoundingClientRect;
	Object.assign(globalThis, {
		ResizeObserver: class {
			constructor(callback: () => void) {
				measure = callback;
			}
			observe() {}
			disconnect() {}
		},
	});
	HTMLElement.prototype.getBoundingClientRect = function () {
		return {
			width:
				this.tagName === "ASIDE"
					? Number.parseFloat(this.style.width)
					: outerWidth,
		} as DOMRect;
	};
	window.localStorage.removeItem("flow-like:bots:chat-width");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
	await act(async () =>
		root.render(
			<CrewSplitLayout dock={<textarea defaultValue="Unsent message" />}>
				<div>Crew</div>
			</CrewSplitLayout>,
		),
	);
});

afterAll(async () => {
	await act(async () => root.unmount());
	host.remove();
	HTMLElement.prototype.getBoundingClientRect = originalRect;
	window.localStorage.removeItem("flow-like:bots:chat-width");
	restoreDomGlobals();
});

function handle() {
	const element = host.querySelector<HTMLElement>('[role="separator"]');
	if (!element) throw new Error("Missing resize handle");
	return element;
}
async function key(key: string) {
	await act(async () => {
		handle().dispatchEvent(
			new KeyboardEvent("keydown", { key, bubbles: true }),
		);
	});
}

test("widens and narrows with the keyboard, keeping the draft mounted", async () => {
	const draft = host.querySelector("textarea");
	expect(handle().getAttribute("aria-valuenow")).toBe("420");
	await key("ArrowLeft");
	expect(handle().getAttribute("aria-valuenow")).toBe("440");
	expect(window.localStorage.getItem("flow-like:bots:chat-width")).toBe("440");
	await key("ArrowRight");
	expect(handle().getAttribute("aria-valuenow")).toBe("420");
	expect(host.querySelector("textarea")).toBe(draft);
	expect(draft?.value).toBe("Unsent message");
});

test("leaves room for the directory and clamps when the window narrows", async () => {
	await key("End");
	expect(handle().getAttribute("aria-valuenow")).toBe("920");
	await act(async () => {
		outerWidth = 1000;
		measure();
	});
	expect(handle().getAttribute("aria-valuenow")).toBe("720");
	await act(async () => {
		outerWidth = 820;
		measure();
	});
	expect(host.querySelector("aside")?.style.width).toBe("540px");
	await act(async () => {
		outerWidth = 1000;
		measure();
	});
	await key("Home");
	expect(handle().getAttribute("aria-valuenow")).toBe("360");
	await key("ArrowRight");
	expect(handle().getAttribute("aria-valuenow")).toBe("360");
});

test("drags left to widen the chat and releases pointer capture", async () => {
	const divider = handle();
	let captured = false;
	divider.setPointerCapture = () => {
		captured = true;
	};
	divider.hasPointerCapture = () => captured;
	divider.releasePointerCapture = () => {
		captured = false;
		divider.dispatchEvent(
			new PointerEvent("lostpointercapture", { bubbles: true }),
		);
	};
	await act(async () => {
		divider.dispatchEvent(
			new PointerEvent("pointerdown", {
				button: 0,
				clientX: 640,
				pointerId: 1,
				bubbles: true,
			}),
		);
	});
	await act(async () => {
		divider.dispatchEvent(
			new PointerEvent("pointermove", {
				clientX: 400,
				pointerId: 1,
				bubbles: true,
			}),
		);
	});
	expect(divider.getAttribute("aria-valuenow")).toBe("600");
	await act(async () => {
		divider.dispatchEvent(
			new PointerEvent("pointerup", { pointerId: 1, bubbles: true }),
		);
	});
	expect(captured).toBe(false);
	expect(window.localStorage.getItem("flow-like:bots:chat-width")).toBe("600");
});

function LayoutProbe() {
	return <output>{useDockPanelLayout()}</output>;
}

const layouts: string[] = [];

async function renderPanel(panelOpen: boolean) {
	await act(async () =>
		root.render(
			<CrewSplitLayout
				dock={<LayoutProbe />}
				panelOpen={panelOpen}
				onPanelLayoutChange={(layout) => layouts.push(layout)}
			>
				<div>Crew</div>
			</CrewSplitLayout>,
		),
	);
}

async function resizeTo(width: number) {
	await act(async () => {
		outerWidth = width;
		measure();
	});
}

function shown() {
	return {
		layout: host.querySelector("output")?.textContent,
		dock: host.querySelector("aside")?.style.width,
		chat: handle().getAttribute("aria-valuenow"),
		max: handle().getAttribute("aria-valuemax"),
	};
}

test("an open panel sits beside the chat while crew, chat and panel fit, widening the dock", async () => {
	await renderPanel(true);
	await resizeTo(1400);
	expect(shown()).toEqual({
		layout: "column",
		dock: "1000px",
		chat: "600",
		max: "720",
	});
	expect(layouts.at(-1)).toBe("column");
	await key("End");
	expect(shown().dock).toBe("1120px");
	await key("ArrowRight");
	expect(shown().chat).toBe("700");
});

test("a narrower tab opens it over the chat, and a closed panel takes no room", async () => {
	await resizeTo(1000);
	expect(shown()).toEqual({
		layout: "sheet",
		dock: "700px",
		chat: "700",
		max: "720",
	});
	expect(layouts.at(-1)).toBe("sheet");

	await resizeTo(1400);
	await renderPanel(false);
	expect(shown()).toEqual({
		layout: "column",
		dock: "700px",
		chat: "700",
		max: "1120",
	});
});
