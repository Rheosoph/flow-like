import type { IMetadata, IWidget } from "@flow-like/flow-like-ui";
import { describe, expect, it } from "vitest";
import { withWidgetName } from "../widget-metadata";

const WIDGET = {
	id: "f804a7ocd2wh7f1uded3gkjp",
	name: "control-center-tile",
	description: "A tile",
	rootComponentId: "root",
	components: [],
	dataModel: [],
	customizationOptions: [],
	tags: ["ui"],
	createdAt: "2026-07-29T10:41:00.000Z",
	updatedAt: "2026-07-29T10:41:00.000Z",
} as unknown as IWidget;

const METADATA: IMetadata = {
	name: "Control Center Tile",
	description: "Curated description",
	tags: [],
	long_description: "",
	preview_media: [],
	created_at: { nanos_since_epoch: 0, secs_since_epoch: 1 },
	updated_at: { nanos_since_epoch: 0, secs_since_epoch: 1 },
};

describe("withWidgetName", () => {
	it("keeps metadata that already carries a name", () => {
		expect(withWidgetName(METADATA, WIDGET)).toBe(METADATA);
	});

	it("falls back to the widget name when metadata is missing", () => {
		const result = withWidgetName(undefined, WIDGET);
		expect(result?.name).toBe("control-center-tile");
		expect(result?.description).toBe("A tile");
		expect(result?.tags).toEqual(["ui"]);
		expect(result?.created_at.secs_since_epoch).toBe(
			Math.floor(Date.parse(WIDGET.createdAt) / 1000),
		);
	});

	it("falls back when metadata carries a blank name", () => {
		const result = withWidgetName({ ...METADATA, name: "   " }, WIDGET);
		expect(result?.name).toBe("control-center-tile");
		expect(result?.description).toBe("Curated description");
	});

	it("returns metadata untouched when no widget is available", () => {
		expect(withWidgetName(undefined, undefined)).toBeUndefined();
	});

	it("does not invent a name for a nameless widget", () => {
		expect(
			withWidgetName(undefined, { ...WIDGET, name: "  " }),
		).toBeUndefined();
	});
});
