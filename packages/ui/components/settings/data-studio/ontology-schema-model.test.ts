import { describe, expect, test } from "bun:test";
import type {
	EdgeLabelMapping,
	NodeLabelMapping,
} from "../../../state/backend-state/graph-state";
import {
	SCHEMA_HEADER_HEIGHT,
	SCHEMA_ROW_HEIGHT,
	boundaryPoint,
	buildSchemaModel,
	curveGeometry,
	loopGeometry,
	routedCurveGeometry,
} from "./ontology-schema-model";

function node(
	label: string,
	table: string,
	columns: string[],
	idColumn = "id",
): NodeLabelMapping {
	return {
		id: label.toLowerCase(),
		label,
		table,
		id_column: idColumn,
		property_columns: columns.map((name) => ({ name, data_type: "Utf8" })),
		style: { color: "#2563eb", icon: "database", size: { mode: "fixed" } },
	};
}

function edge(
	label: string,
	src: string,
	dst: string,
	table: string,
	srcColumn: string,
	dstColumn: string,
	extra: Partial<EdgeLabelMapping> = {},
): EdgeLabelMapping {
	return {
		label,
		table,
		src_column: srcColumn,
		dst_column: dstColumn,
		src_label: src,
		dst_label: dst,
		property_columns: [],
		style: { color: "#2563eb", icon: "arrow-right", size: { mode: "fixed" } },
		...extra,
	};
}

const archive = {
	nodes: [
		node("Article", "articles", ["id", "document_path", "hash", "content"]),
		node("Entity", "canonical_entities", ["entity"], "entity"),
		node("Image", "images", ["id", "article_id"]),
		node("Face", "faces", ["id", "age", "gender", "article_id", "image_id"]),
	],
	edges: [
		edge("MENTIONS", "Article", "Entity", "entities", "article_id", "entity"),
		edge("HAS_IMAGE", "Article", "Image", "images", "article_id", "id"),
		edge("HAS_FACE", "Article", "Face", "faces", "article_id", "id"),
		edge("SHOWS", "Image", "Face", "faces", "image_id", "id"),
	],
};

describe("buildSchemaModel", () => {
	test("lists identity first, then join columns, then plain properties", () => {
		const model = buildSchemaModel(archive.nodes, archive.edges);
		const face = model.objects.find((object) => object.label === "Face");
		expect(face?.rows.map((row) => [row.name, row.role])).toEqual([
			["id", "id"],
			["article_id", "link"],
			["image_id", "link"],
			["age", "property"],
			["gender", "property"],
		]);
		expect(face?.hiddenRows).toBe(0);
	});

	test("caps visible rows and counts the rest", () => {
		const wide = node("Wide", "wide", [
			"id",
			"a",
			"b",
			"c",
			"d",
			"e",
			"f",
			"g",
		]);
		const [object] = buildSchemaModel([wide], []).objects;
		expect(object.rows).toHaveLength(6);
		expect(object.hiddenRows).toBe(2);
		expect(object.height).toBe(
			2 + SCHEMA_HEADER_HEIGHT + 9 + 7 * SCHEMA_ROW_HEIGHT,
		);
	});

	test("keeps a join column that is not a mapped property", () => {
		const image = node("Image", "images", ["id"]);
		const article = node("Article", "articles", ["id"]);
		const model = buildSchemaModel(
			[article, image],
			[edge("HAS_IMAGE", "Article", "Image", "images", "article_id", "id")],
		);
		const rows = model.objects.find((object) => object.label === "Image")?.rows;
		expect(rows?.map((row) => row.name)).toEqual(["id", "article_id"]);
		expect(rows?.[1].dataType).toBeUndefined();
	});

	test("resolves endpoints by label case-insensitively", () => {
		const model = buildSchemaModel(archive.nodes, [
			edge("MENTIONS", "article", "ENTITY", "entities", "article_id", "entity"),
		]);
		expect(model.objects.every((object) => object.kind === "object")).toBe(
			true,
		);
		expect(model.relationships[0].source).toBe("object:article");
		expect(model.relationships[0].target).toBe("object:entity");
	});

	test("draws children in another ontology as a named external ghost", () => {
		const model = buildSchemaModel(
			archive.nodes,
			[
				edge("CONTAINS", "Article", "Page", "pages", "article_id", "id", {
					containment: true,
					dst_ontology: "overlay-2",
				}),
			],
			new Map([["local:overlay-2", "Scans"]]),
		);
		const ghost = model.objects.find((object) => object.kind === "external");
		expect(ghost?.label).toBe("Page");
		expect(ghost?.subtitle).toBe("Scans");
		expect(model.relationships[0].target).toBe(ghost?.id ?? "");
		expect(model.relationships[0].external).toBe(true);
		expect(model.relationships[0].containment).toBe(true);
	});

	test("draws an unknown label as a missing ghost instead of dropping the edge", () => {
		const model = buildSchemaModel(archive.nodes, [
			edge("OWNS", "Article", "Owner", "articles", "id", "owner_id"),
		]);
		const ghost = model.objects.find((object) => object.kind === "missing");
		expect(ghost?.label).toBe("Owner");
		expect(model.relationships[0].target).toBe(ghost?.id ?? "");
	});

	test("fans parallel edges out to opposite sides, whatever their direction", () => {
		const model = buildSchemaModel(archive.nodes, [
			edge("CITES", "Article", "Image", "images", "article_id", "id"),
			edge("ILLUSTRATES", "Image", "Article", "images", "id", "article_id"),
		]);
		const a = { x: 0, y: 0, width: 100, height: 50 };
		const b = { x: 300, y: 0, width: 100, height: 50 };
		const [first, second] = model.relationships;
		const firstLabel = curveGeometry(a, b, first.offset).labelY;
		const secondLabel = curveGeometry(b, a, second.offset).labelY;
		expect(Math.sign(firstLabel - 25)).toBe(-Math.sign(secondLabel - 25));
		expect(Math.abs(firstLabel - 25)).toBeCloseTo(Math.abs(secondLabel - 25));
	});

	test("numbers self-references so their loops nest", () => {
		const model = buildSchemaModel(archive.nodes, [
			edge("PARENT", "Article", "Article", "articles", "id", "parent_id"),
			edge("REPLY_TO", "Article", "Article", "articles", "id", "reply_id"),
		]);
		expect(model.relationships.map((item) => item.loop)).toEqual([0, 1]);
		expect(model.relationships.map((item) => item.offset)).toEqual([0, 0]);
	});
});

