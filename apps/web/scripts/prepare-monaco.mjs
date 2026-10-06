import { copyFileSync, cpSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const packagePath = require.resolve("monaco-editor/package.json");
const packageDirectory = dirname(packagePath);
const { version } = JSON.parse(readFileSync(packagePath, "utf8"));

export function prepareMonacoAssets(
	publicDirectory = fileURLToPath(new URL("../public/", import.meta.url)),
) {
	const destination = join(publicDirectory, "monaco", version);
	// Keep workers, styles, fonts, and language chunks beside Monaco's loader.
	cpSync(join(packageDirectory, "min/vs"), join(destination, "vs"), {
		recursive: true,
	});
	for (const notice of ["LICENSE", "ThirdPartyNotices.txt"]) {
		copyFileSync(join(packageDirectory, notice), join(destination, notice));
	}
}
