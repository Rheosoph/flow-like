import { describe, expect, test } from "bun:test";
import { IIndexType } from "../../state/backend-state/db-state";
import { arrowToLanceSchema } from "./lance-viewer";
import {
	canArrowFieldBeKey,
	getIndexTypeOptions,
	isKeyColumnType,
	isKeyFieldMetadata,
} from "./table-schema";

function optionsFor(columnType?: string): IIndexType[] {
	return getIndexTypeOptions(columnType).map((option) => option.type);
}

describe("index options for a column", () => {
	test.each(["date", "date32", "timestamp", "timestamp_ms", "int64", "number"])(
		"offers range and equality indexes on %s columns",
		(columnType) => {
			const options = optionsFor(columnType);
			expect(options).toContain(IIndexType.Auto);
			expect(options).toContain(IIndexType.BTree);
			expect(options).toContain(IIndexType.ZoneMap);
			expect(options).toContain(IIndexType.BloomFilter);
			expect(options).not.toContain(IIndexType.FullText);
			expect(options).not.toContain(IIndexType.IvfPq);
		},
	);

	test("offers text substring indexes on string columns", () => {
		const options = optionsFor("string");
		expect(options).toContain(IIndexType.FullText);
		expect(options).toContain(IIndexType.Fm);
		expect(options).toContain(IIndexType.NGram);
		expect(options).toContain(IIndexType.ZoneMap);
		expect(options).not.toContain(IIndexType.RTree);
	});

	test("lets users select every supported vector algorithm", () => {
		expect(optionsFor("vector")).toEqual([
			IIndexType.Auto,
			IIndexType.Vector,
			IIndexType.IvfFlat,
			IIndexType.IvfPq,
			IIndexType.IvfSq,
			IIndexType.IvfRq,
			IIndexType.IvfHnswFlat,
			IIndexType.IvfHnswPq,
			IIndexType.IvfHnswSq,
		]);
	});

	test("offers spatial indexes only for verified geometry columns", () => {
		expect(optionsFor("geometry")).toEqual([IIndexType.Auto, IIndexType.RTree]);
		expect(optionsFor("object")).not.toContain(IIndexType.RTree);
		expect(optionsFor("array")).not.toContain(IIndexType.RTree);
		expect(optionsFor("unsupported-geometry")).not.toContain(IIndexType.RTree);
		expect(optionsFor("array")).toContain(IIndexType.LabelList);
	});

	test("offers FM substring indexes on binary columns", () => {
		expect(optionsFor("binary")).toContain(IIndexType.BTree);
		expect(optionsFor("binary")).toContain(IIndexType.Bitmap);
		expect(optionsFor("binary")).toContain(IIndexType.Fm);
		expect(optionsFor("binary")).not.toContain(IIndexType.FullText);
	});

	test("keeps boolean options within the supported index types", () => {
		expect(optionsFor("boolean")).toContain(IIndexType.Bitmap);
		expect(optionsFor("boolean")).not.toContain(IIndexType.BloomFilter);
	});

	test("keeps general choices available when a column kind is unknown", () => {
		expect(optionsFor("unknown")).toEqual(optionsFor(undefined));
		expect(optionsFor("unknown")).not.toContain(IIndexType.RTree);
		expect(optionsFor("unknown")).toContain(IIndexType.IvfHnswSq);
	});

	test("does not offer vector training on a new empty table", () => {
		expect(getIndexTypeOptions(undefined, "scalar")).not.toContainEqual(
			expect.objectContaining({ type: IIndexType.IvfPq }),
		);
	});
});

const POSITION = "lance-schema:unenforced-primary-key:position";
const LEGACY = "lance-schema:unenforced-primary-key";

describe("table key", () => {
	test("reads the position marker and the legacy flag as Lance does", () => {
		expect(isKeyFieldMetadata({ [POSITION]: "1" })).toBe(true);
		expect(isKeyFieldMetadata({ [POSITION]: "0" })).toBe(true);
		expect(isKeyFieldMetadata({ [LEGACY]: "TRUE" })).toBe(true);
		expect(isKeyFieldMetadata({ [LEGACY]: "yes" })).toBe(true);
		expect(isKeyFieldMetadata({ [POSITION]: "x", [LEGACY]: "1" })).toBe(true);
		expect(isKeyFieldMetadata({ [POSITION]: "-1" })).toBe(false);
		expect(isKeyFieldMetadata({ [POSITION]: "1.5" })).toBe(false);
		expect(isKeyFieldMetadata({ [POSITION]: "4294967296" })).toBe(false);
		expect(isKeyFieldMetadata({ [LEGACY]: "false" })).toBe(false);
		expect(isKeyFieldMetadata({})).toBe(false);
		expect(isKeyFieldMetadata(undefined)).toBe(false);
	});

	test("lets only required columns of the types Lance's conflict filter hashes become the key", () => {
		for (const data_type of [
			"Int32",
			"Int64",
			"UInt32",
			"UInt64",
			"Utf8",
			"LargeUtf8",
			"Binary",
			"LargeBinary",
		])
			expect(canArrowFieldBeKey({ data_type, nullable: false })).toBe(true);
		expect(canArrowFieldBeKey({ data_type: "Utf8", nullable: true })).toBe(
			false,
		);
		expect(canArrowFieldBeKey({ data_type: "Utf8" })).toBe(false);
		for (const data_type of [
			"Int16",
			"Float64",
			"Utf8View",
			"Bool",
			{ FixedSizeBinary: 16 },
		])
			expect(canArrowFieldBeKey({ data_type, nullable: false })).toBe(false);
	});

	test("offers the key toggle only for designer types that map to key types", () => {
		expect(
			["string", "int32", "int64", "uint32", "uint64", "binary"].every(
				isKeyColumnType,
			),
		).toBe(true);
		for (const type of ["geometry", "int16", "float64", "vector"])
			expect(isKeyColumnType(type)).toBe(false);
	});

	test("the explorer schema names the key and the columns that could become it", () => {
		const schema = arrowToLanceSchema({
			fields: [
				{ name: "id", data_type: "Utf8", nullable: false },
				{
					name: "sku",
					data_type: "Int64",
					nullable: false,
					metadata: { [POSITION]: "1" },
				},
				{ name: "note", data_type: "Utf8", nullable: true },
				{
					name: "shape",
					data_type: "Binary",
					nullable: false,
					metadata: { "ARROW:extension:name": "geoarrow.wkb" },
				},
			],
		});
		expect(schema.primaryKey).toBe("sku");
		expect(
			schema.fields.filter((f) => f.keyEligible).map((f) => f.name),
		).toEqual(["id", "sku"]);
		expect(
			arrowToLanceSchema({ fields: [{ name: "id", data_type: "Utf8" }] })
				.primaryKey,
		).toBeUndefined();
	});
});
