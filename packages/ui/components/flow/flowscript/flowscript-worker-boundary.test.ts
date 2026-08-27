import { expect, test } from "bun:test";
import { fileURLToPath } from "node:url";

test("the language worker cannot reach its main-thread launcher", async () => {
	const result = await Bun.build({
		entrypoints: [
			fileURLToPath(new URL("./flowscript-language.worker.ts", import.meta.url)),
		],
		target: "browser",
		format: "esm",
		metafile: true,
		throw: false,
	});

	expect(result.success).toBe(true);
	const inputs = Object.keys(result.metafile?.inputs ?? {}).map((path) =>
		path.replaceAll("\\", "/"),
	);
	expect(
		inputs.some((path) => path.endsWith("/flowscript-worker-protocol.ts")),
	).toBe(true);
	expect(
		inputs.filter((path) => path.endsWith("/flowscript-worker-client.ts")),
	).toEqual([]);
});
