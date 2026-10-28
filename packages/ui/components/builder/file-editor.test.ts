import { describe, expect, test } from "bun:test";
import { COMPONENT_PROPS } from "../a2ui/component-prop-manifest";
import type { FileEditorComponent } from "../a2ui/types";
import { CRAFT_EDITOR_IDS } from "../storage/craft-viewers";
import {
	createDefaultComponent,
	normalizeComponent,
} from "./componentDefaults";
import { getComponentSchema } from "./componentSchema";

describe("File Editor builder setup", () => {
	test("offers each native editor and leaves the initial source optional", () => {
		const schema = getComponentSchema("fileEditor");
		expect(schema?.editor).toMatchObject({
			required: true,
			enum: [...CRAFT_EDITOR_IDS],
		});
		expect(schema?.src.required).not.toBe(true);
		for (const key of COMPONENT_PROPS.fileEditor)
			expect(schema?.[key]).toBeDefined();
	});

	test("starts with an empty document and waits for an authored Save workflow", () => {
		const component = createDefaultComponent(
			"fileEditor",
		) as FileEditorComponent;
		expect(component.editor).toEqual({ literalString: "wordcraft" });
		expect(component.src).toEqual({ literalString: "" });
		expect(component.initialEditing).toEqual({ literalBool: true });
		expect(component.eventHandlers).toBeUndefined();
	});

	test("preserves bound source and save metadata when normalizing", () => {
		const component: Omit<FileEditorComponent, "id"> = {
			type: "fileEditor",
			editor: { literalString: "solidcraft" },
			src: { path: "$.file.url" },
			value: { path: "$.result" },
			eventHandlers: {
				save: [{ name: "workflow_event", context: { nodeId: "save" } }],
			},
		};
		expect(
			normalizeComponent({ id: "file-editor", ...component }),
		).toMatchObject(component);
	});

	test("uses the optional URL before filling the source default", () => {
		for (const url of [
			{ literalString: "/files/model.step" },
			{ path: "$.file.url" },
		]) {
			const component: Omit<FileEditorComponent, "id"> = {
				type: "fileEditor",
				editor: { literalString: "solidcraft" },
				url,
			};
			expect(
				normalizeComponent({ id: "file-editor", ...component }),
			).toMatchObject({
				...component,
				src: url,
			});
			expect(
				normalizeComponent({
					id: "file-editor",
					...component,
					src: { literalString: "" },
				}),
			).toMatchObject({ src: { literalString: "" }, url });
		}
	});
});
