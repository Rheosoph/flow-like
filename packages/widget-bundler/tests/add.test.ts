import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { addWidget } from "../src/add";
import { extractContract } from "../src/extract";
import { tmpDir } from "./helpers";

function makeGroup(): string {
	const dir = tmpDir("flwb-group");
	mkdirSync(join(dir, "src"), { recursive: true });
	return dir;
}

describe("addWidget", () => {
	test("scaffolds a widget whose contract extracts cleanly", () => {
		const group = makeGroup();
		const result = addWidget(group, "kpi-card");
		expect(result.widgetDir).toBe(join(group, "src", "widgets", "kpi-card"));
		expect(result.files).toHaveLength(3);
		for (const file of result.files) {
			expect(existsSync(file)).toBeTrue();
		}

		const configPath = join(result.widgetDir, "widget.config.ts");
		expect(readFileSync(configPath, "utf8")).toContain('id: "kpi-card"');

		const { contract, warnings } = extractContract(configPath);
		expect(contract.id).toBe("kpi-card");
		expect(contract.inputs.title?.default).toBe("Kpi Card");
		expect(contract.queries.getTitle).toEqual({
			argsSchema: null,
			resultSchema: { type: "string" },
		});
		expect(warnings).toEqual([]);
	}, 60000);

	test("refuses invalid ids and existing directories", () => {
		const group = makeGroup();
		expect(() => addWidget(group, "Bad_Id")).toThrow(/Invalid widget id/);
		addWidget(group, "twice");
		expect(() => addWidget(group, "twice")).toThrow(/already exists/);
		expect(() => addWidget("/nonexistent-group", "ok-id")).toThrow(/not found/);
	});
});
