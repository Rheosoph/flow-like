import { describe, expect, test } from "bun:test";
import type { Action, MicroWidgetInstanceComponent } from "../types";
import {
	type MicroWidgetFrameMount,
	allowsLegacyMicroWidgetFrame,
	microWidgetFrameSrc,
} from "../use-micro-widget-grant";
import {
	dispatchMicroWidgetContractEvent,
	readDeclaredMicroWidgetEvent,
	resolveMicroWidgetContractEvent,
	resolveMicroWidgetEventRoute,
} from "./A2UIMicroWidget";

const action = (name: string): Action => ({ name, context: {} });

const INHERITED_EVENT_NAMES = [
	"constructor",
	"__proto__",
	"toString",
	"valueOf",
	"hasOwnProperty",
];

const contract = (events: unknown): MicroWidgetInstanceComponent["contract"] =>
	({
		contractVersion: 1,
		id: "chart",
		events,
	}) as MicroWidgetInstanceComponent["contract"];

const component = (
	overrides: Partial<MicroWidgetInstanceComponent> = {},
): MicroWidgetInstanceComponent => ({
	id: "sales-chart",
	type: "microWidgetInstance",
	instanceId: "sales-chart",
	packageId: "com.example.sales",
	widgetId: "chart",
	packageVersion: "1.0.0",
	...overrides,
});

describe("resolveMicroWidgetEventRoute", () => {
	test("prefers exact named handlers and preserves their order", () => {
		const route = resolveMicroWidgetEventRoute(
			component({
				eventHandlers: {
					pointSelected: [action("workflow_event"), action("navigate_page")],
				},
				actionBindings: { pointSelected: { workflow: {} } },
				actions: [action("legacy")],
			}),
			"pointSelected",
		);

		expect(route).toEqual({
			kind: "actions",
			actions: [action("workflow_event"), action("navigate_page")],
		});
	});

	test("uses wildcard handlers before widget bindings", () => {
		const route = resolveMicroWidgetEventRoute(
			component({
				eventHandlers: { "*": [action("external_link")] },
				actionBindings: { refreshRequested: { workflow: {} } },
			}),
			"refreshRequested",
		);

		expect(route).toEqual({
			kind: "actions",
			actions: [action("external_link")],
		});
	});

	test("an explicit empty handler suppresses every fallback", () => {
		const route = resolveMicroWidgetEventRoute(
			component({
				eventHandlers: {
					pointSelected: [],
					"*": [action("wildcard")],
				},
				actionBindings: { pointSelected: { workflow: {} } },
				actions: [action("legacy")],
			}),
			"pointSelected",
		);

		expect(route).toEqual({ kind: "actions", actions: [] });
	});

	test("keeps an existing action binding on the widget_event path", () => {
		expect(
			resolveMicroWidgetEventRoute(
				component({
					actionBindings: { pointSelected: { workflow: {} } },
					actions: [action("legacy")],
				}),
				"pointSelected",
			),
		).toEqual({ kind: "widget_event" });
	});

	test("falls back to only actions[0] when no binding exists", () => {
		const route = resolveMicroWidgetEventRoute(
			component({ actions: [action("legacy"), action("previously-inert")] }),
			"pointSelected",
		);

		expect(route).toEqual({
			kind: "actions",
			actions: [action("legacy")],
		});
	});

	test("preserves widget_event diagnostics when nothing is configured", () => {
		expect(resolveMicroWidgetEventRoute(component(), "pointSelected")).toEqual({
			kind: "widget_event",
		});
	});

	test("inherited binding and handler names never select a configured route", () => {
		for (const name of INHERITED_EVENT_NAMES) {
			expect(
				resolveMicroWidgetEventRoute(
					component({
						eventHandlers: {},
						actionBindings: {},
						actions: [action("legacy")],
					}),
					name,
				),
			).toEqual({ kind: "actions", actions: [action("legacy")] });
		}
	});
});

