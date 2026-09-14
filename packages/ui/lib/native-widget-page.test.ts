import { describe, expect, mock, test } from "bun:test";
import type { Surface, SurfaceComponent } from "../components/a2ui/types";
import type { IPage, IPageBootstrap } from "../state/backend-state/page-state";
import type { NativeWidgetPageNode } from "./native-widget";
import {
	loadNativeWidgetPage,
	nativeWidgetPageQuery,
	projectNativeWidgetPage,
	resolveNativeWidgetBinding,
	resolveNativeWidgetPageMedia,
} from "./native-widget-page";

function component(
	id: string,
	type: string,
	props: Record<string, unknown> = {},
): SurfaceComponent {
	return { id, component: { id, type, ...props } } as SurfaceComponent;
}
function page(...components: SurfaceComponent[]): IPage {
	return {
		id: "page",
		name: "Orders",
		layoutType: "stack",
		content: [],
		components,
		createdAt: "2026-09-01T00:00:00Z",
		updatedAt: "2026-09-12T00:00:00Z",
	};
}
const target = {
	appId: "app",
	path: "/orders",
	queryParams: [{ name: "status", value: "new & ready" }],
};
const text = (id: string, content: unknown) =>
	component(id, "text", { content });
const root = (...ids: string[]) =>
	component("root", "column", { children: { explicitList: ids } });
const nodes = (node?: NativeWidgetPageNode): NativeWidgetPageNode[] =>
	node ? [node, ...(node.children ?? []).flatMap(nodes)] : [];
function surface(stored: IPage, dataModel: Surface["dataModel"] = []): Surface {
	return {
		id: stored.id,
		rootComponentId: "root",
		components: Object.fromEntries(
			stored.components.map((item) => [item.id, item]),
		),
		dataModel,
	};
}

describe("native page binding resolution", () => {
	test("separates legacy routing controls while preserving reserved app query names", () => {
		expect(
			nativeWidgetPageQuery(
				"id=app&route=%2Forders&event=page-event&tag=a%2Bb",
			),
		).toBe("tag=a%2Bb");
		expect(
			nativeWidgetPageQuery(
				"id=app&appQuery=id%3Drecord%26route%3Dcustom%26tag%3Da%252Bb",
			),
		).toBe("id=record&route=custom&tag=a%2Bb");
	});
	test("matches literals, nested paths, indexed records and defaults", () => {
		const data = { orders: [{ customer: { name: "Ada" } }], enabled: false };
		expect(
			resolveNativeWidgetBinding({ path: "/orders/0/customer/name" }, data),
		).toBe("Ada");
		expect(
			resolveNativeWidgetBinding({ path: "$.orders[0].customer.name" }, data),
		).toBe("Ada");
		expect(
			resolveNativeWidgetBinding({ path: "enabled", defaultValue: true }, data),
		).toBe(false);
		expect(
			resolveNativeWidgetBinding(
				{ path: "missing", defaultValue: "Waiting" },
				data,
			),
		).toBe("Waiting");
		expect(resolveNativeWidgetBinding({ literalJson: "[1,2]" }, data)).toEqual([
			1, 2,
		]);
		expect(
			resolveNativeWidgetBinding({ literalJson: "invalid" }, data),
		).toBeUndefined();
		expect(
			resolveNativeWidgetBinding({ path: "$item.customer.name" }, data, {
				item: data.orders[0],
			}),
		).toBe("Ada");
		expect(
			resolveNativeWidgetBinding({ path: "$index" }, data, { index: 3 }),
		).toBe(3);
	});
	test("does not read inherited or prototype properties", () => {
		const data = Object.create({ inherited: "secret" });
		for (const path of [
			"inherited",
			"__proto__",
			"constructor.name",
			"prototype",
		])
			expect(resolveNativeWidgetBinding({ path }, data)).toBeUndefined();
	});
});

