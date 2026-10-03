import { describe, expect, test } from "bun:test";
import {
	type EventFormFieldSpec,
	type FieldValue,
	fieldPayload,
	fieldShape,
	formPayload,
	formSupport,
	isFieldDate,
	seedValue,
	seedValues,
} from "./event-form";

function field(
	name: string,
	data_type: string,
	rest: Partial<EventFormFieldSpec> = {},
): EventFormFieldSpec {
	return {
		name,
		data_type,
		value_type: "Normal",
		optional: false,
		sensitive: false,
		default: null,
		options: null,
		...rest,
	};
}

const send = (spec: EventFormFieldSpec, value: FieldValue) =>
	fieldPayload(spec, value);

describe("event form fields", () => {
	test("each field type gets its control; a list, set or map is a JSON box", () => {
		const cases: [EventFormFieldSpec, string, string][] = [
			[field("title", "String"), "text", "string"],
			[field("area", "Geometry"), "text", "string"],
			[field("count", "Integer"), "integer", "integer"],
			[field("ratio", "Float"), "number", "number"],
			[field("urgent", "Boolean"), "switch", "boolean"],
			[field("due", "Date"), "date", "date"],
			[field("order", "Struct"), "json", "json"],
			[field("anything", "Generic"), "json", "json"],
			[field("receipt", "PathBuf"), "file", "file"],
			[field("blob", "Byte"), "file", "file"],
			[field("tags", "String", { value_type: "Array" }), "json", "json"],
			[field("ids", "Integer", { value_type: "HashSet" }), "json", "json"],
			[field("labels", "String", { value_type: "HashMap" }), "json", "json"],
			[field("files", "PathBuf", { value_type: "Array" }), "file", "file"],
			[
				field("size", "String", { options: ["S", "M", "L"] }),
				"select",
				"string",
			],
			[field("level", "Integer", { options: ["1", "2"] }), "select", "integer"],
			[field("pin", "String", { sensitive: true }), "password", "string"],
			[field("code", "Integer", { sensitive: true }), "password", "integer"],
			[field("key", "Struct", { sensitive: true }), "password", "json"],
			[field("flag", "Boolean", { sensitive: true }), "switch", "boolean"],
		];
		for (const [spec, control, base] of cases)
			expect([
				spec.name,
				fieldShape(spec)?.control,
				fieldShape(spec)?.base,
			]).toEqual([spec.name, control, base]);
		expect(fieldShape(field("title", "String"))?.required).toBe(true);
		expect(
			fieldShape(field("title", "String", { optional: true }))?.required,
		).toBe(false);
	});

	test("a type this client does not know has no control, never a text box", () => {
		expect(fieldShape(field("shape", "Polygon"))).toBeNull();
		expect(fieldShape(field("run", "Execution"))).toBeNull();
		expect(
			fieldShape(field("tags", "String", { value_type: "Tree" })),
		).toBeNull();
		expect(seedValue(field("shape", "Polygon", { default: "x" }))).toBe("");
		expect(send(field("shape", "Polygon"), "x")).toEqual({
			ok: false,
			problem: "unknown",
		});
	});

	test("defaults fill the controls; a sensitive default never does", () => {
		expect(seedValue(field("title", "String", { default: "Hello" }))).toBe(
			"Hello",
		);
		expect(seedValue(field("count", "Integer", { default: 3 }))).toBe("3");
		expect(seedValue(field("ratio", "Float", { default: 0.5 }))).toBe("0.5");
		expect(seedValue(field("urgent", "Boolean", { default: true }))).toBe(true);
		expect(seedValue(field("urgent", "Boolean"))).toBe(false);
		expect(
			seedValue(field("due", "Date", { default: "2026-10-03T09:00:00Z" })),
		).toBe("2026-10-03");
		expect(seedValue(field("order", "Struct", { default: { id: 1 } }))).toBe(
			'{\n  "id": 1\n}',
		);
		expect(
			seedValue(
				field("tags", "String", { value_type: "Array", default: ["a"] }),
			),
		).toBe('[\n  "a"\n]');
		expect(
			seedValue(field("size", "String", { options: ["S"], default: "S" })),
		).toBe("S");
		expect(
			seedValue(field("pin", "String", { sensitive: true, default: "1234" })),
		).toBe("");
		expect(seedValue(field("title", "String"))).toBe("");
		expect(
			seedValues([
				field("title", "String", { default: "Hi" }),
				field("urgent", "Boolean"),
			]),
		).toEqual({ title: "Hi", urgent: false });
	});

	test("text, numbers, switches and dates become the values a device accepts", () => {
		expect(send(field("title", "String"), "  Hello ")).toEqual({
			ok: true,
			value: "  Hello ",
		});
		expect(send(field("count", "Integer"), " 42 ")).toEqual({
			ok: true,
			value: 42,
		});
		expect(send(field("count", "Integer"), "-7")).toEqual({
			ok: true,
			value: -7,
		});
		for (const bad of ["4.2", "1e3", "abc", "9007199254740993"])
			expect(send(field("count", "Integer"), bad)).toEqual({
				ok: false,
				problem: "integer",
			});
		expect(send(field("ratio", "Float"), "2.5")).toEqual({
			ok: true,
			value: 2.5,
		});
		expect(send(field("ratio", "Float"), "1e3")).toEqual({
			ok: true,
			value: 1000,
		});
		for (const bad of ["Infinity", "NaN", "0x10", "1,5", "1e999"])
			expect(send(field("ratio", "Float"), bad)).toEqual({
				ok: false,
				problem: "number",
			});
		expect(send(field("urgent", "Boolean"), true)).toEqual({
			ok: true,
			value: true,
		});
		expect(send(field("urgent", "Boolean"), false)).toEqual({
			ok: true,
			value: false,
		});
		expect(send(field("due", "Date"), "2026-10-03")).toEqual({
			ok: true,
			value: "2026-10-03",
		});
		expect(send(field("due", "Date"), "2026-10-03T09:00:00+02:00")).toEqual({
			ok: true,
			value: "2026-10-03T09:00:00+02:00",
		});
		for (const bad of ["2026-02-30", "03.10.2026", "2026-10-03T25:00:00Z"])
			expect(send(field("due", "Date"), bad)).toEqual({
				ok: false,
				problem: "date",
			});
		expect(send(field("size", "String", { options: ["S", "M"] }), "M")).toEqual(
			{
				ok: true,
				value: "M",
			},
		);
		expect(
			send(field("size", "String", { options: ["S", "M"] }), "XL"),
		).toEqual({
			ok: false,
			problem: "option",
		});
		expect(
			send(field("level", "Integer", { options: ["1", "2"] }), "2"),
		).toEqual({
			ok: true,
			value: 2,
		});
		expect(send(field("pin", "String", { sensitive: true }), "1234")).toEqual({
			ok: true,
			value: "1234",
		});
	});

	test("JSON boxes: an object for a struct, an array for a list or set, an object for a map, members of their type", () => {
		expect(send(field("order", "Struct"), '{"id": 1}')).toEqual({
			ok: true,
			value: { id: 1 },
		});
		expect(send(field("order", "Struct"), "[1]")).toEqual({
			ok: false,
			problem: "object",
		});
		expect(send(field("order", "Struct"), "{id: 1}")).toEqual({
			ok: false,
			problem: "json",
		});
		expect(send(field("anything", "Generic"), '"text"')).toEqual({
			ok: true,
			value: "text",
		});
		const tags = field("tags", "String", { value_type: "Array" });
		expect(send(tags, '["a", "b"]')).toEqual({ ok: true, value: ["a", "b"] });
		expect(send(tags, '{"a": 1}')).toEqual({ ok: false, problem: "array" });
		expect(send(tags, '["a", 2]')).toEqual({ ok: false, problem: "items" });
		const ids = field("ids", "Integer", { value_type: "HashSet" });
		expect(send(ids, "[1, 2]")).toEqual({ ok: true, value: [1, 2] });
		expect(send(ids, "[1, 1]")).toEqual({ ok: false, problem: "unique" });
		expect(send(ids, "[1.5]")).toEqual({ ok: false, problem: "items" });
		const labels = field("labels", "String", { value_type: "HashMap" });
		expect(send(labels, '{"a": "x"}')).toEqual({
			ok: true,
			value: { a: "x" },
		});
		expect(send(labels, '["x"]')).toEqual({ ok: false, problem: "object" });
		expect(send(labels, '{"a": 1}')).toEqual({ ok: false, problem: "items" });
	});

	test("an empty field: required is a problem, optional is left out so the flow's default applies", () => {
		for (const spec of [
			field("title", "String"),
			field("count", "Integer"),
			field("due", "Date"),
			field("order", "Struct"),
			field("size", "String", { options: ["S"] }),
		]) {
			expect(send(spec, "")).toEqual({ ok: false, problem: "required" });
			expect(send({ ...spec, optional: true }, "")).toEqual({
				ok: true,
				omitted: true,
			});
		}
		expect(send(field("count", "Integer", { optional: true }), "   ")).toEqual({
			ok: true,
			omitted: true,
		});
		expect(send(field("title", "String"), " ")).toEqual({
			ok: true,
			value: " ",
		});
	});

	test("a file field can't be sent from here", () => {
		expect(send(field("receipt", "PathBuf"), "c:/x.pdf")).toEqual({
			ok: false,
			problem: "file",
		});
	});

	test("a form's payload: values by name, empty optional fields left out, every problem by name", () => {
		const fields = [
			field("title", "String"),
			field("quantity", "Integer", { default: 1 }),
			field("note", "String", { optional: true }),
			field("urgent", "Boolean"),
		];
		const values = seedValues(fields);
		expect(formPayload(fields, { ...values, title: "Hello" })).toEqual({
			ok: true,
			payload: { title: "Hello", quantity: 1, urgent: false },
		});
		expect(formPayload(fields, { ...values, quantity: "many" })).toEqual({
			ok: false,
			problems: { title: "required", quantity: "integer" },
		});
		expect(formPayload([], {})).toEqual({ ok: true, payload: {} });
	});

	test("whether a form can be sent from here: a file field, then an unknown type, then a cut list", () => {
		const base = { fields_truncated: false, file_fields: 0 };
		expect(
			formSupport({ ...base, fields: [field("title", "String")] }),
		).toEqual({
			ok: true,
		});
		expect(formSupport({ ...base, fields: [] })).toEqual({ ok: true });
		expect(
			formSupport({
				...base,
				file_fields: 1,
				fields: [field("title", "String"), field("receipt", "PathBuf")],
			}),
		).toEqual({ ok: false, reason: "file", field: "receipt" });
		expect(formSupport({ ...base, file_fields: 1, fields: [] })).toEqual({
			ok: false,
			reason: "file",
		});
		expect(
			formSupport({
				...base,
				fields: [field("title", "String"), field("shape", "Polygon")],
			}),
		).toEqual({ ok: false, reason: "unknown_field", field: "shape" });
		expect(
			formSupport({
				...base,
				fields_truncated: true,
				fields: [field("title", "String")],
			}),
		).toEqual({ ok: false, reason: "truncated" });
	});

	test("dates as a device reads them", () => {
		expect(isFieldDate("2024-02-29")).toBe(true);
		expect(isFieldDate("2023-02-29")).toBe(false);
		expect(isFieldDate("2026-10-03T09:00:00.5Z")).toBe(true);
		expect(isFieldDate("2026-10-03T09:00")).toBe(false);
		expect(isFieldDate("")).toBe(false);
	});
});
