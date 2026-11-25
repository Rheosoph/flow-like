import { resolve } from "node:path";

export function parseBuildOptions(
	args,
	{ ids, cwd = process.cwd(), env = process.env },
) {
	const environment = { ...env };
	const selected = [];
	const forwarded = [];
	let development = false;
	let background = false;
	let help = false;

	for (let index = 0; index < args.length; index++) {
		const argument = args[index];
		if (argument === "--cache" || argument === "--report") {
			const value = args[++index];
			if (!value || value.startsWith("--"))
				throw new Error(`${argument} requires a value.`);
			if (argument === "--cache") {
				if (value !== "auto" && value !== "shared")
					throw new Error("--cache must be auto or shared.");
				environment.CRAFT_BUILD_CACHE = value;
				forwarded.push(argument, value);
			} else {
				environment.CRAFT_BUILD_REPORT_DIR = resolve(cwd, value);
				forwarded.push(argument, environment.CRAFT_BUILD_REPORT_DIR);
			}
		} else if (argument === "--rebuild") {
			environment.CRAFT_REBUILD = "1";
			forwarded.push(argument);
		} else if (argument === "--no-stage") {
			environment.CRAFT_SKIP_STAGE = "1";
			forwarded.push(argument);
		} else if (argument === "--dev") {
			development = true;
		} else if (argument === "--background") {
			background = true;
		} else if (argument === "--help" || argument === "-h") {
			help = true;
		} else if (argument.startsWith("-")) {
			throw new Error(`Unknown build option: ${argument}`);
		} else {
			if (!ids.includes(argument))
				throw new Error(
					`Unknown editor: ${argument}. Available editors: ${ids.join(", ")}.`,
				);
			if (!selected.includes(argument)) selected.push(argument);
			forwarded.push(argument);
		}
	}
	if (development && background)
		throw new Error("Use either --dev or --background, not both.");
	const cache = environment.CRAFT_BUILD_CACHE ?? "shared";
	if (cache !== "auto" && cache !== "shared")
		throw new Error("CRAFT_BUILD_CACHE / --cache must be auto or shared.");
	environment.CRAFT_BUILD_CACHE = cache;
	if (environment.CRAFT_BUILD_REPORT_DIR)
		environment.CRAFT_BUILD_REPORT_DIR = resolve(
			cwd,
			environment.CRAFT_BUILD_REPORT_DIR,
		);
	return {
		ids: selected.length ? selected : [...ids],
		development,
		background,
		help,
		cache,
		env: environment,
		backgroundArgs: [...forwarded, "--background"],
	};
}

export function buildUsage(ids) {
	return `Usage: node packages/craft-viewers/build-web.mjs [options] [editor ...]

Build all file editors, or only the named editors.

Options:
  --cache auto|shared  Choose Cargo cache mode (default: CRAFT_BUILD_CACHE or shared).
  --report DIR         Write per-editor reports and suite.json to DIR.
  --rebuild            Run build steps even when packaged assets are cached.
  --no-stage           Leave desktop and web public assets unchanged.
  --dev                Start the requested build in the background.
  --background         Run with the development suite lock without waiting.
  --help, -h           Show this help.

Editors: ${ids.join(", ")}

Compare first and warm runs with the same shared cache:
  node packages/craft-viewers/build-web.mjs --cache shared --rebuild --no-stage --report /tmp/craft-first wordcraft gridcraft
  node packages/craft-viewers/build-web.mjs --cache shared --rebuild --no-stage --report /tmp/craft-warm wordcraft gridcraft

The first run is cold only if its dependencies are not already cached. These
commands preserve existing Cargo caches. Compile timing includes Cargo lock and
queue wait; suite timing measures elapsed wall time, not total CPU time.`;
}
