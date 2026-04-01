#!/usr/bin/env node
/**
 * Build script for Flow-Like TypeScript WASM nodes.
 *
 * Steps:
 *   1. Bundle TypeScript → single JS file via esbuild (WIT imports remain external)
 *   2. Componentize the bundle → WASM component via @bytecodealliance/componentize-js
 */

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { componentize } from "@bytecodealliance/componentize-js";
import { build } from "esbuild";

const __dirname = dirname(fileURLToPath(import.meta.url));
const outDir = resolve(__dirname, "build");
const distDir = resolve(__dirname, "dist");

async function main() {
	console.log("[1/2] Bundling TypeScript...");

	await build({
		entryPoints: [resolve(__dirname, "src/app.ts")],
		bundle: true,
		outfile: resolve(distDir, "app.js"),
		format: "esm",
		target: "es2022",
		platform: "neutral",
		external: ["flow-like:*"],
	});

	console.log("[2/2] Componentizing to WASM...");

	await mkdir(outDir, { recursive: true });

	const { component } = await componentize({
		sourcePath: resolve(distDir, "app.js"),
		witPath: resolve(__dirname, "wit"),
		worldName: "flow-like-node",
		disableFeatures: ["random", "clocks", "http", "stdio", "fetch-event"],
	});

	const outputPath = resolve(outDir, "node.wasm");
	await writeFile(outputPath, component);

	const sizeMB = (component.byteLength / (1024 * 1024)).toFixed(2);
	console.log(`Done! ${outputPath} (${sizeMB} MB)`);
}

main().catch((err) => {
	console.error("Build failed:", err);
	process.exit(1);
});
