import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const packagePath = createRequire(import.meta.url).resolve(
	"maplibre-gl/package.json",
);
const { version } = JSON.parse(readFileSync(packagePath, "utf8"));

export function prepareMapLibreAssets(publicDirectory, basePath = "") {
	const destination = join(
		publicDirectory instanceof URL
			? fileURLToPath(publicDirectory)
			: publicDirectory,
		"maplibre",
		version,
	);
	mkdirSync(destination, { recursive: true });
	// Next's asset pipeline does not emit the worker's relative imports.
	for (const file of ["maplibre-gl-worker.mjs", "maplibre-gl-shared.mjs"]) {
		copyFileSync(
			join(dirname(packagePath), "dist", file),
			join(destination, file),
		);
	}
	copyFileSync(
		join(dirname(packagePath), "LICENSE.txt"),
		join(destination, "LICENSE.txt"),
	);
	return `${basePath}/maplibre/${version}/maplibre-gl-worker.mjs`;
}
