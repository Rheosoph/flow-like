import { spawnSync } from "node:child_process";
import path from "node:path";

const BIOME_FILE = /\.(?:[cm]?[jt]sx?|json|jsonc|css)$/;
const [prefix = "."] = process.argv.slice(2);

function git(args) {
	const result = spawnSync("git", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
	if (result.status !== 0) {
		throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
	}
	return result.stdout;
}

const base = process.env.BIOME_BASE_REF || git(["merge-base", "origin/dev", "HEAD"]).trim();
const scope = path.posix.normalize(prefix).replace(/\/$/, "");
const files = git(["diff", "--name-only", "--diff-filter=d", "--no-renames", base, "--", scope])
	.split("\n")
	.filter((file) => BIOME_FILE.test(file));

if (files.length === 0) {
	console.log(`No Biome-checked files changed under ${scope} since ${base.slice(0, 10)}.`);
	process.exit(0);
}

console.log(`Biome check on ${files.length} file(s) changed under ${scope} since ${base.slice(0, 10)}`);
const check = spawnSync(
	"bunx",
	["biome", "check", "--no-errors-on-unmatched", "--diagnostic-level=error", ...files],
	{ stdio: "inherit" },
);
process.exit(check.status ?? 1);
