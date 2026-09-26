import { expect, test } from "bun:test";
import { mkdirSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { validateSchema } from "@flow-like/widget-sdk/validate";
import { extractContract } from "../src/extract";
import { tmpDir } from "./helpers";

function widget(source: string) {
	const dir = tmpDir("flwb-llm");
	mkdirSync(join(dir, "node_modules/@flow-like"), { recursive: true });
	symlinkSync(
		join(import.meta.dir, "../../widget-sdk"),
		join(dir, "node_modules/@flow-like/widget-sdk"),
		"dir",
	);
	writeFileSync(
		join(dir, "tsconfig.json"),
		JSON.stringify({
			compilerOptions: {
				module: "ESNext",
				moduleResolution: "Bundler",
				target: "ES2022",
				strict: true,
				skipLibCheck: true,
			},
		}),
	);
	const config = join(dir, "widget.config.ts");
	writeFileSync(config, source);
	return config;
}

const marker = (kind: string) => ({
	type: "object",
	"x-flow-like-type": "llm",
	"x-llm": kind,
});

test("SDK model types become native LLM markers in inputs, events and query args and results", () => {
	const { contract } = extractContract(
		widget(`import { defineWidget, type LlmHistory, type LlmResponse, type LlmResponseChunk } from "@flow-like/widget-sdk";
interface Inputs { history?: LlmHistory; }
interface Events { asked: { requestId: string; history: LlmHistory } }
interface PushArgs {
	requestId: string;
	/** Streamed from a model node */
	chunk: LlmResponseChunk;
	response?: LlmResponse | null;
}
interface Queries { push: { args: PushArgs; returns: LlmResponse } }
export default defineWidget<Inputs, Events, Queries>({ id: "llm-widget", name: "LLM" });`),
	);
	expect(contract.inputs.history?.schema).toMatchObject(marker("History"));
	const asked = contract.events.asked?.payloadSchema as {
		properties: Record<string, unknown>;
	};
	expect(asked.properties.history).toMatchObject(marker("History"));
	const args = contract.queries.push?.argsSchema as {
		properties: Record<string, Record<string, unknown>>;
		required: string[];
	};
	expect(args.properties.chunk).toEqual({
		...marker("ResponseChunk"),
		description: "Streamed from a model node",
	});
	expect(args.properties.response).toEqual({
		anyOf: [expect.objectContaining(marker("Response")), { type: "null" }],
	});
	expect([...args.required].sort()).toEqual(["chunk", "requestId"]);
	expect(contract.queries.push?.resultSchema).toMatchObject(marker("Response"));
	// The contract carries only the marker; the SDK validates against the native schema.
	const chunk = {
		id: "chunk-1",
		choices: [{ index: 0, delta: { role: "assistant", content: "Hi" } }],
	};
	expect(
		validateSchema(contract.queries.push?.argsSchema, {
			requestId: "r",
			chunk,
		}).valid,
	).toBe(true);
	expect(
		validateSchema(contract.queries.push?.argsSchema, {
			requestId: "r",
			chunk: { choices: [] },
		}).valid,
	).toBe(false);
}, 30000);

test("an unknown or misplaced @llm annotation fails extraction", () => {
	expect(() =>
		extractContract(
			widget(`import { defineWidget } from "@flow-like/widget-sdk";
/** @llm Completion */
interface Completion { text: string }
interface Queries { push: { args: { value: Completion }; returns: void } }
export default defineWidget<{}, {}, Queries>({ id: "bad-llm", name: "Bad" });`),
		),
	).toThrow("Invalid @llm 'Completion'");
	expect(() =>
		extractContract(
			widget(`import { defineWidget } from "@flow-like/widget-sdk";
interface Args {
	/** @llm Response */
	value: string;
}
interface Queries { push: { args: Args; returns: void } }
export default defineWidget<{}, {}, Queries>({ id: "bad-llm-type", name: "Bad" });`),
		),
	).toThrow("must annotate an object type");
}, 30000);
