import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { createReadStream, existsSync, readFileSync, rmSync } from "node:fs";
import {
	cp,
	mkdir,
	readFile,
	readdir,
	realpath,
	rename,
	rm,
	stat,
	symlink,
	writeFile,
} from "node:fs/promises";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import {
	basename,
	dirname,
	extname,
	join,
	relative,
	resolve,
	sep,
} from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const [command = "help", ...arguments_] = process.argv.slice(2);
if (command === "help" || command === "--help") {
	console.log(
		"node tests/widget-action-stack/web.mjs prepare|build|serve|dev [--state /tmp/flow-widget-stack] [--port 3440] [--api-origin http://127.0.0.1:8440]\nBuild and serve the real apps/web in an isolated copy. Production uses its static export; dev is for smoke checks only. Existing source .next/out directories are untouched.",
	);
	process.exit(0);
}
assert.ok(
	["prepare", "build", "serve", "dev"].includes(command),
	"Unknown command",
);
const options = {
	state: "/tmp/flow-widget-stack",
	port: "3440",
	"api-origin": "http://127.0.0.1:8440",
};
for (let index = 0; index < arguments_.length; index++) {
	const key = arguments_[index].replace(/^--/, "");
	assert.ok(Object.hasOwn(options, key), `Unknown option --${key}`);
	options[key] = arguments_[++index];
	assert.ok(options[key], `Missing value for --${key}`);
}
const port = Number(options.port);
assert.ok(Number.isInteger(port) && port > 0 && port < 65536, "Invalid port");
const api = new URL(options["api-origin"]);
assert.ok(
	["localhost", "127.0.0.1", "[::1]"].includes(api.hostname) &&
		["http:", "https:"].includes(api.protocol) &&
		!api.username &&
		!api.password,
	"The benchmark API must be local",
);
const requestedState = resolve(options.state);
const state = await realpath(requestedState).catch(async () =>
	join(
		await realpath(dirname(requestedState)).catch(() =>
			dirname(requestedState),
		),
		basename(requestedState),
	),
);
const defaultState = join(await realpath("/tmp"), "flow-widget-stack");
const stageSuffix =
	state === defaultState
		? ""
		: `-${createHash("sha256").update(state).digest("hex").slice(0, 16)}`;
const workspace = join(
	repo,
	`tmp/widget-action-stack/web-source${stageSuffix}`,
);
const source = join(repo, "apps/web");
const stage = join(workspace, "apps/web");
const marker = join(workspace, "benchmark-stage.json");
const origin = `http://127.0.0.1:${port}`;
const manifest = {
	fixture: "flow-like-widget-web-v1",
	source,
	stage,
	apiOrigin: api.origin,
	webOrigin: origin,
	stateDirectory: state,
};
await mkdir(workspace, { recursive: true });
const lockPath = join(workspace, "operation.lock");
const lockId = randomUUID();
try {
	await writeFile(
		lockPath,
		JSON.stringify({
			id: lockId,
			pid: process.pid,
			command,
			startedAt: new Date().toISOString(),
		}),
		{ flag: "wx", mode: 0o600 },
	);
} catch (error) {
	if (error.code === "EEXIST")
		throw new Error(
			`Another web benchmark operation owns ${lockPath}. Stop that operation before reusing this stage. A stale lock can be removed after verifying its recorded process has exited.`,
		);
	throw error;
}
process.once("exit", () => {
	try {
		if (JSON.parse(readFileSync(lockPath, "utf8")).id === lockId)
			rmSync(lockPath);
	} catch {}
});

