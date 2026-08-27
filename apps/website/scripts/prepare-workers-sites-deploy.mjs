import { copyFile, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const appDir = join(scriptDir, "..");
const serverDir = join(appDir, "dist", "server");
const clientDir = join(appDir, "dist", "client");
const wranglerConfigPath = join(serverDir, "wrangler.json");
const staticAssetsEntryPath = join(serverDir, "entry-static-assets.mjs");
const markdownNegotiationPath = join(serverDir, "markdown-negotiation.mjs");

function parseHeadersFile(input) {
	const rules = [];
	let currentRule;

	for (const rawLine of input.split(/\r?\n/)) {
		const line = rawLine.trim();
		if (!line || line.startsWith("#")) continue;

		if (/^\s/.test(rawLine)) {
			const match = line.match(/^([^:]+):\s*(.*)$/);
			if (currentRule && match) {
				currentRule.headers.push([match[1], match[2]]);
			}
			continue;
		}

		currentRule = { pattern: line, headers: [] };
		rules.push(currentRule);
	}

	return rules.filter((rule) => rule.headers.length > 0);
}

const config = JSON.parse(await readFile(wranglerConfigPath, "utf8"));
const assetDirectory = config.assets?.directory ?? "../client";
const headerRules = parseHeadersFile(
	await readFile(join(clientDir, "_headers"), "utf8").catch(() => ""),
);

config.main = "entry-static-assets.mjs";
config.assets = {
	...config.assets,
	directory: assetDirectory,
	binding: "ASSETS",
	run_worker_first: true,
};
delete config.site;

await writeFile(wranglerConfigPath, `${JSON.stringify(config, null, 2)}\n`);

await copyFile(
	join(scriptDir, "agent-markdown", "markdown-negotiation.mjs"),
	markdownNegotiationPath,
);

await writeFile(
	staticAssetsEntryPath,
	`globalThis.process ??= {};
globalThis.process.env ??= {};
import astroWorker from "./entry.mjs";
import { serveMarkdown, withVaryAccept } from "./markdown-negotiation.mjs";

const headerRules = ${JSON.stringify(headerRules, null, "\t")};

function matchesHeaderPattern(pattern, pathname) {
\tlet pathPattern = pattern;
\ttry {
\t\tpathPattern = new URL(pattern).pathname;
\t} catch {}

\tif (pathPattern === "/*") return true;
\tif (pathPattern.endsWith("*")) {
\t\treturn pathname.startsWith(pathPattern.slice(0, -1));
\t}

\treturn pathname === pathPattern || pathname + "/" === pathPattern;
}

function applyResponseHeaders(request, response) {
\tif (!headerRules.length) return response;

\tconst pathname = new URL(request.url).pathname;
\tconst withHeaders = new Response(response.body, response);
\tfor (const rule of headerRules) {
\t\tif (!matchesHeaderPattern(rule.pattern, pathname)) continue;
\t\tfor (const [name, value] of rule.headers) {
\t\t\twithHeaders.headers.set(name, value);
\t\t}
\t}

\treturn withHeaders;
}

export default {
\tasync fetch(request, env, ctx) {
\t\tconst markdown = await serveMarkdown(request, (path) =>
\t\t\tenv.ASSETS.fetch(new URL(path, request.url).toString()),
\t\t);
\t\tif (markdown) return applyResponseHeaders(request, markdown);

\t\tconst response = await astroWorker.fetch(request, env, ctx);
\t\treturn applyResponseHeaders(request, withVaryAccept(response));
\t},
};
`,
);

console.log(
	`Prepared Wrangler deploy config for Workers Static Assets at ${assetDirectory}`,
);
