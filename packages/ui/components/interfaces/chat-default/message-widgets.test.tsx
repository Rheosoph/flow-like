import { afterAll, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { IRole } from "../../../lib";
import type { Surface } from "../../a2ui/types";
import type { IChatWidget, IMessage } from "./chat-db";

const rendered: Surface[] = [];
const scheduled: string[] = [];
mock.module("../../a2ui/A2UIRenderer", () => ({
	A2UIRenderer: ({ surface }: { surface: Surface }) => {
		rendered.push(surface);
		return <div data-probe={surface.id} />;
	},
}));
mock.module("../../../lib/widget-snapshot", () => ({
	registerWidgetSnapshotSource: () => {},
	scheduleWidgetSnapshot: (instanceId: string, signature: string) => {
		scheduled.push(`${instanceId}@${signature}`);
	},
	unregisterWidgetSnapshotSource: () => {},
	widgetSnapshotAttribute: () => ({}),
}));
afterAll(() => mock.restore());

function widget(updates: unknown[] = []): IChatWidget {
	return {
		instance_id: "inst-1",
		widget_id: "w-1",
		surface_id: "inst-1",
		component: {
			type: "widgetInstance",
			instanceId: "inst-1",
			widgetId: "w-1",
			inlineWidgetDef: {
				name: "Probe",
				rootComponentId: "root",
				components: [
					{ id: "root", component: { type: "text", text: "hello" } },
				],
			},
		},
		updates,
	};
}

function liveMessage(content: string, widgets: IChatWidget[]): IMessage {
	return {
		id: "a1",
		timestamp: 0,
		inner: { role: IRole.Assistant, content },
		widgets,
	} as unknown as IMessage;
}

function setText(text: string) {
	return {
		type: "upsertElement",
		element_id: "inst-1",
		value: { type: "setText", text },
	};
}

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		navigator: window.navigator,
		window,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		ShadowRoot: window.ShadowRoot,
		Range: window.Range,
		Selection: window.Selection,
		Text: window.Text,
		Comment: window.Comment,
		DOMParser: window.DOMParser,
		getSelection: window.getSelection?.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		ResizeObserver: class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	return { container, root: createRoot(container) };
}

beforeEach(() => {
	rendered.length = 0;
	scheduled.length = 0;
});

describe("embedded widgets during streaming", () => {
	test("streamed text chunks do not re-render an unchanged widget", async () => {
		const { MessageComponent } = await import("./message");
		const { root } = await setup();
		const widgets = [widget()];

		await act(async () => {
			root.render(
				<MessageComponent loading message={liveMessage("Hel", widgets)} />,
			);
		});
		expect(rendered.length).toBe(1);

		for (const content of ["Hello", "Hello wor", "Hello world"]) {
			await act(async () => {
				root.render(
					<MessageComponent loading message={liveMessage(content, widgets)} />,
				);
			});
		}
		expect(rendered.length).toBe(1);
		await act(async () => root.unmount());
	});

	test("a live a2ui update re-renders only the widget it extends", async () => {
		const { MessageComponent } = await import("./message");
		const { root } = await setup();
		const initial = [widget()];

		await act(async () => {
			root.render(
				<MessageComponent loading message={liveMessage("Hi", initial)} />,
			);
		});
		expect(rendered.length).toBe(1);

		// attachA2UIUpdate replaces only the targeted widget object.
		const extended = [{ ...initial[0], updates: [setText("changed")] }];
		await act(async () => {
			root.render(
				<MessageComponent loading message={liveMessage("Hi", extended)} />,
			);
		});
		const last = rendered[rendered.length - 1];
		const component = last.components["inst-1"]?.component as {
			text?: string;
		};
		expect(component.text).toBe("changed");

		const settledCount = rendered.length;
		await act(async () => {
			root.render(
				<MessageComponent
					loading
					message={liveMessage("Hi there", extended)}
				/>,
			);
		});
		expect(rendered.length).toBe(settledCount);
		await act(async () => root.unmount());
	});
});

describe("widget snapshot pre-capture", () => {
	test("is scheduled once per rendered state by default", async () => {
		const { MessageComponent } = await import("./message");
		const { root } = await setup();
		const widgets = [widget()];

		await act(async () => {
			root.render(<MessageComponent message={liveMessage("Hi", widgets)} />);
		});
		await act(async () => {
			root.render(
				<MessageComponent message={liveMessage("Hi again", widgets)} />,
			);
		});
		expect(scheduled).toHaveLength(1);
		expect(scheduled[0]?.startsWith("inst-1@")).toBe(true);

		await act(async () => {
			root.render(
				<MessageComponent
					message={liveMessage("Hi", [
						{ ...widgets[0], updates: [setText("next")] },
					])}
				/>,
			);
		});
		expect(scheduled).toHaveLength(2);
		expect(scheduled[1]).not.toBe(scheduled[0]);
		await act(async () => root.unmount());
	});

	test("never runs when the chat does not attach snapshots", async () => {
		const { MessageComponent } = await import("./message");
		const { root } = await setup();
		const widgets = [widget()];

		await act(async () => {
			root.render(
				<MessageComponent
					message={liveMessage("Hi", widgets)}
					widgetSnapshots={false}
				/>,
			);
		});
		await act(async () => {
			root.render(
				<MessageComponent
					message={liveMessage("Hi", [
						{ ...widgets[0], updates: [setText("next")] },
					])}
					widgetSnapshots={false}
				/>,
			);
		});
		expect(scheduled).toHaveLength(0);
		await act(async () => root.unmount());
	});
});
