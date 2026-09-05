import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const runnerTemp = process.env.RUNNER_TEMP;
const githubPath = process.env.GITHUB_PATH;
if (!runnerTemp || !githubPath) {
	throw new Error(
		"RUNNER_TEMP and GITHUB_PATH must be set to install release tools",
	);
}

const sourceDir = fileURLToPath(new URL("../release-tools/", import.meta.url));
// Installing outside the checkout prevents Bun from installing the monorepo.
const toolsDir = path.join(runnerTemp, "flow-like-release-tools");
fs.mkdirSync(toolsDir, { recursive: true });
for (const file of ["package.json", "bun.lock"]) {
	fs.copyFileSync(path.join(sourceDir, file), path.join(toolsDir, file));
}

execFileSync(
	process.execPath,
	["install", "--frozen-lockfile", "--ignore-scripts"],
	{ cwd: toolsDir, stdio: "inherit" },
);

// The desktop's existing `bun tauri` script can find dotenv and Tauri here.
fs.appendFileSync(
	githubPath,
	`${path.join(toolsDir, "node_modules", ".bin")}\n`,
);
