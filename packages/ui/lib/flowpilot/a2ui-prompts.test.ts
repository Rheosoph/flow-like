import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// Read the prompt source as text: importing the module drags the full component
// tree (CSS included) into the test transform for what is a string-content pin.
const promptSource = readFileSync(
	join(dirname(fileURLToPath(import.meta.url)), "a2ui-prompts.ts"),
	"utf-8",
);

describe("A2UI_SYSTEM_PROMPT actions guidance", () => {
	it("keeps actions invoke-only so board handlers pull element state", () => {
		expect(promptSource).toContain("## Actions");
		expect(promptSource).toContain("An action only INVOKES the named event");
		expect(promptSource).toContain(
			"the board handler reads live input values itself",
		);
		expect(promptSource).toContain("never try to forward what the user typed");
	});
});
