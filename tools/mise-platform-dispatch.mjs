import { spawnSync } from "node:child_process";
import process from "node:process";

function resolveTask(baseTask, platform, arch) {
	const isArm = arch === "arm" || arch === "arm64";

	switch (platform) {
		case "darwin":
			return `${baseTask}:mac:${isArm ? "arm" : "intel"}`;
		case "linux":
			return `${baseTask}:linux:${isArm ? "arm" : "x64"}`;
		case "win32":
			return `${baseTask}:win:${isArm ? "arm" : "x64"}`;
		default:
			throw new Error(`Unsupported platform: ${platform}`);
	}
}

const baseTask = process.argv[2];

if (!baseTask) {
	console.error("Usage: node ./tools/mise-platform-dispatch.mjs <task>");
	process.exit(1);
}

const resolvedTask = resolveTask(baseTask, process.platform, process.arch);
const forwardedArgs = process.argv.slice(3);
const result = spawnSync("mise", ["run", ...forwardedArgs, resolvedTask], {
	env: process.env,
	stdio: "inherit",
});

if (result.error) {
	console.error(`Failed to execute ${resolvedTask}: ${result.error.message}`);
	process.exit(1);
}

process.exit(result.status ?? 1);