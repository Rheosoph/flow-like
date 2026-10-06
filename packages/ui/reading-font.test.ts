import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const REPO = join(import.meta.dir, "../..");
const read = (path: string) => readFileSync(join(REPO, path), "utf8");

const HOST_LAYOUTS = [
	"apps/desktop/app/layout.tsx",
	"apps/web/app/layout.tsx",
	"apps/standalone/frontend/app/layout.tsx",
];

describe("the reading serif", () => {
	test("the answer font asks for --font-reading and keeps the family name and a serif stack as its fallback", () => {
		const declaration =
			/--fl-chat-prose-font:\s*([^;]+);/.exec(
				read("packages/ui/global.css"),
			)?.[1] ?? "";
		expect(declaration).toMatch(/^var\(--font-reading,\s*"Source Serif 4"\)/);
		expect(declaration).toMatch(/Georgia,\s*ui-serif,\s*serif\s*$/);
	});

	for (const layout of HOST_LAYOUTS) {
		test(`${layout} loads Source Serif 4 with its optical-size axis and puts the variable on <html>`, () => {
			const source = read(layout);
			const call = /(\w+)\s*=\s*Source_Serif_4\(\{([^}]*)\}\)/.exec(source);
			expect(call).not.toBeNull();
			const [, name, options] = call as RegExpExecArray;
			expect(options).toMatch(/axes:\s*\["opsz"\]/);
			expect(options).toMatch(/variable:\s*"--font-reading"/);
			expect(source).toMatch(
				new RegExp(`<html[^>]*className=\\{[^}]*\\b${name}\\.variable`),
			);
		});
	}
});
