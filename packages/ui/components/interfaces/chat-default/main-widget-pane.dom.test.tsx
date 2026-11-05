// @vitest-environment happy-dom

import { act, useEffect } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { A2UIServerMessage, Surface } from "../../a2ui/types";
import type { IChatMainWidget, IChatWidget } from "./chat-db";
import { MainWidgetPane } from "./main-widget-pane";

const probe = vi.hoisted(() => ({
	mounts: 0,
	unmounts: 0,
	chatMounts: 0,
	surface: undefined as Surface | undefined,
	feedback: undefined as ((message: A2UIServerMessage) => void) | undefined,
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
// The package's Node export omits panel registration. Exercise its browser
// implementation against the DOM so resize assertions use the real layout.
vi.mock("react-resizable-panels", async () =>
	vi.importActual<typeof import("react-resizable-panels")>(
		"../../../../../node_modules/react-resizable-panels/dist/react-resizable-panels.browser.development.esm.js",
	),
);
vi.mock("../../../lib/widget-snapshot", () => ({
	registerWidgetSnapshotSource: vi.fn(),
	scheduleWidgetSnapshot: vi.fn(),
	unregisterWidgetSnapshotSource: vi.fn(),
	widgetSnapshotAttribute: () => ({}),
}));
vi.mock("../../a2ui/A2UIRenderer", () => ({
	A2UIRenderer: ({
		surface,
		onA2UIMessage,
	}: {
		surface: Surface;
		onA2UIMessage: (message: A2UIServerMessage) => void;
	}) => {
		probe.surface = surface;
		probe.feedback = onA2UIMessage;
		useEffect(() => {
			probe.mounts++;
			return () => {
				probe.unmounts++;
			};
		}, []);
		return (
			<iframe
				title="Browser widget"
				srcDoc="<p>Browser</p>"
				data-instance={surface.id}
			/>
		);
	},
}));

let root: Root;
let container: HTMLDivElement;

function widget(instanceId = "browser"): IChatWidget {
	return {
		instance_id: instanceId,
		widget_id: "browser-widget",
		surface_id: instanceId,
		component: { type: "column", children: { explicitList: [] } },
	};
}

function ChatProbe() {
	useEffect(() => {
		probe.chatMounts++;
	}, []);
	return <input aria-label="Message" defaultValue="Draft message" />;
}

async function render(mainWidget?: IChatMainWidget, sessionId = "session-a") {
	await act(async () => {
		root.render(
			<MainWidgetPane
				mainWidget={mainWidget}
				sessionId={sessionId}
				snapshots={false}
			>
				<ChatProbe />
			</MainWidgetPane>,
		);
	});
}

function control(label: string) {
	const element = container.querySelector<HTMLButtonElement>(
		`button[aria-label="${label}"]`,
	);
	if (!element) throw new Error(`Missing control: ${label}`);
	return element;
}

async function click(label: string) {
	await act(async () => control(label).click());
}

function display() {
	return container.firstElementChild?.getAttribute("data-main-widget-display");
}

function widgetSize() {
	return Number(
		container
			.querySelectorAll("[data-panel-size]")[1]
			?.getAttribute("data-panel-size"),
	);
}

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	probe.mounts = 0;
	probe.unmounts = 0;
	probe.chatMounts = 0;
	probe.surface = undefined;
	probe.feedback = undefined;
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe("main widget pane", () => {
	test("collapse, maximize, restore and resize keep the browser and chat mounted", async () => {
		await render();
		const draft = container.querySelector("input");
		const main = { widget: widget(), title: "Browser" };
		await render(main);
		const browser = container.querySelector("iframe");
		expect(display()).toBe("open");
		expect(widgetSize()).toBe(50);

		const divider = container.querySelector<HTMLElement>("[role=separator]");
		await act(async () => {
			divider?.dispatchEvent(
				new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }),
			);
		});
		const resized = widgetSize();
		expect(resized).toBeGreaterThan(50);
		await click("Collapse main widget");
		expect(display()).toBe("collapsed");
		expect(widgetSize()).toBe(0);
		await click("Show main widget");
		expect(widgetSize()).toBe(resized);
		await click("Maximize main widget");
		expect(display()).toBe("maximized");
		expect(widgetSize()).toBe(100);
		await click("Restore main widget");
		expect(widgetSize()).toBe(resized);
		expect(container.querySelector("iframe")).toBe(browser);
		expect(container.querySelector("input")).toBe(draft);
		expect(probe.mounts).toBe(1);
		expect(probe.chatMounts).toBe(1);
		expect(probe.unmounts).toBe(0);
	});

	test("updates preserve user collapse, new commands apply and session changes reset the pane", async () => {
		const initialWidget = widget();
		const main: IChatMainWidget = {
			widget: initialWidget,
			display: "open",
			command_id: "command-a",
		};
		await render(main);
		await click("Collapse main widget");
		await render({ ...main, widget: { ...initialWidget, updates: [] } });
		expect(display()).toBe("collapsed");
		await render({ ...main, command_id: "command-b" });
		expect(display()).toBe("open");
		await render({ ...main, command_id: "command-c", display: "maximized" });
		expect(display()).toBe("maximized");
		await act(async () => {
			control("Restore main widget").dispatchEvent(
				new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
			);
		});
		expect(display()).toBe("open");
		await render(undefined, "session-b");
		expect(display()).toBe("empty");
		expect(container.querySelector("iframe")).toBeNull();
		await render(main, "session-b");
		expect(widgetSize()).toBe(50);
		expect(probe.mounts).toBe(2);
		expect(probe.chatMounts).toBe(1);
	});

	test("exchanging or clearing a widget preserves the chat draft", async () => {
		await render({ widget: widget() });
		const draft = container.querySelector<HTMLInputElement>("input");
		if (!draft) throw new Error("Missing chat draft");
		draft.value = "Keep this draft";
		await render({ widget: widget("replacement"), title: "Library" });
		expect(
			container.querySelector("iframe")?.getAttribute("data-instance"),
		).toBe("replacement");
		expect(probe.mounts).toBe(2);
		expect(probe.unmounts).toBe(1);
		await render({ widget: null });
		expect(display()).toBe("empty");
		expect(container.querySelector("input")).toBe(draft);
		expect(draft.value).toBe("Keep this draft");
		expect(probe.chatMounts).toBe(1);
	});

	test("a changed snapshot for the same instance updates the existing renderer", async () => {
		const initial = widget();
		await render({ widget: initial });
		const browser = container.querySelector("iframe");
		await render({
			widget: { ...initial, component: { type: "text", text: "Updated" } },
		});
		expect(probe.surface?.components.browser.component).toMatchObject({
			text: "Updated",
		});
		expect(container.querySelector("iframe")).toBe(browser);
		expect(probe.mounts).toBe(1);
		await render({ widget: { ...initial, surface_id: "another-surface" } });
		expect(probe.surface?.id).toBe("another-surface");
	});

	test("switching to mobile hides the pane until its dialog button is pressed", async () => {
		let updateQuery: (() => void) | undefined;
		const query = {
			matches: false,
			addEventListener: (_type: string, listener: () => void) => {
				updateQuery = listener;
			},
			removeEventListener: vi.fn(),
		} as unknown as MediaQueryList;
		vi.spyOn(window, "matchMedia").mockReturnValue(query);
		await render({ widget: widget() });
		const browser = container.querySelector("iframe");
		const dialog = container.querySelector("dialog");
		if (!dialog) throw new Error("Missing widget dialog");
		await act(async () => {
			Object.assign(query, { matches: true });
			updateQuery?.();
		});
		expect(widgetSize()).toBe(0);
		expect(display()).toBe("collapsed");
		expect(dialog.open).toBe(false);
		expect(
			container.querySelector<HTMLElement>("[role=separator]")?.style.display,
		).toBe("none");
		await click("Show main widget");
		expect(display()).toBe("dialog");
		expect(dialog.open).toBe(true);
		expect(dialog.getAttribute("aria-modal")).toBe("true");
		await click("Close main widget");
		expect(dialog.open).toBe(false);
		expect(widgetSize()).toBe(0);
		await act(async () => {
			Object.assign(query, { matches: false });
			updateQuery?.();
		});
		expect(display()).toBe("open");
		expect(widgetSize()).toBe(50);
		expect(dialog.open).toBe(true);
		expect(container.querySelector("iframe")).toBe(browser);
		expect(probe.mounts).toBe(1);
	});

	test("mobile commands and replacements never reopen a dialog the user closed", async () => {
		vi.spyOn(window, "matchMedia").mockReturnValue({
			matches: true,
			addEventListener: vi.fn(),
			removeEventListener: vi.fn(),
		} as unknown as MediaQueryList);
		const main: IChatMainWidget = {
			widget: widget(),
			display: "maximized",
			command_id: "initial",
		};
		await render(main);
		const dialog = container.querySelector("dialog");
		if (!dialog) throw new Error("Missing widget dialog");
		expect(dialog.open).toBe(false);
		expect(widgetSize()).toBe(0);
		await click("Show main widget");
		await render({
			...main,
			widget: widget("replacement"),
			command_id: "replacement",
		});
		expect(dialog.open).toBe(true);
		await act(async () => {
			dialog.dispatchEvent(new Event("cancel", { cancelable: true }));
		});
		expect(dialog.open).toBe(false);
		await render({ ...main, command_id: "maximize-again" });
		expect(dialog.open).toBe(false);
		expect(widgetSize()).toBe(0);
		await click("Show main widget");
		await render(main, "another-session");
		expect(dialog.open).toBe(false);
		expect(display()).toBe("collapsed");
	});

	test("persisted action feedback does not append a child twice", async () => {
		const initial = widget();
		await render({ widget: initial });
		const update = {
			type: "upsertElement",
			element_id: "browser",
			value: { type: "pushChild", childId: "new-child" },
		};
		await act(async () => probe.feedback?.(update as A2UIServerMessage));
		await render({ widget: { ...initial, updates: [update] } });
		expect(probe.surface?.components.browser.component).toMatchObject({
			children: { explicitList: ["new-child"] },
		});
	});
});
