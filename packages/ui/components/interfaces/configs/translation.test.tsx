import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { IBoard } from "../../../lib/schema/flow/board";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IEventPayload } from "../../../lib/schema/flow/event-payload";
import type { INode } from "../../../lib/schema/flow/node";
import type { IEventMapping } from "../interfaces";
import { EventTranslation, EventTypeConfiguration } from "./translation";

const node: INode = {
	id: "entry",
	name: "events_simple",
	friendly_name: "Run",
	description: "",
	category: "events",
	pins: {},
};
const event = { event_type: "quick_action" } as IEvent;
const mapping: IEventMapping = {
	events_simple: {
		defaultEventType: "quick_action",
		eventTypes: ["quick_action", "api"],
		configs: { quick_action: {}, api: { sink_type: "http" } },
		withSink: [],
		configInterfaces: {},
		useInterfaces: {},
	},
};

function renderTypeConfiguration(eventConfig: IEventMapping, entry = node) {
	return renderToStaticMarkup(
		<EventTypeConfiguration
			eventConfig={eventConfig}
			node={entry}
			event={event}
			disabled={false}
			onUpdate={() => {}}
		/>,
	);
}

describe("event type configuration", () => {
	test("omits the selector while the node mapping is unavailable", () => {
		expect(renderTypeConfiguration({})).toBe("");
	});

	test("omits the selector for an unregistered node", () => {
		expect(
			renderTypeConfiguration(mapping, { ...node, name: "events_custom" }),
		).toBe("");
	});

	test("omits the selector when the node has only one event type", () => {
		expect(
			renderTypeConfiguration({
				events_simple: {
					...mapping.events_simple,
					eventTypes: ["quick_action"],
				},
			}),
		).toBe("");
	});

	test("keeps the selector for registered nodes with multiple event types", () => {
		expect(renderTypeConfiguration(mapping)).toContain('role="combobox"');
	});

	test("updates defaults and the configuration panel when a mapping arrives", async () => {
		const window = new Window();
		Object.assign(window, { SyntaxError, TypeError, Error });
		const globals = {
			window,
			document: window.document,
			navigator: window.navigator,
			HTMLElement: window.HTMLElement,
			Element: window.Element,
			Node: window.Node,
			Event: window.Event,
			IS_REACT_ACT_ENVIRONMENT: true,
		};
		const previous = Object.fromEntries(
			Object.keys(globals).map((key) => [
				key,
				Object.getOwnPropertyDescriptor(globalThis, key),
			]),
		);
		Object.assign(globalThis, globals);
		const { createRoot } = await import("react-dom/client");
		const container = window.document.createElement("div");
		window.document.body.append(container);
		const root = createRoot(container as unknown as HTMLElement);
		const updates: [string, Partial<IEventPayload>][] = [];
		const onUpdate = (type: string, config: Partial<IEventPayload>) => {
			updates.push([type, config]);
		};
		const board = { id: "board", nodes: { [node.id]: node } } as IBoard;
		const resolvedMapping: IEventMapping = {
			events_simple: {
				...mapping.events_simple,
				defaultEventType: "api",
				eventTypes: ["api"],
				configInterfaces: {
					api: () => <p>HTTP endpoint configuration</p>,
				},
			},
		};
		const render = (eventConfig: IEventMapping) => (
			<>
				<EventTypeConfiguration
					eventConfig={eventConfig}
					node={node}
					event={event}
					disabled={false}
					onUpdate={onUpdate}
				/>
				<EventTranslation
					appId="app"
					eventConfig={eventConfig}
					eventType="api"
					editing
					board={board}
					nodeId={node.id}
					config={{}}
					onUpdate={() => {}}
				/>
			</>
		);

		try {
			await act(async () => root.render(render({})));
			expect(updates).toEqual([]);
			expect(container.textContent).toContain("No specific configuration");

			await act(async () => root.render(render(resolvedMapping)));
			expect(updates).toEqual([["api", { sink_type: "http" }]]);
			expect(container.textContent).toContain("HTTP endpoint configuration");
		} finally {
			await act(async () => root.unmount());
			await window.happyDOM.abort();
			for (const [key, descriptor] of Object.entries(previous)) {
				if (descriptor) Object.defineProperty(globalThis, key, descriptor);
				else Reflect.deleteProperty(globalThis, key);
			}
		}
	});
});
