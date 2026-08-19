import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const workbenchSource = readFileSync(
	fileURLToPath(new URL("./components/workbench-view.tsx", import.meta.url)),
	"utf8",
);
const themeSource = readFileSync(
	fileURLToPath(new URL("./theme.css", import.meta.url)),
	"utf8",
);

describe("workbench viewport containment", () => {
	test("keeps all scrolling inside the fixed viewport shell", () => {
		expect(themeSource).toMatch(
			/html,\s*body\s*{[^}]*overflow:\s*hidden;[^}]*overscroll-behavior:\s*none;/s,
		);
		expect(themeSource).toMatch(
			/#root\s*{[^}]*width:\s*100%;[^}]*min-width:\s*0;[^}]*height:\s*100dvh;[^}]*max-height:\s*100dvh;[^}]*overflow:\s*hidden;[^}]*overscroll-behavior:\s*none;/s,
		);
		expect(workbenchSource).toContain(
			"grid h-0 min-h-0 min-w-0 w-full max-w-full flex-1",
		);
		expect(
			workbenchSource.match(
				/overflow-x-hidden overflow-y-auto overscroll-none/g,
			) ?? [],
		).toHaveLength(3);
		expect(workbenchSource).not.toContain("overflow-y-auto overscroll-contain");
	});

	test("prevents unbroken locale values from widening grid tracks", () => {
		expect(workbenchSource).toContain(
			"relative grid min-w-0 w-full max-w-full",
		);
		expect(workbenchSource).toContain(
			"grid-cols-[minmax(0,1fr)_minmax(0,1fr)_112px] overflow-hidden",
		);
		expect(
			workbenchSource.match(/\[overflow-wrap:anywhere\]/g) ?? [],
		).toHaveLength(3);
		expect(workbenchSource).toContain("max-w-full break-all rounded-full");
	});
});