describe("resolveMicroWidgetContractEvent", () => {
	const wildcardPage = (events: unknown) =>
		component({
			contract: contract(events),
			eventHandlers: { "*": [action("workflow_event")] },
			actionBindings: {},
			actions: [action("legacy")],
		});

	for (const name of INHERITED_EVENT_NAMES) {
		test(`drops the undeclared inherited "${name}" event before a page "*" handler`, () => {
			const widget = wildcardPage({ pointSelected: {} });

			expect(readDeclaredMicroWidgetEvent(widget.contract, name)).toBeNull();
			expect(
				resolveMicroWidgetContractEvent(widget, { name, payload: {} }),
			).toEqual({ kind: "dropped", reason: "undeclared" });
		});
	}

	test("drops events a contract only inherits from its prototype", () => {
		const widget = wildcardPage(Object.create({ pointSelected: {} }));

		expect(
			resolveMicroWidgetContractEvent(widget, {
				name: "pointSelected",
				payload: {},
			}),
		).toEqual({ kind: "dropped", reason: "undeclared" });
	});

	test("drops a declared event whose spec is not a plain object", () => {
		for (const spec of [true, 1, "yes", null, []]) {
			const widget = wildcardPage({ pointSelected: spec });

			expect(
				readDeclaredMicroWidgetEvent(widget.contract, "pointSelected"),
			).toBeNull();
			expect(
				resolveMicroWidgetContractEvent(widget, {
					name: "pointSelected",
					payload: {},
				}),
			).toEqual({ kind: "dropped", reason: "undeclared" });
		}
	});

	test("drops events without a contract", () => {
		expect(
			resolveMicroWidgetContractEvent(
				component({ eventHandlers: { "*": [action("workflow_event")] } }),
				{ name: "constructor", payload: {} },
			),
		).toEqual({ kind: "dropped", reason: "undeclared" });
	});

	test("drops a declared event whose payload fails its schema", () => {
		expect(
			resolveMicroWidgetContractEvent(
				wildcardPage({ pointSelected: { payloadSchema: { type: "object" } } }),
				{ name: "pointSelected", payload: "not-an-object" },
			),
		).toMatchObject({ kind: "dropped", reason: "invalid_payload" });
	});

	test("dispatches an own declared event to the page route with its context", () => {
		expect(
			resolveMicroWidgetContractEvent(wildcardPage({ pointSelected: {} }), {
				name: "pointSelected",
				payload: { index: 2 },
			}),
		).toEqual({
			kind: "dispatch",
			route: { kind: "actions", actions: [action("workflow_event")] },
			context: { index: 2, actionId: "pointSelected", payload: { index: 2 } },
		});
	});

	test("an own declared event named like an object member is allowed", () => {
		expect(
			resolveMicroWidgetContractEvent(wildcardPage({ toString: {} }), {
				name: "toString",
				payload: null,
			}),
		).toEqual({
			kind: "dispatch",
			route: { kind: "actions", actions: [action("workflow_event")] },
			context: { actionId: "toString", payload: null },
		});
	});
});

describe("dispatchMicroWidgetContractEvent", () => {
	type ExecuteAction = Parameters<typeof dispatchMicroWidgetContractEvent>[2];

	const dispatch = async (widget: MicroWidgetInstanceComponent) => {
		const resolution = resolveMicroWidgetContractEvent(widget, {
			name: "pointSelected",
			payload: { route: "/attacker", url: "https://attacker.example" },
		});
		if (resolution.kind !== "dispatch") {
			throw new Error("The event was not dispatched");
		}
		const calls: Parameters<ExecuteAction>[] = [];
		await dispatchMicroWidgetContractEvent(
			resolution,
			"sales-chart",
			async (...args) => {
				calls.push(args);
			},
		);
		return { calls, context: resolution.context };
	};

	test("starts every routed action with the micro widget origin", async () => {
		const { calls, context } = await dispatch(
			component({
				contract: contract({ pointSelected: {} }),
				eventHandlers: {
					pointSelected: [action("navigate_page"), action("external_link")],
				},
			}),
		);

		expect(calls).toEqual([
			[
				action("navigate_page"),
				"sales-chart",
				context,
				{ origin: "micro_widget" },
			],
			[
				action("external_link"),
				"sales-chart",
				context,
				{ origin: "micro_widget" },
			],
		]);
	});

	test("starts a widget_event with the micro widget origin", async () => {
		const { calls, context } = await dispatch(
			component({
				contract: contract({ pointSelected: {} }),
				actionBindings: { pointSelected: { workflow: {} } },
			}),
		);

		expect(calls).toEqual([
			[
				{ name: "widget_event", context },
				"sales-chart",
				undefined,
				{ origin: "micro_widget" },
			],
		]);
	});
});