async function verifyStage() {
	const previous = JSON.parse(await readFile(marker, "utf8"));
	assert.equal(previous.fixture, manifest.fixture, "Unknown web stage");
	assert.equal(previous.source, source, "Stage belongs to another checkout");
	if (previous.stateDirectory !== undefined)
		assert.equal(
			previous.stateDirectory,
			state,
			"Stage belongs to another fixture state directory",
		);
}
async function prepare() {
	if (existsSync(marker)) await verifyStage();
	else assert.ok(!existsSync(stage), "Refusing to overwrite an unmarked stage");
	await mkdir(stage, { recursive: true });
	for (const entry of await readdir(stage)) {
		if (![".next", "out"].includes(entry))
			await rm(join(stage, entry), { recursive: true, force: true });
	}
	// Copy source so Next's generated config, assets, and build products stay isolated.
	await cp(source, stage, {
		recursive: true,
		filter: (path) =>
			!relative(source, path)
				.split(sep)
				.some(
					(part) =>
						["node_modules", ".next", "out", ".git"].includes(part) ||
						part.startsWith(".env"),
				),
	});
	for (const [name, target] of [
		["packages", join(repo, "packages")],
		["node_modules", join(repo, "node_modules")],
		["apps/desktop", join(repo, "apps/desktop")],
	]) {
		const link = join(workspace, name);
		if (!existsSync(link)) await symlink(target, link, "dir");
	}
	await rename(
		join(stage, "next.config.mjs"),
		join(stage, "benchmark-source-config.mjs"),
	);
	await writeFile(
		join(stage, "next.config.mjs"),
		`import original from './benchmark-source-config.mjs';\nexport default async (phase) => {\n const config = await original(phase);\n return {...config, turbopack: {...config.turbopack, root: ${JSON.stringify(repo)}}, experimental: {...config.experimental, cpus: 2}};\n};\n`,
	);
	const tsconfig = JSON.parse(
		await readFile(join(stage, "tsconfig.json"), "utf8"),
	);
	// Tests use relative workspace imports. A copied app would give shared classes two identities.
	tsconfig.exclude = [
		...(tsconfig.exclude ?? []),
		"**/*.test.ts",
		"**/*.test.tsx",
		"**/*.spec.ts",
		"**/*.spec.tsx",
	];
	tsconfig.include = tsconfig.include.map((path) =>
		path.startsWith("../../packages/") ? resolve(source, path) : path,
	);
	await writeFile(
		join(stage, "tsconfig.json"),
		`${JSON.stringify(tsconfig, null, 2)}\n`,
	);
	await writeFile(marker, `${JSON.stringify(manifest, null, 2)}\n`);
	await mkdir(state, { recursive: true });
	await writeFile(
		join(state, "web-stage.json"),
		`${JSON.stringify(manifest, null, 2)}\n`,
	);
}
async function sourceIdentity() {
	const inputs = [
		"apps/web",
		"packages/ui",
		"packages/locales",
		"packages/board-canvas",
		"packages/craft-viewers",
		"packages/device-crypto",
		"packages/widget-sdk",
		"package.json",
		"bun.lock",
	];
	const paths = execFileSync(
		"git",
		[
			"ls-files",
			"-z",
			"--cached",
			"--others",
			"--exclude-standard",
			"--",
			...inputs,
		],
		{ cwd: repo, encoding: "utf8" },
	)
		.split("\0")
		.filter(Boolean)
		.sort();
	const digest = createHash("sha256");
	let files = 0;
	for (const path of paths) {
		if (path.split("/").some((part) => part.startsWith(".env"))) continue;
		if (path === "apps/web/next-env.d.ts") continue;
		const file =
			path === "apps/web/next.config.mjs"
				? join(stage, "benchmark-source-config.mjs")
				: path.startsWith("apps/web/")
					? join(stage, relative("apps/web", path))
					: join(repo, path);
		if (!existsSync(file)) continue;
		digest
			.update(path)
			.update("\0")
			.update(await readFile(file))
			.update("\0");
		files++;
	}
	digest
		.update("benchmark-next-config-wrapper\0")
		.update(await readFile(join(stage, "next.config.mjs")))
		.update("\0");
	return {
		hashVersion: 2,
		scope:
			"Git-visible app and shared-package sources, staged Next configuration, package.json, and bun.lock. Environment files, generated next-env declarations, ignored generated assets, installed dependency contents, and build artifacts are excluded.",
		sha256: digest.digest("hex"),
		files,
		gitRevision: execFileSync("git", ["rev-parse", "HEAD"], {
			cwd: repo,
			encoding: "utf8",
		}).trim(),
	};
}
const environment = {
	...process.env,
	NEXT_TELEMETRY_DISABLED: "1",
	NEXT_PUBLIC_API_URL: api.origin,
	NEXT_PUBLIC_REDIRECT_URL: `${origin}/callback`,
	NEXT_PUBLIC_REDIRECT_LOGOUT_URL: `${origin}/`,
	NEXT_PUBLIC_SITE_URL: origin,
	NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG: "0",
};

if (command === "prepare" || command === "build" || command === "dev")
	await prepare();