describe("native app page projection", () => {
	test("clips multilingual text without producing partial emoji for the Swift decoder", () => {
		const stored = page(
			component("root", "card", {
				description: { literalString: `${"a".repeat(79)}😀` },
				children: { explicitList: ["text", "table", "chart"] },
			}),
			text("text", { literalString: `${"a".repeat(999)}😀` }),
			component("table", "table", {
				columns: {
					literalJson: JSON.stringify([
						{ id: "value", header: { literalString: `${"a".repeat(49)}😀` } },
					]),
				},
				data: {
					literalJson: JSON.stringify([{ value: `${"a".repeat(79)}😀` }]),
				},
			}),
			component("chart", "nivoChart", {
				chartType: { literalString: "pie" },
				data: {
					literalJson: JSON.stringify([
						{ id: `${"a".repeat(99)}😀`, value: 1 },
					]),
				},
			}),
		);
		const result = projectNativeWidgetPage(stored, target);
		expect(result.root?.value).toBe("a".repeat(79));
		expect(result.root?.children?.[0].text).toBe("a".repeat(999));
		expect(result.root?.children?.[1].table).toEqual({
			columns: ["a".repeat(49)],
			rows: [["a".repeat(79)]],
		});
		expect(result.root?.children?.[2].chart?.points[0].label).toBe(
			"a".repeat(99),
		);
		expect(JSON.stringify(result)).not.toMatch(/\\u[dD][89aAbB][0-9a-fA-F]{2}/);
	});
	test("projects the chosen container, semantic text and hidden bindings", () => {
		const stored = page(
			root("card", "other"),
			component("card", "card", {
				title: { literalString: "Today" },
				children: { explicitList: ["total", "hidden"] },
			}),
			text("total", { literalNumber: 42 }),
			component("hidden", "text", {
				content: { literalString: "Secret" },
				hidden: { literalString: "true" },
			}),
			text("other", { literalString: "Outside" }),
		);
		const result = projectNativeWidgetPage(stored, {
			...target,
			containerId: "card",
		});
		expect(result.supported).toBe(true);
		expect(result.root?.kind).toBe("card");
		expect(result.root?.text).toBe("Today");
		expect(result.root?.children?.map((node) => node.text)).toEqual(["42"]);
		expect(JSON.stringify(result)).not.toContain("Secret");
		expect(JSON.stringify(result)).not.toContain("Outside");
	});
	test("resolves captured data and repeated children with unique identifiers", () => {
		const stored = page(
			component("root", "column", {
				children: {
					template: { dataPath: "/orders", templateComponentId: "row" },
				},
			}),
			component("row", "row", {
				children: { explicitList: ["customer", "index"] },
			}),
			text("customer", { path: "$item.customer" }),
			text("index", { path: "$index" }),
		);
		const result = projectNativeWidgetPage(stored, target, {
			surface: surface(stored, [
				{ path: "orders", value: [{ customer: "Ada" }, { customer: "Lin" }] },
			]),
		});
		const projected = nodes(result.root);
		expect(
			projected.filter((node) => node.kind === "text").map((node) => node.text),
		).toEqual(["Ada", "0", "Lin", "1"]);
		expect(new Set(projected.map((node) => node.id)).size).toBe(
			projected.length,
		);
		expect(result.warnings).toEqual([]);
	});
	test("uses live input state when it differs from the initial data model", () => {
		const stored = page(root("value"), text("value", { path: "/value" }));
		const result = projectNativeWidgetPage(stored, target, {
			surface: surface(stored, [{ path: "value", value: "old" }]),
			data: { value: "edited" },
		});
		expect(result.root?.children?.[0].text).toBe("edited");
	});
	test("expands reusable widget props and runtime child updates without changing their definition", () => {
		const stored = page(
			component("root", "widgetInstance", {
				instanceId: "instance",
				widgetId: "widget",
				exposedPropValues: { label: "Configured" },
				runtimeChildUpdates: { name: [{ type: "setText", text: "Current" }] },
			}),
		);
		stored.widgetRefs = {
			instance: {
				id: "widget",
				name: "Metric",
				rootComponentId: "name",
				components: [text("name", { literalString: "Original" })],
				exposedProps: [
					{
						id: "label",
						targetComponentId: "name",
						propertyPath: "content",
						propType: "String",
					},
				],
				tags: [],
				createdAt: stored.createdAt,
				updatedAt: stored.updatedAt,
			},
		};
		const result = projectNativeWidgetPage(stored, target);
		expect(result.root?.text).toBe("Current");
		expect(stored.widgetRefs.instance.components[0].component).toMatchObject({
			content: { literalString: "Original" },
		});
	});
	test("exports media sources separately and drops raw actions, capabilities and CSS", () => {
		const stored = page(
			root("photo", "icon", "button"),
			component("photo", "image", {
				src: { literalString: "https://assets.test/private.png?token=secret" },
			}),
			component("icon", "icon", { name: { literalString: "ArrowUpRight" } }),
			component("button", "button", {
				label: { literalString: "Approve" },
				actions: [
					{
						name: "run",
						context: { token: "secret-context" },
						pageAction: {
							actionId: "lda1_secret",
							capabilityJwt: "secret-capability",
						},
					},
				],
				style: { className: "custom-css" },
			}),
		);
		const result = projectNativeWidgetPage(stored, target);
		expect(result.media.map((item) => item.source)).toEqual([
			"https://assets.test/private.png?token=secret",
			"arrow-up-right",
		]);
		const serialized = JSON.stringify(result.root);
		for (const secret of [
			"assets.test",
			"secret",
			"capability",
			"custom-css",
			"pageAction",
		])
			expect(serialized).not.toContain(secret);
		expect(result.root?.children?.[2].action).toEqual({
			kind: "open_app",
			...target,
		});
	});
	test("preserves raw query values for internal links and keeps external links in the app", () => {
		const stored = page(
			root("link", "external"),
			component("link", "link", {
				href: { literalString: "/detail?tag=a%2Bb" },
				label: { literalString: "Detail" },
				queryParams: { literalJson: JSON.stringify({ search: "café & 50%" }) },
			}),
			component("external", "link", {
				href: { literalString: "https://external.test" },
				label: { literalString: "External" },
			}),
		);
		const result = projectNativeWidgetPage(stored, target);
		expect(result.root?.children?.[0].action).toMatchObject({
			appId: "app",
			path: "/detail",
			queryParams: [
				{ name: "tag", value: "a+b" },
				{ name: "search", value: "café & 50%" },
			],
		});
		expect(result.root?.children?.[1].action).toMatchObject({
			appId: "app",
			path: "/orders",
		});
	});
	test("projects compact tables and supported chart data without copying row objects", () => {
		const stored = page(
			root("table", "chart"),
			component("table", "table", {
				columns: {
					literalJson: JSON.stringify([
						{
							id: "customer",
							header: { literalString: "Customer" },
							accessor: { literalString: "customer.name" },
						},
					]),
				},
				data: {
					literalJson: JSON.stringify([
						{ customer: { name: "Ada", private: "hidden" } },
					]),
				},
			}),
			component("chart", "nivoChart", {
				chartType: { literalString: "line" },
				data: {
					literalJson: JSON.stringify([
						{
							id: "Sales",
							data: [
								{ x: "Jan", y: 10 },
								{ x: "Feb", y: 14 },
							],
						},
					]),
				},
			}),
		);
		const result = projectNativeWidgetPage(stored, target);
		expect(result.root?.children?.[0].table).toEqual({
			columns: ["Customer"],
			rows: [["Ada"]],
		});
		expect(
			result.root?.children?.[1].chart?.points.map((point) => point.value),
		).toEqual([10, 14]);
		expect(JSON.stringify(result.root)).not.toContain("hidden");
	});
	test("reports unsupported interactive content and missing data without executing it", () => {
		const stored = page(
			root("camera", "html", "missing"),
			component("camera", "cameraView"),
			component("html", "iframe", {
				src: { literalString: "https://unsafe.test" },
			}),
			text("missing", { path: "notLoaded" }),
		);
		stored.onLoadEventId = "load-event";
		const result = projectNativeWidgetPage(stored, target);
		expect(result.supported).toBe(false);
		expect(result.root).toBeUndefined();
		expect(result.requiresCapture).toBe(true);
		expect(
			result.warnings.some((warning) => warning.includes("cameraView")),
		).toBe(true);
		expect(
			result.warnings.some((warning) => warning.includes("not loaded")),
		).toBe(true);
	});
	test("bounds repeated content, media, tables and cycles", () => {
		const ids = Array.from({ length: 100 }, (_, index) => `text-${index}`);
		const stored = page(
			root(...ids),
			...ids.map((id) => text(id, { literalString: id })),
		);
		const result = projectNativeWidgetPage(stored, target);
		expect(nodes(result.root).length).toBeLessThanOrEqual(60);
		expect(result.warnings.some((warning) => warning.includes("larger"))).toBe(
			true,
		);
		const cycle = projectNativeWidgetPage(page(root("root")), target);
		expect(
			cycle.warnings.some((warning) => warning.includes("recursive")),
		).toBe(true);
		const images = page(
			root("a", "b", "c", "d", "e"),
			...["a", "b", "c", "d", "e"].map((id) =>
				component(id, "image", { src: { literalString: `/${id}.png` } }),
			),
		);
		expect(projectNativeWidgetPage(images, target).media).toHaveLength(4);
		const oversized = page(
			root(...ids),
			...ids.map((id) => text(id, { literalString: "文".repeat(2000) })),
		);
		const bounded = projectNativeWidgetPage(oversized, target);
		expect(
			new TextEncoder().encode(JSON.stringify(bounded.root)).length,
		).toBeLessThan(48 * 1024);
		expect(
			bounded.warnings.some((warning) => warning.includes("too much content")),
		).toBe(true);
		expect(
			projectNativeWidgetPage(page(root("constructor")), target).supported,
		).toBe(false);
	});
});

