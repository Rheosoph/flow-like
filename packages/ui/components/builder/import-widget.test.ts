import { describe, expect, mock, test } from "bun:test";
import type { IMetadata } from "../../lib/schema/bit/bit";
import type {
	IWidget,
	IWidgetState,
} from "../../state/backend-state/widget-state";
import type { SurfaceComponent } from "../a2ui/types";
import { importWidget } from "./import-widget";

function component(
	id: string,
	props: Record<string, unknown>,
): SurfaceComponent {
	return { id, component: props } as unknown as SurfaceComponent;
}

function widget(id: string, components?: SurfaceComponent[]): IWidget {
	return {
		id,
		name: `Widget ${id}`,
		rootComponentId: "root",
		components: components ?? [
			component("root", { type: "column", children: { explicitList: [] } }),
		],
		dataModel: [],
		customizationOptions: [],
		tags: ["shared"],
		version: [3, 2, 1],
		createdAt: "2025-01-01T00:00:00.000Z",
		updatedAt: "2025-01-01T00:00:00.000Z",
	};
}

function backend(sources: Record<string, IWidget>) {
	const saved = new Map<string, IWidget>();
	const getWidget = mock(async (_app: string, id: string) => sources[id]);
	const getWidgetAuthoritative = mock(async (_app: string, id: string) => {
		if (!sources[id]) throw new Error(`Missing widget ${id}`);
		return sources[id];
	});
	const updateWidget = mock(async (_app: string, value: IWidget) => {
		saved.set(value.id, structuredClone(value));
	});
	const pushWidgetMeta = mock(
		async (_app: string, _id: string, _meta: IMetadata) => {},
	);
	const getWidgetMeta = mock(
		async (_app: string, _id: string): Promise<IMetadata> => {
			throw new Error("No metadata");
		},
	);
	const deleteWidget = mock(async (_app: string, id: string) => {
		saved.delete(id);
	});
	return {
		saved,
		getWidget,
		getWidgetAuthoritative,
		getWidgetMeta,
		updateWidget,
		pushWidgetMeta,
		deleteWidget,
		state: {
			getWidget,
			getWidgetAuthoritative,
			getWidgetMeta,
			updateWidget,
			pushWidgetMeta,
			deleteWidget,
		} as unknown as IWidgetState,
	};
}

const encode = (value: unknown) =>
	new TextEncoder().encode(JSON.stringify(value));
const decode = (value: Uint8Array | undefined) =>
	JSON.parse(new TextDecoder().decode(value));

