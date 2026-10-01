import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { FlowLikeClient } from "../../../libs/platform/nodejs/src/index";

const source = readFileSync(
	new URL("../src/content/docs/dev/sdks/nodejs.mdx", import.meta.url),
	"utf8",
);
function snippet(title: string): string {
	const fence = `\x60\x60\x60typescript title="${title}"\n`;
	const start = source.indexOf(fence);
	assert(start >= 0, `Missing documented example: ${title}`);
	return source.slice(start + fence.length).split("\n```", 1)[0];
}
function withoutImport(code: string): string {
	return code.replace(/^import .* from "@flow-like\/sdk";\n/gm, "");
}
const titles = [
	"environment-client.ts",
	"pat-client.ts",
	"api-key-client.ts",
	"chat-completions.ts",
	"responses.ts",
];
const virtualFile = fileURLToPath(
	new URL("./__sdk_examples__.ts", import.meta.url),
);
const sdkFile = fileURLToPath(
	new URL("../../../libs/platform/nodejs/src/index.ts", import.meta.url),
);
const virtualSource = [
	`import { FlowLikeClient } from ${JSON.stringify(sdkFile)};`,
	...titles.map((title, index) => {
		const parameter = title.endsWith("client.ts")
			? ""
			: "client: FlowLikeClient";
		return `async function example${index}(${parameter}) {\n${withoutImport(snippet(title))}\n}`;
	}),
].join("\n");
const options: ts.CompilerOptions = {
	target: ts.ScriptTarget.ES2022,
	module: ts.ModuleKind.ESNext,
	moduleResolution: ts.ModuleResolutionKind.Bundler,
	strict: true,
	noEmit: true,
	skipLibCheck: true,
	allowImportingTsExtensions: true,
};
const host = ts.createCompilerHost(options);
const originalGetSourceFile = host.getSourceFile.bind(host);
host.getSourceFile = (
	name,
	languageVersion,
	onError,
	shouldCreateNewSourceFile,
) =>
	name === virtualFile
		? ts.createSourceFile(name, virtualSource, languageVersion, true)
		: originalGetSourceFile(
				name,
				languageVersion,
				onError,
				shouldCreateNewSourceFile,
			);
const program = ts.createProgram([virtualFile], options, host);
const diagnostics = ts
	.getPreEmitDiagnostics(program)
	.filter((diagnostic) => diagnostic.file?.fileName === virtualFile);
assert.equal(
	diagnostics.length,
	0,
	ts.formatDiagnosticsWithColorAndContext(diagnostics, {
		getCurrentDirectory: () => process.cwd(),
		getCanonicalFileName: (name) => name,
		getNewLine: () => "\n",
	}),
);

const oldFetch = globalThis.fetch;
const envKeys = [
	"FLOW_LIKE_BASE_URL",
	"FLOW_LIKE_PAT",
	"FLOW_LIKE_API_KEY",
] as const;
const savedEnv = new Map(envKeys.map((key) => [key, process.env[key]]));
const calls: {
	path: string;
	headers: Headers;
	body: Record<string, unknown>;
}[] = [];
let legacyModel = false;
const json = (body: unknown) =>
	new Response(JSON.stringify(body), {
		headers: { "Content-Type": "application/json" },
	});
globalThis.fetch = (async (
	input: string | URL | Request,
	init?: RequestInit,
) => {
	const url = new URL(input instanceof Request ? input.url : input);
	assert.equal(url.origin, "https://api.flow-like.com");
	const body = init?.body ? JSON.parse(String(init.body)) : {};
	calls.push({ path: url.pathname, headers: new Headers(init?.headers), body });
	if (url.pathname === "/api/v1/apps") return json([]);
	if (url.pathname === "/api/v1/bit") {
		return json(
			["Responses", "ChatCompletions"].map((surface) => ({
				id: surface,
				type: "Llm",
				meta: { en: { name: surface } },
				parameters: {
					provider: {
						provider_name: "Hosted OpenAI",
						...(legacyModel && surface === "ChatCompletions"
							? {}
							: { api_surface: surface }),
					},
				},
			})),
		);
	}
	if (url.pathname === "/api/v1/chat/completions") {
		assert.equal(body.model, "ChatCompletions");
		assert(Array.isArray(body.messages));
		assert.equal(body.max_tokens, 200);
		return json({
			choices: [{ message: { role: "assistant", content: "Mock answer" } }],
		});
	}
	if (url.pathname === "/api/v1/responses") {
		assert.equal(body.model, "Responses");
		assert.equal(typeof body.input, "string");
		assert.equal(body.max_output_tokens, 200);
		return json({ output: [] });
	}
	throw new Error(`Unexpected request in documentation check: ${url.pathname}`);
}) as typeof fetch;

async function run(title: string, client?: FlowLikeClient, suffix = "") {
	const prelude = title.endsWith("client.ts")
		? ""
		: "const client = providedClient;\n";
	const code = ts.transpileModule(
		prelude + withoutImport(snippet(title)) + suffix,
		{
			compilerOptions: {
				target: ts.ScriptTarget.ES2022,
				module: ts.ModuleKind.ESNext,
			},
		},
	).outputText;
	const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
	return new AsyncFunction("FlowLikeClient", "providedClient", "console", code)(
		FlowLikeClient,
		client,
		{ log() {} },
	);
}

try {
	for (const key of envKeys) delete process.env[key];
	for (const title of ["pat-client.ts", "api-key-client.ts"]) {
		await run(title, undefined, "\nawait client.listApps();");
		const headers = calls.at(-1)!.headers;
		if (title === "pat-client.ts") {
			assert.equal(headers.get("Authorization"), "pat_myid.mysecret");
			assert.equal(headers.get("X-API-Key"), null);
		} else {
			assert.equal(headers.get("X-API-Key"), "flk_appid.keyid.secret");
			assert.equal(headers.get("Authorization"), null);
		}
	}
	process.env.FLOW_LIKE_BASE_URL = "https://api.flow-like.com";
	process.env.FLOW_LIKE_PAT = "pat_mock.example";
	await run("environment-client.ts", undefined, "\nawait client.listApps();");
	assert.equal(calls.at(-1)!.headers.get("Authorization"), "pat_mock.example");
	const client = new FlowLikeClient();
	for (const legacy of [false, true]) {
		legacyModel = legacy;
		await run("chat-completions.ts", client);
		await run("responses.ts", client);
	}
	console.log(
		"Node.js SDK docs: 5 examples type-checked; auth and both model APIs passed with mocked fetch.",
	);
} finally {
	globalThis.fetch = oldFetch;
	for (const [key, value] of savedEnv) {
		if (value === undefined) delete process.env[key];
		else process.env[key] = value;
	}
}