else await verifyStage();
if (command === "prepare") {
	console.log(`Prepared isolated web source: ${stage}`);
	process.exit(0);
}
if (command === "build") {
	// A failed rebuild must not leave a success manifest beside partially replaced output.
	await rm(join(workspace, "build.json"), { force: true });
	const sourceIdentityBefore = await sourceIdentity();
	const nextCli = createRequire(import.meta.url).resolve("next/dist/bin/next");
	const started = Date.now();
	const child = spawn(
		process.execPath,
		[nextCli, "build", stage, "--turbopack"],
		{
			cwd: stage,
			env: {
				...environment,
				NODE_ENV: "production",
				NODE_OPTIONS: process.env.NODE_OPTIONS || "--max-old-space-size=8192",
			},
			stdio: "inherit",
		},
	);
	for (const signal of ["SIGINT", "SIGTERM"])
		process.once(signal, () => child.kill(signal));
	const code = await new Promise((resolve, reject) => {
		child.once("error", reject);
		child.once("exit", resolve);
	});
	if (code !== 0) process.exit(code ?? 1);
	const sourceIdentityAfter = await sourceIdentity();
	assert.equal(
		sourceIdentityBefore.sha256,
		sourceIdentityAfter.sha256,
		"Browser source changed during the build; rebuild for a reproducible measurement",
	);
	await writeFile(
		join(workspace, "build.json"),
		`${JSON.stringify({ ...manifest, builtAt: new Date().toISOString(), buildMs: Date.now() - started, mode: "production-static-export", source: sourceIdentityAfter, buildId: (await readFile(join(stage, ".next/BUILD_ID"), "utf8")).trim(), nodeVersion: process.version, nextVersion: JSON.parse(await readFile(createRequire(import.meta.url).resolve("next/package.json"), "utf8")).version, stagingOverrides: { compileWorkers: 2, excludesTestsFromTypeCheck: true, includesProductionTypeCheck: true } }, null, 2)}\n`,
	);
	process.exit(0);
}
const started = performance.now();
let next;
let handle;
let build;
if (command === "dev") {
	Object.assign(process.env, environment, { NODE_ENV: "development" });
	const createNext = (await import("next")).default;
	next = createNext({
		dev: true,
		dir: stage,
		hostname: "127.0.0.1",
		port,
		turbopack: true,
	});
	await next.prepare();
	handle = next.getRequestHandler();
} else {
	build = JSON.parse(await readFile(join(workspace, "build.json"), "utf8"));
	assert.equal(
		build.apiOrigin,
		api.origin,
		"Rebuild after changing the API origin",
	);
	assert.equal(build.webOrigin, origin, "Rebuild after changing the web port");
	assert.ok(
		existsSync(join(stage, "out/use.html")) ||
			existsSync(join(stage, "out/use/index.html")),
		"The production /use export is missing",
	);
}
const mime = {
	".html": "text/html; charset=utf-8",
	".js": "text/javascript; charset=utf-8",
	".mjs": "text/javascript; charset=utf-8",
	".css": "text/css; charset=utf-8",
	".json": "application/json",
	".txt": "text/plain; charset=utf-8",
	".svg": "image/svg+xml",
	".png": "image/png",
	".jpg": "image/jpeg",
	".woff": "font/woff",
	".woff2": "font/woff2",
	".wasm": "application/wasm",
	".ico": "image/x-icon",
};
const output = join(stage, "out");
const server = createServer(async (request, response) => {
	try {
		const url = new URL(request.url, origin);
		if (url.pathname === "/__benchmark/health") {
			response.setHeader("Content-Type", "application/json");
			response.setHeader("Cache-Control", "no-store");
			return response.end(
				JSON.stringify({
					mode:
						command === "dev"
							? "development-smoke"
							: "production-static-export",
					startupMs: readyMs,
					apiOrigin: api.origin,
					build: build && {
						builtAt: build.builtAt,
						buildMs: build.buildMs,
						source: build.source,
						buildId: build.buildId,
						nodeVersion: build.nodeVersion,
						nextVersion: build.nextVersion,
						stagingOverrides: build.stagingOverrides,
						provenance: build.provenance,
					},
				}),
			);
		}
		if (handle) return handle(request, response);
		if (!["GET", "HEAD"].includes(request.method))
			return response.writeHead(405).end();
		const pathname = decodeURIComponent(url.pathname);
		const candidate = resolve(output, `.${pathname}`);
		if (!candidate.startsWith(`${output}${sep}`) && candidate !== output)
			return response.writeHead(400).end();
		const candidates = [
			candidate,
			`${candidate}.html`,
			join(candidate, "index.html"),
		];
		if (/^\/use(?:\/|$)/.test(pathname))
			candidates.push(join(output, "use.html"), join(output, "use/index.html"));
		if (/^\/a(?:\/|$)/.test(pathname))
			candidates.push(join(output, "a.html"), join(output, "a/index.html"));
		for (const path of candidates) {
			const info = await stat(path).catch(() => null);
			if (!info?.isFile()) continue;
			response.setHeader(
				"Content-Type",
				mime[extname(path)] ?? "application/octet-stream",
			);
			response.setHeader("Content-Length", info.size);
			response.setHeader(
				"Cache-Control",
				pathname.startsWith("/_next/static/")
					? "public, max-age=31536000, immutable"
					: "no-cache",
			);
			if (request.method === "HEAD") return response.end();
			return createReadStream(path).pipe(response);
		}
		response.writeHead(404).end("Not found");
	} catch {
		response.writeHead(500).end("Local web server error");
	}
});
const readyMs = performance.now() - started;
await new Promise((resolve, reject) => {
	server.once("error", reject);
	server.listen(port, "127.0.0.1", resolve);
});
console.log(
	`Real web ready at ${origin} (${command === "dev" ? "development smoke" : "production static export"})`,
);
for (const signal of ["SIGINT", "SIGTERM"])
	process.once(signal, async () => {
		server.close();
		await next?.close();
		process.exit(0);
	});
