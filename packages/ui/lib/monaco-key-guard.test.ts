import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const uiRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

function editorFiles(): string[] {
	const out = execFileSync(
		"grep",
		["-rl", "--include=*.tsx", "<Editor", "components"],
		{ cwd: uiRoot, encoding: "utf-8" },
	);
	return out.split("\n").filter(Boolean);
}

describe("Monaco surfaces opt out of React Flow key handling", () => {
	it("marks every Monaco editor with the opt-out class", () => {
		const files = editorFiles();
		expect(files.length).toBeGreaterThan(0);
		const unguarded = files.filter((file) => {
			const source = readFileSync(join(uiRoot, file), "utf-8");
			if (!source.includes("@monaco-editor/react")) return false;
			return !source.includes("FLOW_KEY_OPT_OUT_CLASS");
		});
		expect(unguarded).toEqual([]);
	});
});