describe("edge geometry", () => {
	const rect = { x: 0, y: 0, width: 200, height: 100 };

	test("boundaryPoint leaves through the side the ray points at", () => {
		expect(boundaryPoint(rect, { x: 500, y: 50 })).toEqual({ x: 200, y: 50 });
		expect(boundaryPoint(rect, { x: 100, y: -300 })).toEqual({ x: 100, y: 0 });
		expect(boundaryPoint(rect, { x: 100, y: 50 })).toEqual({ x: 100, y: 50 });
	});

	test("an unbent curve runs straight between the facing borders", () => {
		const target = { x: 400, y: 0, width: 200, height: 100 };
		const geometry = curveGeometry(rect, target, 0);
		expect(geometry.path).toBe("M 200 50 Q 300 50 400 50");
		expect(geometry.labelX).toBe(300);
		expect(geometry.labelY).toBe(50);
	});

	test("a routed curve bends around a card sitting between its ends", () => {
		const target = { x: 800, y: 0, width: 200, height: 100 };
		const between = { x: 400, y: 30, width: 200, height: 140 };
		const direct = curveGeometry(rect, target, 0);
		const routed = routedCurveGeometry(rect, target, 0, [between]);
		expect(routed).not.toEqual(direct);
		expect(routed.labelY).toBeLessThan(between.y);
		expect(routedCurveGeometry(rect, target, 0, [])).toEqual(direct);
	});

	test("a laned curve passes through its lane unless a card now sits there", () => {
		const target = { x: 800, y: 0, width: 200, height: 100 };
		const lane = { x: 500, y: 220 };
		const laned = routedCurveGeometry(rect, target, 0, [], lane);
		expect(laned.labelX).toBeCloseTo(lane.x);
		expect(laned.labelY).toBeCloseTo(lane.y);
		const blocker = { x: 450, y: 200, width: 100, height: 60 };
		const rerouted = routedCurveGeometry(rect, target, 0, [blocker], lane);
		expect(rerouted.labelY).not.toBeCloseTo(lane.y);
	});

	test("a loop starts on the top border and ends on the right border", () => {
		const geometry = loopGeometry(rect, 0);
		expect(geometry.path.startsWith("M 160 0 ")).toBe(true);
		expect(geometry.path.endsWith(" 200 24")).toBe(true);
		expect(geometry.labelY).toBeLessThan(rect.y + 24);
		expect(geometry.labelX).toBeGreaterThan(rect.x + rect.width - 40);
	});
});