describe("widget imports", () => {
	test("keeps widgets already in the target app unchanged", async () => {
		const source = widget("local");
		const api = backend({ local: source });
		expect(await importWidget(api.state, "target", "local", "target")).toBe(
			source,
		);
		expect(api.getWidget).toHaveBeenCalledWith("target", "local");
		expect(api.getWidgetAuthoritative).not.toHaveBeenCalled();
		expect(api.updateWidget).not.toHaveBeenCalled();
	});

	test("copies components, references and byte defaults while preserving content and semantic IDs", async () => {
		const source = widget("source", [
			component("card", {
				type: "column",
				id: "card",
				children: { explicitList: ["input", "tabs"] },
			}),
			component("input", {
				type: "text",
				id: "input",
				text: { literalString: "input" },
				onClick: {
					name: "submit",
					context: {
						targetComponentId: { literalString: "input" },
						sourceComponentId: { path: "/input", defaultValue: "card" },
						payload: {
							literalJson: JSON.stringify({
								componentId: "input",
								text: "input",
							}),
						},
						appId: { literalString: "source-app" },
					},
				},
			}),
			component("tabs", {
				type: "tabs",
				tabs: [{ id: "semantic-tab", contentComponentId: "input" }],
				children: {
					template: {
						templateComponentId: "input",
						dataPath: "/input",
						itemIdPath: "input",
					},
				},
				child: "input",
				entryPointChild: "card",
				contentChild: "input",
				baseComponentId: "card",
				element_ids: ["input", "outside"],
			}),
		]);
		source.rootComponentId = "card";
		source.dataModel = [
			{
				key: "input",
				value: {
					text: "input",
					path: "/input",
					ids: [1, 2, 3],
					componentId: "input",
				},
			},
		];
		source.actions = [
			{
				id: "submit",
				label: "Submit",
				contextSchema: [
					{
						name: "input",
						label: "Input",
						fieldType: "String",
						defaultPath: "/input",
					},
				],
			},
		];
		source.exposedProps = [
			{
				id: "title",
				label: "Title",
				targetComponentId: "input",
				propertyPath: "text",
				propType: "String",
				defaultValue: encode("input"),
			},
			{
				id: "target",
				label: "Target",
				targetComponentId: "tabs",
				propertyPath: "componentId",
				propType: "String",
				defaultValue: encode({ literalString: "input" }),
			},
			{
				id: "context",
				label: "Context",
				targetComponentId: "input",
				propertyPath: "onClick.context",
				propType: "Json",
				defaultValue: Array.from(
					encode({ componentId: "input", appId: "source-app", label: "input" }),
				) as unknown as Uint8Array,
			},
		];
		source.customizationOptions = [
			{
				id: "context",
				label: "Context",
				customizationType: "Json",
				validations: [],
				defaultValue: encode({ targetComponentId: "card", text: "card" }),
			},
		];
		const before = structuredClone(source);
		const api = backend({ source });
		const imported = await importWidget(
			api.state,
			"source-app",
			"source",
			"target-app",
		);
		const [root, input, tabs] = imported.components;
		expect(imported.id).not.toBe(source.id);
		expect(imported.version).toEqual([0, 0, 1]);
		expect(imported.rootComponentId).toBe("root");
		expect(root.id).toBe("root");
		expect(input.id).not.toBe("input");
		expect(tabs.id).not.toBe("tabs");
		expect(root.component).toMatchObject({
			id: "root",
			children: { explicitList: [input.id, tabs.id] },
		});
		expect(input.component).toMatchObject({
			id: input.id,
			text: { literalString: "input" },
			onClick: {
				name: "submit",
				context: {
					targetComponentId: { literalString: input.id },
					sourceComponentId: { path: "/input", defaultValue: "root" },
					appId: { literalString: "source-app" },
					payload: {
						literalJson: JSON.stringify({
							componentId: input.id,
							text: "input",
						}),
					},
				},
			},
		});
		expect(tabs.component).toMatchObject({
			tabs: [{ id: "semantic-tab", contentComponentId: input.id }],
			children: {
				template: {
					templateComponentId: input.id,
					dataPath: "/input",
					itemIdPath: "input",
				},
			},
			child: input.id,
			entryPointChild: "root",
			contentChild: input.id,
			baseComponentId: "root",
			element_ids: [input.id, "outside"],
		});
		expect(imported.dataModel).toEqual([
			{
				key: "input",
				value: {
					text: "input",
					path: "/input",
					ids: [1, 2, 3],
					componentId: input.id,
				},
			},
		]);
		expect(imported.actions).toEqual(source.actions);
		expect(imported.exposedProps?.map((prop) => prop.id)).toEqual([
			"title",
			"target",
			"context",
		]);
		expect(imported.exposedProps?.[0].targetComponentId).toBe(input.id);
		expect(decode(imported.exposedProps?.[0].defaultValue)).toBe("input");
		expect(decode(imported.exposedProps?.[1].defaultValue)).toEqual({
			literalString: input.id,
		});
		expect(
			JSON.parse(
				new TextDecoder().decode(
					new Uint8Array(imported.exposedProps?.[2].defaultValue ?? []),
				),
			),
		).toEqual({ componentId: input.id, appId: "source-app", label: "input" });
		expect(decode(imported.customizationOptions[0].defaultValue)).toEqual({
			targetComponentId: "root",
			text: "card",
		});
		expect(api.getWidgetAuthoritative).toHaveBeenCalledWith(
			"source-app",
			"source",
			undefined,
		);
		expect(api.updateWidget).toHaveBeenCalledWith("target-app", imported);
		expect(api.pushWidgetMeta.mock.calls[0][2]).toMatchObject({
			name: source.name,
			tags: ["shared"],
		});
		expect(source).toEqual(before);
	});

	test("imports shared nested dependencies once, handles cycles, and preserves package widget identities", async () => {
		const a = widget("a", [
			component("root", {
				type: "column",
				children: {
					explicitList: ["first", "second", "local", "package", "label"],
				},
			}),
			component("first", {
				type: "widgetInstance",
				instanceId: "first-instance",
				widgetId: "b",
				runtimeChildUpdates: {
					label: [{ text: { literalString: "Updated" }, componentId: "label" }],
				},
				exposedPropValues: {
					target: { literalString: "label" },
					title: "label",
					bytes: [34, 108, 97, 98, 101, 108, 34],
				},
			}),
			component("second", {
				type: "widgetInstance",
				instanceId: "second-instance",
				widgetId: "b",
				appId: "source",
			}),
			component("local", {
				type: "widgetInstance",
				instanceId: "local-instance",
				widgetId: "existing",
				appId: "target",
				exposedPropValues: { context: { componentId: "label" } },
				runtimeChildUpdates: { label: [{ componentId: "label" }] },
				inlineWidgetDef: {
					name: "Inline",
					rootComponentId: "label",
					components: [
						component("label", {
							type: "text",
							text: { literalString: "Inline" },
						}),
					],
				},
			}),
			component("package", {
				type: "microWidgetInstance",
				instanceId: "package-instance",
				widgetId: "b",
				packageId: "package-id",
			}),
			component("label", { type: "text", text: { literalString: "Parent" } }),
		]);
		const b = widget("b", [
			component("root", {
				type: "column",
				children: { explicitList: ["back", "label"] },
			}),
			component("back", {
				type: "widgetInstance",
				instanceId: "back-instance",
				widgetId: "a",
				appId: "source",
			}),
			component("label", { type: "text", text: { literalString: "Original" } }),
		]);
		b.exposedProps = [
			{
				id: "target",
				label: "Target",
				targetComponentId: "label",
				propertyPath: "componentId",
				propType: "String",
			},
		];
		const api = backend({ a, b });
		const imported = await importWidget(api.state, "source", "a", "target");
		expect(api.getWidgetAuthoritative).toHaveBeenCalledTimes(2);
		expect(api.saved.size).toBe(2);
		const copiedB = [...api.saved.values()].find(
			(entry) => entry.name === b.name,
		);
		expect(copiedB).toBeDefined();
		expect(copiedB?.id).not.toBe("b");
		for (const index of [1, 2]) {
			expect(imported.components[index].component).toMatchObject({
				widgetId: copiedB?.id,
				appId: "target",
			});
			expect(
				(
					imported.components[index].component as unknown as Record<
						string,
						unknown
					>
				).instanceId,
			).not.toBe(
				(a.components[index].component as unknown as Record<string, unknown>)
					.instanceId,
			);
		}
		expect(imported.components[1].component).toMatchObject({
			runtimeChildUpdates: {
				[copiedB?.components[2].id ?? ""]: [
					{
						text: { literalString: "Updated" },
						componentId: copiedB?.components[2].id,
					},
				],
			},
			exposedPropValues: {
				target: { literalString: copiedB?.components[2].id },
				title: "label",
				bytes: [34, 108, 97, 98, 101, 108, 34],
			},
		});
		expect(copiedB?.components[1].component).toMatchObject({
			widgetId: imported.id,
			appId: "target",
		});
		expect(imported.components[3].component).toMatchObject({
			widgetId: "existing",
			appId: "target",
			exposedPropValues: { context: { componentId: "label" } },
			runtimeChildUpdates: { label: [{ componentId: "label" }] },
			inlineWidgetDef: {
				name: "Inline",
				rootComponentId: "label",
				components: [
					component("label", {
						type: "text",
						text: { literalString: "Inline" },
					}),
				],
			},
		});
		expect(imported.components[4].component).toMatchObject({
			widgetId: "b",
			packageId: "package-id",
		});
		expect(
			(imported.components[4].component as unknown as Record<string, unknown>)
				.instanceId,
		).not.toBe("package-instance");
	});

	test("preserves source metadata and uses its display name for the new definition", async () => {
		const api = backend({ source: widget("source") });
		api.getWidgetMeta.mockImplementation(async () => ({
			name: "Visible name",
			description: "Description",
			long_description: "Details",
			tags: ["tag"],
			icon: "icon-url",
			preview_media: ["preview-url"],
			created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
		}));
		const imported = await importWidget(
			api.state,
			"source-app",
			"source",
			"target-app",
		);
		expect(imported.name).toBe("Visible name");
		expect(api.pushWidgetMeta.mock.calls[0][2]).toMatchObject({
			name: "Visible name",
			description: "Description",
			long_description: "Details",
			tags: ["tag"],
			icon: "icon-url",
			preview_media: ["preview-url"],
		});
		expect(
			api.pushWidgetMeta.mock.calls[0][2].created_at.secs_since_epoch,
		).toBeGreaterThan(1);
	});

	test("reads every dependency before writing anything", async () => {
		const source = widget("source", [
			component("root", {
				type: "widgetInstance",
				widgetId: "missing",
				instanceId: "nested",
			}),
		]);
		const api = backend({ source });
		await expect(
			importWidget(api.state, "source-app", "source", "target-app"),
		).rejects.toThrow("Missing widget missing");
		expect(api.updateWidget).not.toHaveBeenCalled();
		expect(api.deleteWidget).not.toHaveBeenCalled();
	});

	test("removes all attempted copies if a write fails after persisting", async () => {
		const a = widget("a", [
			component("root", {
				type: "widgetInstance",
				widgetId: "b",
				instanceId: "nested",
			}),
		]);
		const api = backend({ a, b: widget("b") });
		const failure = new Error("Write failed");
		api.updateWidget.mockImplementation(async (_app, value) => {
			api.saved.set(value.id, value);
			if (value.name === a.name) throw failure;
		});
		await expect(importWidget(api.state, "source", "a", "target")).rejects.toBe(
			failure,
		);
		expect(api.saved.size).toBe(0);
		expect(api.deleteWidget.mock.calls.map(([, id]) => id)).toEqual(
			api.updateWidget.mock.calls.map(([, value]) => value.id).reverse(),
		);
	});

	test("removes the new definition if metadata persistence fails", async () => {
		const api = backend({ source: widget("source") });
		api.pushWidgetMeta.mockImplementation(async () => {
			throw new Error("Metadata failed");
		});
		await expect(
			importWidget(api.state, "source", "source", "target"),
		).rejects.toThrow("Metadata failed");
		expect(api.saved.size).toBe(0);
		expect(api.deleteWidget).toHaveBeenCalledTimes(1);
	});
});
