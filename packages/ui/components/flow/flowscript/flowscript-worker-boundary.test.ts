import { expect, test } from "bun:test";
import { fileURLToPath } from "node:url";

const BUNDLE_SCRIPT = `const result = await Bun.build({
	entrypoints: [process.argv[1]],
	target: "browser",
	format: "esm",
	metafile: true,
	throw: false,
});
console.log(JSON.stringify({
	errors: result.success ? [] : result.logs.map(String),
	inputs: Object.keys(result.metafile?.inputs ?? {}),
}));`;

// Bundled in a fresh process: a `bun test` path filter without `./` scans the tree first and
// keeps its directory handles open, after which an in-process `Bun.build` of this worker fails
// to resolve `../../../lib/geometry` (Bun 1.3.8).
function bundle(entry: string): { errors: string[]; inputs: string[] } {
	const child = Bun.spawnSync([process.execPath, "-e", BUNDLE_SCRIPT, entry], {
		cwd: fileURLToPath(new URL("../../../", import.meta.url)),
	});
	const stdout = child.stdout.toString().trim();
	if (child.exitCode !== 0 || !stdout) {
		throw new Error(
			`bundling ${entry} exited ${child.exitCode} with no result: ${child.stderr.toString()}`,
		);
	}
	return JSON.parse(stdout);
}

test("the language worker cannot reach its main-thread launcher", () => {
	const { errors, inputs } = bundle(
		fileURLToPath(new URL("./flowscript-language.worker.ts", import.meta.url)),
	);

	expect(errors).toEqual([]);
	const paths = inputs.map((path) => path.replaceAll("\\", "/"));
	expect(
		paths.some((path) => path.endsWith("/flowscript-worker-protocol.ts")),
	).toBe(true);
	expect(
		paths.filter((path) => path.endsWith("/flowscript-worker-client.ts")),
	).toEqual([]);
});
