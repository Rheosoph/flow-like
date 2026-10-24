import { describe, expect, test } from "bun:test";
import type { Surface, SurfaceComponent } from "../types";
import {
	CHAT_EXTERNAL_LINK_ACTION,
	CHAT_SEND_MESSAGE_ACTION,
} from "./chat-actions";
import {
	type ChatSurfaceOptions,
	MAX_CHAT_COMPONENTS,
	createChatSurfaceBuilder,
	sanitizeChatStyle,
} from "./chat-surface";

const STREAMING: ChatSurfaceOptions = {
	streaming: true,
	canSendMessages: true,
};
const SETTLED: ChatSurfaceOptions = { streaming: false, canSendMessages: true };

const lines = (...objects: unknown[]) =>
	objects.map((object) => JSON.stringify(object)).join("\n");

function surfaceOf(body: string, options = SETTLED): Surface {
	const { surface } = createChatSurfaceBuilder("chat").build(body, options);
	if (!surface) throw new Error("no surface");
	return surface;
}

function props(surface: Surface, id: string): Record<string, unknown> {
	const entry = surface.components[id];
	if (!entry) throw new Error(`missing ${id}`);
	return entry.component as unknown as Record<string, unknown>;
}

const RECIPE = lines(
	{ id: "root", type: "column", gap: "12px", children: ["title", "cards"] },
	{
		id: "title",
		type: "text",
		content: "Recipe Remix",
		variant: "heading",
		size: "xl",
	},
	{ id: "cards", type: "grid", columns: 2, gap: "12px", children: ["c1"] },
	{
		id: "c1",
		type: "card",
		title: "Bean & Rice Skillet",
		description: "25 min · serves 2",
		children: ["c1-go"],
	},
	{
		id: "c1-go",
		type: "button",
		label: "Remix this",
		action: {
			name: "send_message",
			context: { message: "Remix the bean skillet" },
		},
	},
);

describe("chat surface streaming", () => {
	test("components appear as they close and keep identity across tokens", () => {
		const builder = createChatSurfaceBuilder("chat");
		const seen = new Map<string, SurfaceComponent>();
		let count = 0;
		for (let end = 0; end <= RECIPE.length; end++) {
			const state = builder.build(RECIPE.slice(0, end), STREAMING);
			const surface = state.surface;
			if (!surface) {
				expect(count).toBe(0);
				continue;
			}
			expect(surface.rootComponentId).toBe("root");
			const real = Object.values(surface.components).filter(
				(component) => component.component.type !== "skeleton",
			);
			expect(real.length).toBeGreaterThanOrEqual(count);
			count = real.length;
			for (const component of real) {
				const previous = seen.get(component.id);
				if (previous) expect(component).toBe(previous);
				seen.set(component.id, component);
			}
		}
		expect(count).toBe(5);
	});

	test("pending text alone returns the same state", () => {
		const builder = createChatSurfaceBuilder("chat");
		const first = builder.build(`${RECIPE}\n{"id":"x","type":"te`, STREAMING);
		expect(first.pending).toBe(true);
		const second = builder.build(`${RECIPE}\n{"id":"x","type":"tex`, STREAMING);
		expect(second).toBe(first);
		const closed = builder.build(
			`${RECIPE}\n{"id":"x","type":"text","content":"x"}`,
			STREAMING,
		);
		expect(closed.pending).toBe(false);
		expect(closed.surface?.components.root).toBe(
			first.surface?.components.root,
		);
	});

	test("unresolved children render a skeleton while streaming, nothing once settled", () => {
		const body = lines(
			{ id: "root", type: "row", children: ["a", "later"] },
			{ id: "a", type: "text", content: "A" },
		);
		const streaming = surfaceOf(body, STREAMING);
		expect(props(streaming, "later").type).toBe("skeleton");
		const settled = surfaceOf(body, SETTLED);
		expect(settled.components.later).toBeUndefined();
	});

	test("root is `root`, else the first component", () => {
		expect(
			surfaceOf(
				lines({ id: "a", type: "divider" }, { id: "b", type: "spacer" }),
			).rootComponentId,
		).toBe("a");
		expect(
			surfaceOf(
				lines({ id: "a", type: "divider" }, { id: "root", type: "row" }),
			).rootComponentId,
		).toBe("root");
	});

	test("nothing renders until a component exists", () => {
		const state = createChatSurfaceBuilder("chat").build(
			'{"data":{"x":1}}\nnot json',
			SETTLED,
		);
		expect(state.surface).toBeNull();
		expect(state.data).toEqual({ x: 1 });
	});

	test("duplicate ids keep the first component; the count is capped", () => {
		const duplicated = surfaceOf(
			lines(
				{ id: "root", type: "text", content: "first" },
				{ id: "root", type: "text", content: "second" },
			),
		);
		expect(props(duplicated, "root").content).toEqual({
			literalString: "first",
		});
		const many = Array.from(
			{ length: MAX_CHAT_COMPONENTS + 5 },
			(_, index) => ({
				id: `c${index}`,
				type: "divider",
			}),
		);
		const state = createChatSurfaceBuilder("chat").build(
			lines(...many),
			SETTLED,
		);
		expect(state.componentCount).toBe(MAX_CHAT_COMPONENTS);
		expect(state.droppedCount).toBe(5);
	});
});

