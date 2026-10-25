import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import manifest from "../../../core/a2ui-schema/src/component-manifest.json";

interface ManifestComponent {
	readonly type: string;
	readonly props: readonly {
		readonly name: string;
		readonly values?: readonly string[];
	}[];
}

const components: readonly ManifestComponent[] = manifest.components;

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

describe("A2UI prompts authoring form", () => {
	it("teaches plain values instead of BoundValue wrappers", () => {
		expect(promptSource).not.toContain("literalString");
		expect(promptSource).not.toContain("explicitList");
		expect(promptSource).toContain('"children": ["header", "body"]');
	});

	it("names only component types the renderer knows", () => {
		const known = new Set(components.map(({ type }) => type));
		const categories = promptSource.slice(
			promptSource.indexOf("## Component Categories"),
			promptSource.indexOf("## Style System"),
		);
		const listed = [...categories.matchAll(/- \*\*(\w+)\*\*/g)].map(
			([, type]) => type,
		);
		expect(listed.length).toBeGreaterThan(30);
		expect(listed.filter((type) => !known.has(type))).toEqual([]);
	});

	it("uses only text variants the renderer understands", () => {
		const text = components.find(({ type }) => type === "text");
		const variants =
			text?.props.find(({ name }) => name === "variant")?.values ?? [];
		const taught = [
			...promptSource.matchAll(/text with "variant": "(\w+)"/g),
		].map(([, variant]) => variant);
		expect(taught.length).toBeGreaterThan(0);
		expect(taught.filter((variant) => !variants.includes(variant))).toEqual([]);
	});
});
