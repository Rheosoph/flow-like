import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
	mkdtemp,
	mkdir,
	readFile,
	rm,
	symlink,
	writeFile,
} from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const tsc = require.resolve("typescript/bin/tsc");
const temporary = await mkdtemp(join(tmpdir(), "flow-like-sdk-package-"));
const keep = process.argv.includes("--keep");

function run(command, args, cwd = packageDir) {
	return execFileSync(command, args, {
		cwd,
		encoding: "utf8",
		stdio: ["ignore", "pipe", "inherit"],
	});
}

async function consumer(name, tarball) {
	const root = join(temporary, name);
	const destination = join(root, "node_modules/@flow-like/sdk");
	await mkdir(destination, { recursive: true });
	run("tar", ["-xzf", tarball, "--strip-components=1", "-C", destination]);
	await writeFile(
		join(root, "package.json"),
		JSON.stringify({ type: "module" }),
	);
	await writeFile(
		join(root, "tsconfig.json"),
		JSON.stringify({
			compilerOptions: {
				target: "ES2022",
				module: "NodeNext",
				moduleResolution: "NodeNext",
				strict: true,
				skipLibCheck: false,
				noEmit: true,
				lib: ["ES2022", "DOM"],
				types: [],
			},
			include: ["smoke.ts"],
		}),
	);
	return root;
}

async function peer(root, name) {
	let peerDir = dirname(
		require.resolve(name === "@types/node" ? `${name}/package.json` : name),
	);
	while (
		JSON.parse(
			await readFile(join(peerDir, "package.json"), "utf8").catch(() => "{}"),
		).name !== name
	) {
		assert.notEqual(
			peerDir,
			dirname(peerDir),
			`Cannot locate installed peer ${name}`,
		);
		peerDir = dirname(peerDir);
	}
	const destination = join(root, "node_modules", name);
	await mkdir(dirname(destination), { recursive: true });
	await symlink(peerDir, destination, "dir");
}

try {
	const manifest = JSON.parse(
		await readFile(join(packageDir, "package.json"), "utf8"),
	);
	const packed = run("npm", [
		"pack",
		"--json",
		"--pack-destination",
		temporary,
		"--cache",
		join(temporary, "npm-cache"),
	]);
	const [entry] = JSON.parse(packed.slice(packed.indexOf("[")));
	assert.equal(entry.version, manifest.version);
	for (const path of [
		"dist/index.js",
		"dist/index.d.ts",
		"dist/langchain.js",
		"dist/langchain.d.ts",
		"dist/lancedb.js",
		"dist/lancedb.d.ts",
		"src/index.ts",
	])
		assert.ok(
			entry.files.some((file) => file.path === path),
			`Package missing ${path}`,
		);
	assert.ok(
		entry.files.every(
			(file) =>
				!file.path.startsWith("node_modules/") &&
				!file.path.startsWith("tests/") &&
				!file.path.includes(".env"),
		),
	);
	const tarball = join(temporary, entry.filename);
	const core = await consumer("core", tarball);
	await writeFile(
		join(core, "smoke.ts"),
		`
import { FlowLikeClient, type PollResult } from "@flow-like/sdk";
const sdk = new FlowLikeClient({ baseUrl: "https://flow.test", pat: "pat_test" });
const poll: Promise<PollResult> = sdk.pollExecution("poll");
void poll;
void sdk.uploadFile("a", new Uint8Array([1, 2]), { key: "f" });
void sdk.createLanceConnection("a").then(db => db.tableNames());
void sdk.asLangChainChat("model").then(chat => chat.invoke("hello"));
`,
	);
	run(process.execPath, [tsc, "-p", join(core, "tsconfig.json")]);
	await writeFile(
		join(core, "smoke.mjs"),
		`
import assert from "node:assert/strict";
import { FlowLikeClient } from "@flow-like/sdk";
const sdk = new FlowLikeClient({ baseUrl: "https://flow.test/prefix/api/v1", pat: "pat_test" });
globalThis.fetch = async (url, options) => {
  assert.equal(url, "https://flow.test/prefix/api/v1/health");
  assert.equal(new Headers(options.headers).get("Authorization"), "pat_test");
  assert.equal(options.credentials, "omit");
  return Response.json({ healthy: true });
};
assert.deepEqual(await sdk.health(), { healthy: true });
`,
	);
	run(process.execPath, [join(core, "smoke.mjs")]);
	const adapters = await consumer("adapters", tarball);
	await peer(adapters, "@lancedb/lancedb");
	await peer(adapters, "@langchain/core");
	await peer(adapters, "@types/node");
	await writeFile(
		join(adapters, "smoke.ts"),
		`
import { FlowLikeClient } from "@flow-like/sdk";
import { createLanceConnection, type LanceConnection } from "@flow-like/sdk/lancedb";
import { asLangChainChat, asLangChainEmbeddings, FlowLikeChatModel, FlowLikeEmbeddings } from "@flow-like/sdk/langchain";
const sdk = new FlowLikeClient({ baseUrl: "https://flow.test", pat: "pat_test" });
const db: Promise<LanceConnection> = createLanceConnection(sdk, "a");
const chat: Promise<FlowLikeChatModel> = asLangChainChat(sdk, "model");
const embeddings: Promise<FlowLikeEmbeddings> = asLangChainEmbeddings(sdk, "model");
void db; void chat; void embeddings;
`,
	);
	run(process.execPath, [
		tsc,
		"-p",
		join(adapters, "tsconfig.json"),
		"--types",
		"node",
	]);
	await writeFile(
		join(adapters, "smoke.mjs"),
		`
import assert from "node:assert/strict";
import { FlowLikeClient } from "@flow-like/sdk";
import { createLanceConnection } from "@flow-like/sdk/lancedb";
import { asLangChainChat, asLangChainEmbeddings, FlowLikeChatModel, FlowLikeEmbeddings } from "@flow-like/sdk/langchain";
const sdk = new FlowLikeClient({ baseUrl: "https://flow.test", pat: "pat_test" });
assert.equal(typeof createLanceConnection, "function");
assert.ok(await asLangChainChat(sdk, "model") instanceof FlowLikeChatModel);
assert.ok(await asLangChainEmbeddings(sdk, "model") instanceof FlowLikeEmbeddings);
`,
	);
	run(process.execPath, [join(adapters, "smoke.mjs")]);
	console.log(
		`Package ${entry.name}@${entry.version}: isolated core and optional adapter type/runtime checks passed on ${process.version}.`,
	);
	if (keep) console.log(`Tarball retained at ${tarball}`);
} finally {
	if (!keep) await rm(temporary, { recursive: true, force: true });
}
