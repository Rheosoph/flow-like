#!/usr/bin/env node

import { copyFile, mkdir, rename, rm, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const bookDirectory = resolve(scriptDirectory, "..");
const sourcePath = resolve(bookDirectory, "../../output/pdf/flowbook.pdf");
const destinationPath = resolve(bookDirectory, "public/flowbook.pdf");
const temporaryPath = `${destinationPath}.${process.pid}.tmp`;

async function publishPdf() {
	let source;
	try {
		source = await stat(sourcePath);
	} catch (error) {
		if (error?.code === "ENOENT") {
			throw new Error(
				"FlowBook PDF is missing at output/pdf/flowbook.pdf. Run `bun run --cwd apps/book pdf` first.",
			);
		}
		throw error;
	}

	if (!source.isFile() || source.size === 0) {
		throw new Error("FlowBook PDF exists but is not a non-empty file.");
	}

	await mkdir(dirname(destinationPath), { recursive: true });
	try {
		await copyFile(sourcePath, temporaryPath);
		await rename(temporaryPath, destinationPath);
	} finally {
		await rm(temporaryPath, { force: true });
	}

	console.log(
		`Published output/pdf/flowbook.pdf to apps/book/public/flowbook.pdf (${source.size.toLocaleString("en-US")} bytes).`,
	);
}

publishPdf().catch((error) => {
	console.error(error instanceof Error ? error.message : error);
	process.exitCode = 1;
});