describe("chat policy", () => {
	test("only chat component types render; dropped ids get no skeleton", () => {
		const body = lines(
			{ id: "root", type: "column", children: ["frame", "w", "ok"] },
			{ id: "frame", type: "iframe", src: "https://example.com" },
			{ id: "w", type: "widgetInstance", instanceId: "i", widgetId: "w" },
			{ id: "ok", type: "badge", content: "ok" },
		);
		const surface = surfaceOf(body, STREAMING);
		expect(surface.components.frame).toBeUndefined();
		expect(surface.components.w).toBeUndefined();
		expect(props(surface, "ok").type).toBe("badge");
	});

	test("strips every action but send_message and external_link", () => {
		const surface = surfaceOf(
			lines({
				id: "root",
				type: "button",
				label: "Go",
				actions: [
					{
						name: "workflow_event",
						context: { nodeId: "n", appId: "a", boardId: "b" },
					},
					{ name: "navigate_page", context: { route: "/admin" } },
					{
						name: "send_message",
						context: { message: " Remix ", extra: "dropped" },
						pageAction: { id: "x" },
					},
					{ name: "external_link", context: { url: "javascript:alert(1)" } },
					{ name: "external_link", context: { url: "https://example.com/a" } },
				],
				eventHandlers: {
					click: [{ name: "widget_event", context: { actionId: "x" } }],
					hover: [],
				},
			}),
		);
		const button = props(surface, "root");
		expect(button.actions).toEqual([
			{ name: CHAT_SEND_MESSAGE_ACTION, context: { message: "Remix" } },
			{
				name: CHAT_EXTERNAL_LINK_ACTION,
				context: { url: "https://example.com/a" },
			},
		]);
		expect(button.eventHandlers).toEqual({ hover: [] });
	});

	test("a text field's action runs on Enter, not on blur", () => {
		const field = props(
			surfaceOf(
				lines({
					id: "root",
					type: "textField",
					value: "",
					action: {
						name: "send_message",
						context: { message: "Plan for {{root}}" },
					},
				}),
			),
			"root",
		);
		expect(field.actions).toBeUndefined();
		expect(field.eventHandlers).toEqual({
			submit: [
				{
					name: CHAT_SEND_MESSAGE_ACTION,
					context: { message: "Plan for {{root}}" },
				},
			],
		});
	});

	test("links open http(s) URLs outside the app; anything else becomes text", () => {
		const surface = surfaceOf(
			lines(
				{ id: "root", type: "column", children: ["web", "app", "act"] },
				{
					id: "web",
					type: "link",
					href: "https://example.com/x",
					label: "Docs",
					target: "_self",
					route: "/settings",
				},
				{ id: "app", type: "link", href: "/settings", label: "Settings" },
				{
					id: "act",
					type: "link",
					label: "Ask",
					action: { name: "send_message", context: { message: "More" } },
				},
			),
		);
		expect(props(surface, "web")).toMatchObject({
			type: "link",
			href: { literalString: "https://example.com/x" },
			external: true,
			target: "_blank",
		});
		expect(props(surface, "web").route).toBeUndefined();
		expect(props(surface, "app")).toEqual({
			type: "text",
			content: { literalString: "Settings" },
		});
		expect(props(surface, "act")).toMatchObject({
			type: "link",
			href: { literalString: "" },
		});
	});

	test("send_message buttons are disabled without a follow-up sender", () => {
		const builder = createChatSurfaceBuilder("chat");
		const enabled = builder.build(RECIPE, SETTLED).surface;
		const disabled = builder.build(RECIPE, {
			streaming: false,
			canSendMessages: false,
		}).surface;
		if (!enabled || !disabled) throw new Error("no surface");
		expect(props(enabled, "c1-go").disabled).toBeUndefined();
		expect(props(disabled, "c1-go").disabled).toEqual({ literalBool: true });
		expect(disabled.components.title).toBe(enabled.components.title);
	});

	test("cycles and shared children render each component once", () => {
		const surface = surfaceOf(
			lines(
				{ id: "root", type: "column", children: ["a", "a", "b"] },
				{ id: "a", type: "row", children: ["root", "c"] },
				{ id: "b", type: "row", children: ["c"] },
				{ id: "c", type: "divider" },
			),
		);
		expect(props(surface, "root").children).toEqual({
			explicitList: ["a", "b"],
		});
		expect(props(surface, "a").children).toEqual({ explicitList: ["c"] });
		expect(props(surface, "b").children).toBeUndefined();
	});

	test("data lines merge into the data model", () => {
		const builder = createChatSurfaceBuilder("chat");
		const state = builder.build(
			[
				lines(
					{ data: { recipe: { servings: 2 }, tags: ["a"] } },
					{ id: "root", type: "slider", value: { path: "$.recipe.servings" } },
				),
				'{"data":{"recipe":{"time":25},"tags":["b"],"__proto__":{"x":1}}}',
			].join("\n"),
			SETTLED,
		);
		expect(Object.getPrototypeOf(state.data)).toBe(Object.prototype);
		expect(state.data).toEqual({
			recipe: { servings: 2, time: 25 },
			tags: ["b"],
		});
		expect(state.surface?.dataModel).toEqual([
			{ path: "recipe", value: { servings: 2, time: 25 } },
			{ path: "tags", value: ["b"] },
		]);
		expect(({} as Record<string, unknown>).x).toBeUndefined();
	});

	test("plain authored props lower through validation (options, children, hidden)", () => {
		const surface = surfaceOf(
			lines(
				{ id: "root", type: "column", children: ["pick"], hidden: false },
				{
					id: "pick",
					type: "select",
					value: "s",
					options: ["s", { value: "m", label: "Medium" }],
				},
			),
		);
		expect(props(surface, "root").hidden).toEqual({ literalBool: false });
		expect(props(surface, "pick").options).toEqual({
			literalOptions: [
				{ value: "s", label: "s" },
				{ value: "m", label: "Medium" },
			],
		});
	});

	test("custom CSS escapes are removed from styles", () => {
		const surface = surfaceOf(
			lines({
				id: "root",
				type: "text",
				content: "x",
				style: {
					className:
						"p-4 bg-[url(file:///etc/x)] [:root:has(&)_*]:hidden fixed z-50",
					cursor: "url(https://example.com/c.png), auto",
					background: { image: { url: "file:///etc/passwd" } },
				},
			}),
		);
		expect(surface.components.root?.style).toEqual({
			className: "p-4 fixed z-50",
		});
	});

	test("grids drop columns below a readable width and rows wrap by default", () => {
		const surface = surfaceOf(
			lines(
				{ id: "root", type: "column", children: ["g", "fixed", "r", "nowrap"] },
				{ id: "g", type: "grid", columns: 3, gap: "12px", children: [] },
				{ id: "fixed", type: "grid", columns: "1fr 2fr", children: [] },
				{ id: "r", type: "row", children: [] },
				{ id: "nowrap", type: "row", wrap: false, children: [] },
			),
		);
		expect(props(surface, "g").columns).toEqual({
			literalString:
				"repeat(auto-fill, minmax(min(100%, max(13rem, calc((100% - 2 * 12px) / 3))), 1fr))",
		});
		expect(props(surface, "fixed").columns).toEqual({
			literalString: "1fr 2fr",
		});
		expect(props(surface, "r").wrap).toEqual({ literalBool: true });
		expect(props(surface, "nowrap").wrap).toEqual({ literalBool: false });
	});
});

