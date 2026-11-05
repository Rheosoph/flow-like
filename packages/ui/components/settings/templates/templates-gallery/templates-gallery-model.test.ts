import { describe, expect, test } from "bun:test";
import type { IMetadata } from "../../../../lib/schema/bit/bit";
import type {
	IBoard,
	ILayer,
	INode,
	IPin,
} from "../../../../lib/schema/flow/board";
import {
	type ITemplateEntry,
	buildThumbnail,
	defaultFlowName,
	digestTemplate,
	filterBySource,
	sortTemplates,
	sourceGroups,
	toTemplateItem,
	versionLabel,
} from "./templates-gallery-model";

const pin = (
	id: string,
	pinType: "Input" | "Output",
	connectedTo: string[] = [],
	dataType = "Execution",
): IPin =>
	({
		id,
		name: id,
		friendly_name: id,
		pin_type: pinType,
		data_type: dataType,
		connected_to: connectedTo,
		depends_on: [],
	}) as unknown as IPin;

const node = (
	id: string,
	options: Omit<Partial<INode>, "pins"> & { pins?: IPin[] } = {},
): INode => {
	const { pins = [], ...rest } = options;
	return {
		id,
		name: id,
		friendly_name: id,
		category: "Utils",
		coordinates: [0, 0, 0],
		description: "",
		pins: Object.fromEntries(pins.map((item) => [item.id, item])),
		...rest,
	} as unknown as INode;
};

const board = (partial: Partial<IBoard>): IBoard =>
	({
		id: "template",
		name: "Card Generator",
		description: "",
		nodes: {},
		layers: {},
		variables: {},
		comments: {},
		version: [0, 0, 0],
		...partial,
	}) as unknown as IBoard;

const meta = (name: string, updatedSecs: number): IMetadata =>
	({
		name,
		description: "",
		tags: [],
		preview_media: [],
		created_at: { secs_since_epoch: updatedSecs, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: updatedSecs, nanos_since_epoch: 0 },
	}) as IMetadata;

const item = (
	templateId: string,
	name: string,
	sourceName: string,
	updatedSecs: number,
	nodeCount = 0,
) =>
	toTemplateItem(
		["app", templateId, meta(name, updatedSecs)] as ITemplateEntry,
		board({
			name: sourceName,
			nodes: Object.fromEntries(
				Array.from({ length: nodeCount }, (_, index) => [
					`n${index}`,
					node(`n${index}`),
				]),
			),
		}),
		"ready",
	);

describe("digestTemplate", () => {
	test("reads the source flow name, counts and entry events from the board", () => {
		const digest = digestTemplate(
			board({
				name: "Course Sync",
				version: [0, 2, 1],
				nodes: {
					start: node("start", { friendly_name: "Chat Event", start: true }),
					a: node("a", { friendly_name: "Read PDF" }),
					b: node("b", { friendly_name: "Read PDF" }),
					c: node("c", { friendly_name: "Insert Rows" }),
				},
				layers: { l: { id: "l" } as unknown as ILayer },
				variables: { v: {} as never, w: {} as never },
			}),
		);
		expect(digest.sourceName).toBe("Course Sync");
		expect(digest.version).toEqual([0, 2, 1]);
		expect(digest.nodeCount).toBe(4);
		expect(digest.layerCount).toBe(1);
		expect(digest.variableCount).toBe(2);
		expect(digest.entryNames).toEqual(["Chat Event"]);
		expect(digest.nodeTypes[0]).toBe("Read PDF");
	});
});

describe("versionLabel", () => {
	test("hides the first version and labels later ones", () => {
		expect(versionLabel([0, 0, 0])).toBeUndefined();
		expect(versionLabel([0, 1, 0])).toBe("v0.1.0");
	});
});

describe("buildThumbnail", () => {
	test("returns nothing for an empty board", () => {
		expect(buildThumbnail(board({}))).toBeUndefined();
	});

	test("draws top-level nodes and wires between them", () => {
		const thumbnail = buildThumbnail(
			board({
				nodes: {
					a: node("a", {
						start: true,
						coordinates: [0, 0, 0],
						pins: [pin("a-out", "Output", ["b-in"])],
					}),
					b: node("b", {
						coordinates: [400, 120, 0],
						pins: [pin("b-in", "Input")],
					}),
				},
			}),
		);
		expect(thumbnail?.boxes.map((box) => [box.id, box.kind])).toEqual([
			["a", "event"],
			["b", "logic"],
		]);
		expect(thumbnail?.wires).toHaveLength(1);
		expect(thumbnail?.wires[0]?.execution).toBe(true);
		expect(thumbnail?.wires[0]?.d.startsWith("M220 ")).toBe(true);
	});

	test("folds nodes inside nested layers into their top-level layer", () => {
		const thumbnail = buildThumbnail(
			board({
				nodes: {
					a: node("a", { pins: [pin("a-out", "Output", ["inner-in"])] }),
					inner: node("inner", {
						layer: "child",
						pins: [pin("inner-in", "Input")],
					}),
				},
				layers: {
					top: {
						id: "top",
						coordinates: [500, 0, 0],
						pins: {},
					} as unknown as ILayer,
					child: {
						id: "child",
						parent_id: "top",
						coordinates: [0, 0, 0],
						pins: {},
					} as unknown as ILayer,
				},
			}),
		);
		expect(thumbnail?.boxes.map((box) => box.id).sort()).toEqual(["a", "top"]);
		expect(thumbnail?.wires.map((wire) => wire.id)).toEqual(["a>top"]);
	});

	test("survives missing coordinates and keeps a minimum view", () => {
		const thumbnail = buildThumbnail(
			board({ nodes: { a: node("a", { coordinates: null }) } }),
		);
		const [, , width, height] = (thumbnail?.viewBox ?? "")
			.split(" ")
			.map(Number);
		expect(width).toBeGreaterThanOrEqual(900);
		expect(height).toBeGreaterThanOrEqual(420);
	});
});

describe("source groups, filters and sorting", () => {
	const items = [
		item("t1", "Backup 04.11", "Card Generator", 300, 31),
		item("t2", "Backup 04.11", "Course Sync", 200, 27),
		item("t3", "Simple Cards", "Card Generator", 100, 14),
	];

	test("groups by source flow, largest group first", () => {
		expect(sourceGroups(items)).toEqual([
			{ name: "Card Generator", count: 2 },
			{ name: "Course Sync", count: 1 },
		]);
	});

	test("filters to one source flow", () => {
		expect(
			filterBySource(items, "Course Sync").map((entry) => entry.templateId),
		).toEqual(["t2"]);
		expect(filterBySource(items, null)).toHaveLength(3);
	});

	test("sorts by update time, name and node count", () => {
		const ids = (sort: Parameters<typeof sortTemplates>[1]) =>
			sortTemplates(items, sort).map((entry) => entry.templateId);
		expect(ids("updated")).toEqual(["t1", "t2", "t3"]);
		expect(ids("name")).toEqual(["t1", "t2", "t3"]);
		expect(ids("nodes")).toEqual(["t1", "t2", "t3"]);
		expect(
			sortTemplates([items[2], items[0]], "nodes").map((e) => e.templateId),
		).toEqual(["t1", "t3"]);
	});

	test("names a new flow after its source and the template", () => {
		expect(defaultFlowName(items[0])).toBe("Card Generator (Backup 04.11)");
		expect(defaultFlowName(item("t4", "Ingest", "Ingest", 1))).toBe("Ingest");
	});
});