describe("allowsLegacyMicroWidgetFrame", () => {
	test("page contracts without csp may use the pre-grant frame", () => {
		expect(allowsLegacyMicroWidgetFrame(null)).toBe(true);
		expect(allowsLegacyMicroWidgetFrame(undefined)).toBe(true);
		expect(allowsLegacyMicroWidgetFrame(contract({}))).toBe(true);
		expect(
			allowsLegacyMicroWidgetFrame({
				id: "chart",
				capabilities: { downloads: true },
			} as never),
		).toBe(true);
	});

	test("contracts that declare csp or a newer version need a grant-aware server", () => {
		expect(
			allowsLegacyMicroWidgetFrame({
				contractVersion: 2,
				id: "chart",
				csp: { connectSrc: ["https://tiles.example.com"] },
			} as never),
		).toBe(false);
		expect(
			allowsLegacyMicroWidgetFrame({
				contractVersion: 2,
				id: "chart",
			} as never),
		).toBe(false);
		expect(
			allowsLegacyMicroWidgetFrame({
				contractVersion: 1,
				id: "chart",
				csp: {},
			} as never),
		).toBe(false);
	});
});

describe("microWidgetFrameSrc", () => {
	const hash = "b".repeat(64);
	const grant = "1".repeat(64);
	const mount = (
		overrides: Partial<MicroWidgetFrameMount> = {},
	): MicroWidgetFrameMount => ({
		kind: "grant",
		packageId: "com.example.sales",
		packageVersion: "1.0.0",
		bundleHash: hash,
		widgetId: "chart",
		grant,
		runtime: null,
		policy: { downloads: true },
		notice: null,
		invalidReason: null,
		...overrides,
	});
	const desktop = { desktop: true, useHttpBridge: false, apiUrl: null };
	const web = {
		desktop: false,
		useHttpBridge: false,
		apiUrl: (path: string) => `https://api.example.com/api/v1/${path}`,
	};

	test("desktop grant frames carry the grant in the path and never a query", () => {
		expect(microWidgetFrameSrc(mount(), desktop)).toBe(
			`flow-widget://localhost/com.example.sales/${hash}/frame/chart/${grant}`,
		);
		expect(
			microWidgetFrameSrc(mount({ grant: null }), {
				...desktop,
				useHttpBridge: true,
			}),
		).toBe(
			`http://flow-widget.localhost/com.example.sales/${hash}/frame/chart/0`,
		);
		expect(
			microWidgetFrameSrc(mount({ bundleHash: null }), desktop),
		).toBeNull();
	});

	test("web grant frames use the widget-sandbox route once the API origin is known", () => {
		const token = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";
		expect(microWidgetFrameSrc(mount({ grant: token }), web)).toBe(
			`https://api.example.com/api/v1/registry/package/com.example.sales/widget-sandbox/1.0.0/frame/chart/${token}`,
		);
		expect(microWidgetFrameSrc(mount(), { ...web, apiUrl: null })).toBeNull();
	});

	test("web grants with runtime sources carry their runtime component; desktop never does", () => {
		const token = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";
		const runtime = "eyJ0aWxlVXJsIjpbImh0dHBzOi8vYS5leGFtcGxlLmNvbSJdfQ";
		expect(microWidgetFrameSrc(mount({ grant: token, runtime }), web)).toBe(
			`https://api.example.com/api/v1/registry/package/com.example.sales/widget-sandbox/1.0.0/frame/chart/${token}~${runtime}`,
		);
		expect(() =>
			microWidgetFrameSrc(mount({ grant: token, runtime: "a~b" }), web),
		).toThrow("malformed runtime component");
		expect(microWidgetFrameSrc(mount({ runtime }), desktop)).toBe(
			`flow-widget://localhost/com.example.sales/${hash}/frame/chart/${grant}`,
		);
	});

	test("a grant of the wrong platform shape is refused", () => {
		expect(() => microWidgetFrameSrc(mount(), web)).toThrow("malformed grant");
		expect(() =>
			microWidgetFrameSrc(mount({ grant: "not-a-grant" }), desktop),
		).toThrow("malformed grant");
	});

	test("legacy frames keep the pre-grant URL for servers that cannot describe", () => {
		expect(
			microWidgetFrameSrc(mount({ kind: "legacy", grant: null }), desktop),
		).toBe(
			`flow-widget://localhost/com.example.sales/${hash}/frame/chart?downloads=1`,
		);
		expect(
			microWidgetFrameSrc(
				mount({ kind: "legacy", grant: null, policy: {} }),
				web,
			),
		).toBe(
			"https://api.example.com/api/v1/registry/package/com.example.sales/widget-asset/1.0.0/frame/chart",
		);
	});
});
