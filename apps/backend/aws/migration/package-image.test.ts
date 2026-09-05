import { expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

test("the Dockerfile packages every module needed to import its entrypoint", () => {
	const repo = resolve(import.meta.dir, "../../../..");
	const dockerfile = readFileSync(join(import.meta.dir, "Dockerfile"), "utf8");
	const stage = mkdtempSync(join(tmpdir(), "flowlike-migration-image-"));
	try {
		for (const line of dockerfile.split("\n")) {
			if (!line.startsWith("COPY apps/backend/aws/migration/")) continue;
			for (const source of line.split(/\s+/).slice(1, -1)) {
				copyFileSync(join(repo, source), join(stage, basename(source)));
			}
		}
		symlinkSync(join(import.meta.dir, "node_modules"), join(stage, "node_modules"), "dir");
		const imported = spawnSync(process.execPath, ["-e", 'await import("./migrate.ts")'], {
			cwd: stage, encoding: "utf8",
		});
		expect(imported.stderr.toString()).toBe("");
		expect(imported.status).toBe(0);
	} finally {
		rmSync(stage, { recursive: true, force: true });
	}
});
