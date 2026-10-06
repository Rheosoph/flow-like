import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { generateSystemOneTypes } from "./systemone-schema";

test("native question branches keep their discriminants and required criteria", () => {
	const schema = JSON.parse(
		readFileSync("packages/schema/bit/bit/systemone-question.json", "utf8"),
	);
	const output = generateSystemOneTypes(schema);
	expect(output).toContain('"type": "choice";');
	expect(output).toContain('"criteria": { [property: string]: unknown; };');
	expect(output).toContain('"criteria": Array<unknown>;');
	expect(output).toContain('"criteria"?: (ISystemOneNoulCriteria) | (null);');
	expect(output).toContain('"instructions": unknown;');
	schema.oneOf[0].required = ["type", "criteria"];
	expect(generateSystemOneTypes(schema)).toContain('"instructions"?: unknown;');
});

test("requests share question types and responses retain required answer fields", () => {
	const read = (name: string) =>
		JSON.parse(
			readFileSync(`packages/schema/bit/bit/systemone-${name}.json`, "utf8"),
		);
	const request = generateSystemOneTypes(read("request"));
	expect(request).toContain(
		'import type { ISystemOneQuestion } from "./systemone-question";',
	);
	expect(request).toContain('"images"?: Array<string>;');
	expect(request).not.toContain("export type ISystemOneQuestion");
	const response = generateSystemOneTypes(read("response"));
	expect(response).toContain('"choice": string;');
	expect(response).toContain('"score": number;');
	expect(response).toContain('"noul": number;');
	expect(response).toContain("export type ISystemOneUsage");
	expect(generateSystemOneTypes(read("parameters"))).toContain(
		'import type { IModelProvider } from "./llm-parameters";',
	);
});

test("unrelated schemas use quicktype and unsupported references fail explicitly", () => {
	expect(
		generateSystemOneTypes({ title: "Bit", type: "object" }),
	).toBeUndefined();
	expect(() =>
		generateSystemOneTypes({
			title: "SystemOneRequest",
			$ref: "#/$defs/Missing",
		}),
	).toThrow("Unsupported schema reference");
});