describe("authenticated native page loading", () => {
	test("reads only the authenticated bootstrap and keeps the exact page revision", async () => {
		const stored = page(
			root("title"),
			text("title", { literalString: "Published" }),
		);
		stored.onLoadEventId = "must-not-run";
		const getPageBootstrap = mock(
			async () =>
				({
					page: stored,
					event: { id: "page-event" },
					canonicalRoute: "/orders",
					revision: "content-v2",
					executionRevision: "authority-v3",
				}) as IPageBootstrap,
		);
		const result = await loadNativeWidgetPage({ getPageBootstrap }, target);
		expect(getPageBootstrap).toHaveBeenCalledWith("app", "/orders");
		expect(result.pageRevision).toBe('["content-v2","authority-v3"]');
		expect(result.requiresCapture).toBe(true);
		expect(result.root?.children?.[0].text).toBe("Published");
	});
	test("does not downgrade denied or missing routes to a draft page", async () => {
		const denied = {
			getPageBootstrap: mock(async (): Promise<IPageBootstrap> => {
				throw new Error("Access denied");
			}),
		};
		await expect(loadNativeWidgetPage(denied, target)).rejects.toThrow(
			"Access denied",
		);
		await expect(
			loadNativeWidgetPage(
				{
					getPageBootstrap: async () =>
						({
							event: {},
							routeMiss: true,
							page: page(text("root", { literalString: "Fallback" })),
						}) as IPageBootstrap,
				},
				target,
			),
		).rejects.toThrow("does not match");
	});
});

