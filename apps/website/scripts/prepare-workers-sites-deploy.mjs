import { copyFile, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const appDir = join(scriptDir, "..");
const serverDir = join(appDir, "dist", "server");
const clientDir = join(appDir, "dist", "client");
const wranglerConfigPath = join(serverDir, "wrangler.json");
const legacyEntryPath = join(serverDir, "entry-workers-sites.mjs");
const kvAssetHandlerPath = join(serverDir, "kv-asset-handler.mjs");
const markdownNegotiationPath = join(serverDir, "markdown-negotiation.mjs");
const require = createRequire(import.meta.url);

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

config.main = "entry-workers-sites.mjs";
config.site = {
	bucket: assetDirectory,
};
delete config.assets;

await writeFile(wranglerConfigPath, `${JSON.stringify(config, null, 2)}\n`);

await copyFile(
	require.resolve("@cloudflare/kv-asset-handler"),
	kvAssetHandlerPath,
);

await copyFile(
	join(scriptDir, "agent-markdown", "markdown-negotiation.mjs"),
	markdownNegotiationPath,
);

await writeFile(
	legacyEntryPath,
	`globalThis.process ??= {};
globalThis.process.env ??= {};
import astroWorker from "./entry.mjs";
import manifestJSON from "__STATIC_CONTENT_MANIFEST";
import {
\tMethodNotAllowedError,
\tNotFoundError,
\tgetAssetFromKV,
} from "./kv-asset-handler.mjs";
import { serveMarkdown, withVaryAccept } from "./markdown-negotiation.mjs";

const assetManifest = JSON.parse(manifestJSON);
const headerRules = ${JSON.stringify(headerRules, null, "\t")};

function toRequest(input, baseRequest) {
\tif (input instanceof Request) return input;
\treturn new Request(new URL(String(input), baseRequest.url).toString());
}

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

function createAssetsBinding(baseRequest, env, ctx) {
\treturn {
\t\tasync fetch(input) {
\t\t\ttry {
\t\t\t\treturn await getAssetFromKV(
\t\t\t\t\t{
\t\t\t\t\t\trequest: toRequest(input, baseRequest),
\t\t\t\t\t\twaitUntil: (promise) => ctx.waitUntil(promise),
\t\t\t\t\t},
\t\t\t\t\t{
\t\t\t\t\t\tASSET_NAMESPACE: env.__STATIC_CONTENT,
\t\t\t\t\t\tASSET_MANIFEST: assetManifest,
\t\t\t\t\t},
\t\t\t\t);
\t\t\t} catch (error) {
\t\t\t\tif (error instanceof NotFoundError) {
\t\t\t\t\treturn new Response("Not Found", { status: 404 });
\t\t\t\t}
\t\t\t\tif (error instanceof MethodNotAllowedError) {
\t\t\t\t\treturn new Response("Method Not Allowed", { status: 405 });
\t\t\t\t}
\t\t\t\tthrow error;
\t\t\t}
\t\t},
\t};
}

export default {
\tasync fetch(request, env, ctx) {
\t\tconst runtimeEnv = {
\t\t\t...env,
\t\t\tASSETS: env.ASSETS ?? createAssetsBinding(request, env, ctx),
\t\t};

\t\tconst markdown = await serveMarkdown(request, (path) =>
\t\t\truntimeEnv.ASSETS.fetch(new URL(path, request.url).toString()),
\t\t);
\t\tif (markdown) return applyResponseHeaders(request, markdown);

\t\tconst response = await astroWorker.fetch(request, runtimeEnv, ctx);
\t\treturn applyResponseHeaders(request, withVaryAccept(response));
\t},
};
`,
);

console.log(
	`Prepared Wrangler deploy config for Workers Sites assets at ${assetDirectory}`,
);
