import { mkdir, readdir, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import tailwind from "@tailwindcss/postcss";
import postcss from "postcss";

export const PACKAGE_DIR = resolve(import.meta.dir, "../..");

/** Freezes everything that moves on its own so two runs paint the same pixels. */
const DETERMINISM_CSS = `
*, *::before, *::after {
	animation: none !important;
	transition: none !important;
	caret-color: transparent !important;
	scroll-behavior: auto !important;
}
::-webkit-scrollbar { display: none; }
html, body { margin: 0; scroll-behavior: auto !important; }
`;

export type HarnessBuild = {
	readonly dir: string;
	readonly stylesheets: readonly string[];
	readonly bundleBytes: number;
	readonly cssBytes: number;
	readonly durationMs: number;
	readonly warnings: readonly string[];
};

async function compileTailwind(): Promise<string> {
	const from = join(PACKAGE_DIR, "global.css");
	const result = await postcss([tailwind({ base: PACKAGE_DIR })]).process(
		await Bun.file(from).text(),
		{ from },
	);
	return result.css;
}

function pageHtml(stylesheets: readonly string[], mode: string) {
	const links = stylesheets
		.map((href) => `<link rel="stylesheet" href="${href}">`)
		.join("\n");
	return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
${links}
<style>${DETERMINISM_CSS}</style>
<script>window.process = { env: { NODE_ENV: ${JSON.stringify(mode)} } };</script>
</head>
<body>
<div id="app"></div>
<script type="module" src="entry.js"></script>
</body>
</html>
`;
}

export async function buildHarness(
	dir: string,
	mode: "development" | "production",
): Promise<HarnessBuild> {
	const started = performance.now();
	await rm(dir, { recursive: true, force: true });
	await mkdir(dir, { recursive: true });

	const [bundle, css] = await Promise.all([
		Bun.build({
			entrypoints: [join(import.meta.dir, "entry.tsx")],
			outdir: dir,
			target: "browser",
			format: "esm",
			splitting: false,
			minify: false,
			naming: {
				entry: "[name].[ext]",
				asset: "assets/[name]-[hash].[ext]",
			},
			define: { "process.env.NODE_ENV": JSON.stringify(mode) },
			throw: false,
		}),
		compileTailwind(),
	]);
	if (!bundle.success) {
		const details = bundle.logs.map((log) => String(log)).join("\n");
		throw new Error(`editor-visual: bundling entry.tsx failed\n${details}`);
	}
	await Bun.write(join(dir, "tailwind.css"), css);

	const emitted = await readdir(dir);
	const componentCss = emitted
		.filter((name) => name.endsWith(".css") && name !== "tailwind.css")
		.sort();
	const stylesheets = ["tailwind.css", ...componentCss];
	await Bun.write(join(dir, "index.html"), pageHtml(stylesheets, mode));

	return {
		dir,
		stylesheets,
		bundleBytes: bundle.outputs.reduce(
			(total, output) => total + output.size,
			0,
		),
		cssBytes: css.length,
		durationMs: Math.round(performance.now() - started),
		warnings: bundle.logs
			.filter((log) => log.level === "warning")
			.map((log) => String(log)),
	};
}