describe("native page storage artwork", () => {
	test("signs app-relative image paths with the current backend without signing icons or emoji", async () => {
		const stored = page(
			root("photo", "same", "icon", "emoji"),
			component("photo", "image", {
				src: { literalString: "storage://images/order.png" },
			}),
			component("same", "image", {
				src: { literalString: "images/order.png" },
			}),
			component("icon", "icon", { name: { literalString: "bell" } }),
			component("emoji", "image", { src: { literalString: "🦊" } }),
		);
		const projection = projectNativeWidgetPage(stored, target);
		const downloadStorageItems = mock(
			async (_appId: string, _paths: string[]) => [
				{
					prefix: "images/order.png",
					url: "https://storage.test/order.png?signed=account-a",
				},
			],
		);
		const result = await resolveNativeWidgetPageMedia(
			{ downloadStorageItems },
			"app",
			projection,
		);
		expect(downloadStorageItems).toHaveBeenCalledWith("app", [
			"images/order.png",
		]);
		expect(result.media.map((item) => item.source)).toEqual([
			"https://storage.test/order.png?signed=account-a",
			"https://storage.test/order.png?signed=account-a",
			"bell",
			"🦊",
		]);
		expect(JSON.stringify(result.root)).not.toContain("signed=");
		expect(projection.media[0].source).toBe("storage://images/order.png");
	});
	test("does not reuse a previous account's signed URL", async () => {
		const projection = projectNativeWidgetPage(
			page(
				component("root", "image", {
					src: { literalString: "images/order.png" },
				}),
			),
			target,
		);
		const first = await resolveNativeWidgetPageMedia(
			{
				downloadStorageItems: async () => [
					{ prefix: "images/order.png", url: "https://storage.test/a" },
				],
			},
			"app",
			projection,
		);
		const second = await resolveNativeWidgetPageMedia(
			{
				downloadStorageItems: async () => [
					{ prefix: "images/order.png", url: "https://storage.test/b" },
				],
			},
			"app",
			projection,
		);
		expect(first.media[0].source).toBe("https://storage.test/a");
		expect(second.media[0].source).toBe("https://storage.test/b");
	});
	test("retains text when a storage image is unavailable", async () => {
		const projection = projectNativeWidgetPage(
			page(
				root("text", "photo"),
				text("text", { literalString: "Order ready" }),
				component("photo", "image", {
					src: { literalString: "images/missing.png" },
				}),
			),
			target,
		);
		const result = await resolveNativeWidgetPageMedia(
			{
				downloadStorageItems: async () => [
					{ prefix: "images/missing.png", error: "Missing" },
				],
			},
			"app",
			projection,
		);
		expect(result.media).toEqual([]);
		expect(result.root?.children?.[0].text).toBe("Order ready");
		expect(result.warnings).toContain(
			"Some page images could not be loaded from app storage.",
		);
	});
	test("cancels a storage read promptly and never uses late credentials", async () => {
		const projection = projectNativeWidgetPage(
			page(
				component("root", "image", {
					src: { literalString: "images/order.png" },
				}),
			),
			target,
		);
		let finish!: (result: Array<{ prefix: string; url: string }>) => void;
		const downloadStorageItems = mock(
			() =>
				new Promise<Array<{ prefix: string; url: string }>>((resolve) => {
					finish = resolve;
				}),
		);
		const controller = new AbortController();
		const result = resolveNativeWidgetPageMedia(
			{ downloadStorageItems },
			"app",
			projection,
			controller.signal,
		);
		controller.abort();
		await expect(result).rejects.toMatchObject({ name: "AbortError" });
		finish([{ prefix: "images/order.png", url: "https://storage.test/late" }]);
		await expect(
			resolveNativeWidgetPageMedia(
				{ downloadStorageItems },
				"app",
				projection,
				controller.signal,
			),
		).rejects.toMatchObject({ name: "AbortError" });
		expect(downloadStorageItems).toHaveBeenCalledTimes(1);
	});
});