describe("sanitizeChatStyle", () => {
	test("keeps https and inline image backgrounds, drops other sources and bindings", () => {
		const https = { image: { url: { literalString: "https://x.test/a.png" } } };
		expect(sanitizeChatStyle({ background: https })).toEqual({
			background: https,
		});
		const inline = {
			image: { url: { literalString: "data:image/png;base64,AAAA" } },
		};
		expect(sanitizeChatStyle({ background: inline })).toEqual({
			background: inline,
		});
		for (const url of [
			{ literalString: "asset://localhost/x.png" },
			{ literalString: "http://x.test/a.png" },
			{ path: "$.image" },
		]) {
			expect(
				sanitizeChatStyle({ background: { image: { url } } }),
			).toBeUndefined();
		}
	});

	test("drops values that load resources or hide escapes", () => {
		expect<unknown>(
			sanitizeChatStyle({
				background: {
					gradient: {
						type: "linear",
						stops: [
							{ color: "red), url(https://evil.test/x", position: 0 },
							{ color: "blue", position: 100 },
						],
					},
				},
				filter: "u\\72l(#x)",
				color: "red",
			}),
		).toEqual({
			background: {
				gradient: {
					type: "linear",
					stops: [{ position: 0 }, { color: "blue", position: 100 }],
				},
			},
			color: "red",
		});
		expect(sanitizeChatStyle("p-4")).toBeUndefined();
	});
});
