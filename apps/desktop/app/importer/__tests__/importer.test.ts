import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { INode, IPin } from "@tm9657/flow-like-ui";
import { IPinType, IVariableType } from "@tm9657/flow-like-ui";
import {
	detectFormat,
	translateDify,
	translateN8n,
} from "@tm9657/flow-like-ui";
import type {
	DifyWorkflow,
	N8nManualMappingOverrides,
	N8nWorkflow,
	TranslationResult,
} from "@tm9657/flow-like-ui";
import {
	addExecPins,
	connectPins,
	createEmptyBoard,
	createNode,
	createPin,
	createVariable,
	findPinByName,
} from "@tm9657/flow-like-ui/lib/importer/board-builder";
import { describe, expect, it } from "vitest";

const FIXTURES = resolve(__dirname, "fixtures");

function loadFixture(name: string): string {
	return readFileSync(resolve(FIXTURES, name), "utf-8");
}

function decodePinDefault(pin: IPin): unknown {
	if (!pin.default_value) return undefined;
	const decoded = new TextDecoder().decode(new Uint8Array(pin.default_value));
	return JSON.parse(decoded);
}

// ---------------------------------------------------------------------------
// Format Detection
// ---------------------------------------------------------------------------
describe("detectFormat", () => {
	it("detects n8n JSON (AI chat workflow)", () => {
		const raw = loadFixture("n8n-ai-chat.json");
		const result = detectFormat(raw);
		expect(result.format).toBe("n8n");
		expect(result.parsed).not.toBeNull();
		expect((result.parsed as N8nWorkflow).name).toBe("Demo workflow");
	});

	it("detects n8n JSON (support pipeline)", () => {
		const raw = loadFixture("n8n-support-pipeline.json");
		const result = detectFormat(raw);
		expect(result.format).toBe("n8n");
		expect(result.parsed).not.toBeNull();
		expect((result.parsed as N8nWorkflow).nodes.length).toBe(9);
	});

	it("detects Dify JSON (FAQ bot)", () => {
		const raw = loadFixture("dify-faq-bot.json");
		const result = detectFormat(raw);
		expect(result.format).toBe("dify");
		expect(result.parsed).not.toBeNull();
		expect((result.parsed as DifyWorkflow).app.name).toBe("Customer FAQ Bot");
	});

	it("detects Dify JSON (data pipeline)", () => {
		const raw = loadFixture("dify-data-pipeline.json");
		const result = detectFormat(raw);
		expect(result.format).toBe("dify");
		expect(result.parsed).not.toBeNull();
	});

	it("returns unknown for invalid JSON", () => {
		const result = detectFormat("{not valid json!!!");
		expect(result.format).toBe("unknown");
		expect(result.parsed).toBeNull();
		expect(result.error).toBeDefined();
	});

	it("returns unknown for empty input", () => {
		const result = detectFormat("");
		expect(result.format).toBe("unknown");
		expect(result.parsed).toBeNull();
	});

	it("returns unknown for plain text", () => {
		const result = detectFormat("Hello world, this is just text.");
		expect(result.format).toBe("unknown");
		expect(result.parsed).toBeNull();
	});

	it("returns unknown for an unrecognized JSON structure", () => {
		const result = detectFormat(JSON.stringify({ foo: "bar", baz: 42 }));
		expect(result.format).toBe("unknown");
		expect(result.parsed).toBeNull();
	});

	it("returns unknown for JSON array input", () => {
		const result = detectFormat(
			JSON.stringify([{ nodes: [], connections: {} }]),
		);
		expect(result.format).toBe("unknown");
		expect(result.parsed).toBeNull();
	});

	it("returns unknown for YAML non-object input", () => {
		const result = detectFormat("- alpha\n- beta");
		expect(result.format).toBe("unknown");
		expect(result.parsed).toBeNull();
	});

	it("parses Dify YAML arrays and block scalars", () => {
		const raw = `kind: app
app:
  mode: workflow
workflow:
  graph:
    nodes:
      - id: node-1
        type: start
        data:
          title: Start
          desc: |
            first line
            second line
      - id: node-2
        type: end
        data:
          title: End
    edges:
      - source: node-1
        target: node-2
`;

		const result = detectFormat(raw);
		expect(result.format).toBe("dify");
		expect(result.parsed).not.toBeNull();

		const parsed = result.parsed as DifyWorkflow;
		const nodes = parsed.workflow.graph.nodes;
		expect(Array.isArray(nodes)).toBe(true);
		expect(nodes).toHaveLength(2);
		expect(nodes[0]?.id).toBe("node-1");
		expect(nodes[0]?.data?.desc).toBe("first line\nsecond line");
		expect(parsed.workflow.graph.edges).toHaveLength(1);
	});

	it("detects n8n with minimal structure (nodes + connections, no type prefix)", () => {
		const minimal = JSON.stringify({
			name: "Minimal",
			nodes: [
				{
					id: "1",
					name: "A",
					type: "custom.thing",
					position: [0, 0],
					parameters: {},
				},
			],
			connections: { A: { main: [[]] } },
		});
		const result = detectFormat(minimal);
		expect(result.format).toBe("n8n");
	});
});

// ---------------------------------------------------------------------------
// n8n Translation — Real AI Chat Workflow
// ---------------------------------------------------------------------------
describe("translateN8n — AI Chat (real n8n workflow)", () => {
	let result: TranslationResult;

	it("translates without throwing", () => {
		const raw = loadFixture("n8n-ai-chat.json");
		const parsed = JSON.parse(raw) as N8nWorkflow;
		result = translateN8n(parsed);
		expect(result).toBeDefined();
	});

	it("reports n8n format", () => {
		expect(result.format).toBe("n8n");
	});

	it("maps all 3 nodes", () => {
		expect(result.stats.totalNodes).toBe(3);
	});

	it("maps chatTrigger and ollamaModel, chainLlm becomes TODO without catalog", () => {
		// chatTrigger → events_simple (direct), lmChatOllama → model builder (direct)
		// chainLlm → TODO (agent composition requires catalog)
		expect(result.stats.directMapped).toBe(2);
		expect(result.stats.todo).toBe(1);
	});

	it("wires execution connections", () => {
		expect(result.stats.connections).toBeGreaterThanOrEqual(1);
	});

	it("creates credential variables for ollamaApi", () => {
		expect(result.stats.variables).toBeGreaterThanOrEqual(1);
		const vars = Object.values(result.board.variables);
		const ollamaCred = vars.find((v) => v.name.includes("ollamaApi"));
		expect(ollamaCred).toBeDefined();
		expect(ollamaCred!.secret).toBe(true);
	});

	it("board has valid structure", () => {
		const board = result.board;
		expect(board.name).toBe("Demo workflow");
		expect(board.id).toBeTruthy();
		expect(board.nodes).toBeDefined();
		// chatTrigger (1) + ollamaModel (1) + chainLlm TODO placeholder (1) + variable_get (1)
		expect(Object.keys(board.nodes).length).toBe(4);
		// Only chainLlm remains as a TODO layer
		expect(Object.keys(board.layers).length).toBe(1);
	});

	it("produces diagnostics", () => {
		expect(result.diagnostics.length).toBeGreaterThan(0);
		const infos = result.diagnostics.filter((d) => d.level === "info");
		expect(infos.length).toBeGreaterThan(0);
	});
});

// ---------------------------------------------------------------------------
// n8n Translation — Complex Support Pipeline
// ---------------------------------------------------------------------------
describe("translateN8n — Support Pipeline", () => {
	let result: TranslationResult;

	it("translates the full pipeline", () => {
		const raw = loadFixture("n8n-support-pipeline.json");
		const parsed = JSON.parse(raw) as N8nWorkflow;
		result = translateN8n(parsed);
		expect(result).toBeDefined();
		expect(result.format).toBe("n8n");
	});

	it("skips disabled nodes", () => {
		// 9 total nodes, 1 disabled → 8 active
		expect(result.stats.totalNodes).toBe(8);
	});

	it("has direct and composed mappings", () => {
		// webhook → events_generic (composition), set → struct_set (composition)
		// if → control_branch (direct), http → http_fetch (composition)
		// gmail → email_smtp_send (composition), code → python_interpreter (composition)
		// respondToWebhook → log_info (composition), slack → TODO
		expect(result.stats.directMapped).toBeGreaterThan(0);
		expect(result.stats.composed).toBeGreaterThan(0);
		expect(result.stats.todo).toBe(1);
	});

	it("slack node creates TODO layer (no native equivalent)", () => {
		const layers = Object.values(result.board.layers);
		const slackTodo = layers.find((l) => l.name.includes("Notify Slack"));
		expect(slackTodo).toBeDefined();
		const errors = result.diagnostics.filter(
			(d) => d.level === "error" && d.message.includes("slack"),
		);
		expect(errors.length).toBeGreaterThan(0);
	});

	it("wires multiple connection chains", () => {
		// Webhook→Set→If→(HTTP,Code), HTTP→Gmail→Slack, Code→Respond
		expect(result.stats.connections).toBeGreaterThanOrEqual(5);
	});

	it("extracts credentials as secret variables", () => {
		const vars = Object.values(result.board.variables);
		const secrets = vars.filter((v) => v.secret);
		// httpHeaderAuth, gmailOAuth2, slackApi = 3
		expect(secrets.length).toBeGreaterThanOrEqual(3);
	});

	it("creates timezone variable from settings", () => {
		const vars = Object.values(result.board.variables);
		const tzVar = vars.find((v) => v.name === "workflow_timezone");
		expect(tzVar).toBeDefined();
	});

	it("preserves node positions", () => {
		const nodes = Object.values(result.board.nodes);
		const webhookNode = nodes.find(
			(n) => n.friendly_name === "Webhook Receiver",
		);
		expect(webhookNode).toBeDefined();
		expect(webhookNode!.coordinates![0]).toBe(200);
		expect(webhookNode!.coordinates![1]).toBe(300);
	});

	it("HTTP node has request and response pins", () => {
		const nodes = Object.values(result.board.nodes);
		const httpNode = nodes.find(
			(n) => n.friendly_name === "Fetch Customer Data",
		);
		expect(httpNode).toBeDefined();
		const pins = Object.values(httpNode!.pins);
		const requestPin = pins.find((p) => p.name === "request");
		expect(requestPin).toBeDefined();
		const responsePin = pins.find((p) => p.name === "response");
		expect(responsePin).toBeDefined();
	});

	it("Set node has struct_set pins", () => {
		const nodes = Object.values(result.board.nodes);
		const setNode = nodes.find((n) => n.friendly_name === "Normalize Data");
		expect(setNode).toBeDefined();
		const pins = Object.values(setNode!.pins);
		const structIn = pins.find((p) => p.name === "struct_in");
		expect(structIn).toBeDefined();
		const fieldPin = pins.find((p) => p.name === "field");
		expect(fieldPin).toBeDefined();
		const structOut = pins.find((p) => p.name === "struct_out");
		expect(structOut).toBeDefined();
	});

	it("status is partial when there are TODO nodes", () => {
		expect(result.status).toBe("partial");
	});

	it("error diagnostics reference the unmapped slack node", () => {
		const errors = result.diagnostics.filter((d) => d.level === "error");
		const slackError = errors.find((d) => d.message.includes("slack"));
		expect(slackError).toBeDefined();
	});
});

// ---------------------------------------------------------------------------
// Dify Translation — FAQ Bot
// ---------------------------------------------------------------------------
describe("translateDify — FAQ Bot", () => {
	let result: TranslationResult;

	it("translates without throwing", () => {
		const raw = loadFixture("dify-faq-bot.json");
		const parsed = JSON.parse(raw) as DifyWorkflow;
		result = translateDify(parsed);
		expect(result).toBeDefined();
	});

	it("reports dify format", () => {
		expect(result.format).toBe("dify");
	});

	it("counts actionable nodes (excludes start/end)", () => {
		// 8 graph nodes total; start + end = 2 excluded → 6 actionable
		expect(result.stats.totalNodes).toBe(6);
	});

	it("maps known node types", () => {
		expect(result.stats.directMapped + result.stats.composed).toBeGreaterThan(
			0,
		);
	});

	it("creates input variables from start node", () => {
		const vars = Object.values(result.board.variables);
		const queryVar = vars.find((v) => v.name === "user_query");
		expect(queryVar).toBeDefined();
		expect(queryVar!.exposed).toBe(true);
		const langVar = vars.find((v) => v.name === "language");
		expect(langVar).toBeDefined();
	});

	it("creates output variables from end node", () => {
		const vars = Object.values(result.board.variables);
		const answerVar = vars.find((v) => v.name === "output_answer");
		expect(answerVar).toBeDefined();
		const escalatedVar = vars.find((v) => v.name === "output_escalated");
		expect(escalatedVar).toBeDefined();
	});

	it("imports environment variables", () => {
		const vars = Object.values(result.board.variables);
		const apiKey = vars.find((v) => v.name === "OPENAI_API_KEY");
		expect(apiKey).toBeDefined();
		expect(apiKey!.secret).toBe(true);
		const maxTokens = vars.find((v) => v.name === "MAX_TOKENS");
		expect(maxTokens).toBeDefined();
	});

	it("imports conversation variables", () => {
		const vars = Object.values(result.board.variables);
		const chatHistory = vars.find((v) => v.name === "conv_chat_history");
		expect(chatHistory).toBeDefined();
	});

	it("preserves viewport", () => {
		expect(result.board.viewport[0]).toBe(0);
		expect(result.board.viewport[1]).toBe(0);
		expect(result.board.viewport[2]).toBe(0.8);
	});

	it("wires edges including if-else branching", () => {
		expect(result.stats.connections).toBeGreaterThanOrEqual(4);
	});

	it("if-else node has true/false execution pins", () => {
		const nodes = Object.values(result.board.nodes);
		const ifNode = nodes.find(
			(n) => n.friendly_name === "Check Response Quality",
		);
		expect(ifNode).toBeDefined();
		const pins = Object.values(ifNode!.pins);
		const truePin = pins.find((p) => p.name === "exec_true");
		expect(truePin).toBeDefined();
		expect(truePin!.pin_type).toBe(IPinType.Output);
		const falsePin = pins.find((p) => p.name === "exec_false");
		expect(falsePin).toBeDefined();
	});

	it("if-else branching wires true→http and false→template", () => {
		const nodes = Object.values(result.board.nodes);
		const ifNode = nodes.find(
			(n) => n.friendly_name === "Check Response Quality",
		)!;
		const httpNode = nodes.find(
			(n) => n.friendly_name === "Escalate to Human",
		)!;
		const templateNode = nodes.find(
			(n) => n.friendly_name === "Format Success Response",
		)!;

		const truePin = Object.values(ifNode.pins).find(
			(p) => p.name === "exec_true",
		)!;
		const falsePin = Object.values(ifNode.pins).find(
			(p) => p.name === "exec_false",
		)!;
		const httpExecIn = Object.values(httpNode.pins).find(
			(p) => p.name === "exec_in",
		)!;
		const templateExecIn = Object.values(templateNode.pins).find(
			(p) => p.name === "exec_in",
		)!;

		// true → http (escalate)
		expect(truePin.connected_to).toContain(httpExecIn.id);
		// false → template (format success)
		expect(falsePin.connected_to).toContain(templateExecIn.id);
	});

	it("LLM node has model and prompt pins", () => {
		const nodes = Object.values(result.board.nodes);
		const llmNode = nodes.find((n) => n.friendly_name === "Generate Answer");
		expect(llmNode).toBeDefined();
		const pins = Object.values(llmNode!.pins);
		const modelPin = pins.find((p) => p.name === "model_name");
		expect(modelPin).toBeDefined();
		const systemPrompt = pins.find((p) => p.name === "prompt_system");
		expect(systemPrompt).toBeDefined();
		const userPrompt = pins.find((p) => p.name === "prompt_user");
		expect(userPrompt).toBeDefined();
	});

	it("knowledge-retrieval node has query and results pins", () => {
		const nodes = Object.values(result.board.nodes);
		const kbNode = nodes.find(
			(n) => n.friendly_name === "Search Knowledge Base",
		);
		expect(kbNode).toBeDefined();
		const pins = Object.values(kbNode!.pins);
		expect(pins.find((p) => p.name === "search_query")).toBeDefined();
		expect(pins.find((p) => p.name === "search_results")).toBeDefined();
	});
});

// ---------------------------------------------------------------------------
// Dify Translation — Data Pipeline (with unknown node, iteration, agent)
// ---------------------------------------------------------------------------
describe("translateDify — Data Pipeline", () => {
	let result: TranslationResult;

	it("translates the pipeline", () => {
		const raw = loadFixture("dify-data-pipeline.json");
		const parsed = JSON.parse(raw) as DifyWorkflow;
		result = translateDify(parsed);
		expect(result).toBeDefined();
	});

	it("counts nodes correctly (excludes start/end)", () => {
		// 10 nodes total, start + end = 2 → 8 actionable
		expect(result.stats.totalNodes).toBe(8);
	});

	it("handles iteration node with array pins", () => {
		const nodes = Object.values(result.board.nodes);
		const iterNode = nodes.find(
			(n) => n.friendly_name === "Process Each Document",
		);
		expect(iterNode).toBeDefined();
		const pins = Object.values(iterNode!.pins);
		const inputArray = pins.find((p) => p.name === "input_array");
		expect(inputArray).toBeDefined();
		const currentItem = pins.find((p) => p.name === "current_item");
		expect(currentItem).toBeDefined();
		const currentIndex = pins.find((p) => p.name === "current_index");
		expect(currentIndex).toBeDefined();
		expect(currentIndex!.data_type).toBe(IVariableType.Integer);
	});

	it("creates TODO layer for unknown custom-plugin-node", () => {
		expect(result.stats.todo).toBe(1);
		const layers = Object.values(result.board.layers);
		const todoLayer = layers.find((l) => l.name.includes("My Custom Plugin"));
		expect(todoLayer).toBeDefined();
	});

	it("status is partial with unknown nodes", () => {
		expect(result.status).toBe("partial");
	});

	it("wires edges between known nodes", () => {
		expect(result.stats.connections).toBeGreaterThan(0);
	});

	it("imports environment variables", () => {
		const vars = Object.values(result.board.variables);
		const dbUrl = vars.find((v) => v.name === "DATABASE_URL");
		expect(dbUrl).toBeDefined();
		expect(dbUrl!.secret).toBe(true);
	});

	it("imports start node input variables", () => {
		const vars = Object.values(result.board.variables);
		const docsVar = vars.find((v) => v.name === "documents");
		expect(docsVar).toBeDefined();
	});

	it("imports end node output variables", () => {
		const vars = Object.values(result.board.variables);
		const countVar = vars.find((v) => v.name === "output_processed_count");
		expect(countVar).toBeDefined();
	});

	it("preserves custom viewport", () => {
		expect(result.board.viewport[0]).toBe(-50);
		expect(result.board.viewport[1]).toBe(100);
		expect(result.board.viewport[2]).toBe(1.2);
	});

	it("agent node maps to simple_agent", () => {
		const nodes = Object.values(result.board.nodes);
		const agentNode = nodes.find(
			(n) => n.friendly_name === "Quality Check Agent",
		);
		expect(agentNode).toBeDefined();
		expect(agentNode!.name).toBe("simple_agent");
	});

	it("doc-extractor maps to markitdown_convert", () => {
		const nodes = Object.values(result.board.nodes);
		const docNode = nodes.find((n) => n.friendly_name === "Extract Content");
		expect(docNode).toBeDefined();
		expect(docNode!.name).toBe("markitdown_convert");
	});

	it("deduplicates prompt pin names for repeated roles", () => {
		const workflow = {
			app: {
				name: "Prompt Role Duplicates",
				mode: "workflow",
			},
			workflow: {
				graph: {
					nodes: [
						{
							id: "start",
							data: {
								title: "Start",
								variables: [],
							},
							type: "start",
							x: 0,
							y: 0,
						},
						{
							id: "llm",
							data: {
								model: { name: "gpt-4.1", provider: "openai" },
								prompt_template: [
									{ role: "user", text: "First" },
									{ role: "user", text: "Second" },
								],
								title: "LLM",
							},
							type: "llm",
							x: 100,
							y: 0,
						},
						{
							id: "end",
							data: {
								outputs: [],
								title: "End",
							},
							type: "end",
							x: 200,
							y: 0,
						},
					],
					edges: [
						{ id: "edge-1", source: "start", target: "llm" },
						{ id: "edge-2", source: "llm", target: "end" },
					],
				},
			},
		} as unknown as DifyWorkflow;

		const translation = translateDify(workflow);
		const llmNode = Object.values(translation.board.nodes).find(
			(node) => node.friendly_name === "LLM",
		);
		expect(llmNode).toBeDefined();

		const pins = Object.values(llmNode!.pins);
		expect(pins.find((pin) => pin.name === "prompt_user")).toBeDefined();
		expect(pins.find((pin) => pin.name === "prompt_user_2")).toBeDefined();
	});

	it("code node maps to dify_code", () => {
		const nodes = Object.values(result.board.nodes);
		const codeNode = nodes.find((n) => n.friendly_name === "Transform Data");
		expect(codeNode).toBeDefined();
		expect(codeNode!.name).toBe("dify_code");
	});
});

// ---------------------------------------------------------------------------
// Board Builder Utilities
// ---------------------------------------------------------------------------
describe("board-builder utilities", () => {
	it("createEmptyBoard returns valid structure", () => {
		const board = createEmptyBoard("Test", "A test board");
		expect(board.id).toBeTruthy();
		expect(board.name).toBe("Test");
		expect(board.description).toBe("A test board");
		expect(board.nodes).toEqual({});
		expect(board.variables).toEqual({});
		expect(board.comments).toEqual({});
		expect(board.layers).toEqual({});
		expect(board.viewport).toEqual([0, 0, 1]);
	});

	it("createNode produces correct structure", () => {
		const node = createNode({
			name: "http_fetch",
			friendlyName: "My HTTP",
			description: "Fetch data",
			category: "Web/HTTP",
			x: 100,
			y: 200,
		});
		expect(node.id).toBeTruthy();
		expect(node.name).toBe("http_fetch");
		expect(node.friendly_name).toBe("My HTTP");
		expect(node.coordinates).toEqual([100, 200, 0]);
		expect(node.pins).toEqual({});
	});

	it("createPin generates unique ids", () => {
		const p1 = createPin({
			name: "a",
			friendlyName: "A",
			description: "",
			pinType: IPinType.Input,
			dataType: IVariableType.String,
		});
		const p2 = createPin({
			name: "b",
			friendlyName: "B",
			description: "",
			pinType: IPinType.Output,
			dataType: IVariableType.Integer,
		});
		expect(p1.id).not.toBe(p2.id);
		expect(p1.pin_type).toBe(IPinType.Input);
		expect(p2.pin_type).toBe(IPinType.Output);
		expect(p2.data_type).toBe(IVariableType.Integer);
	});

	it("createPin encodes default value", () => {
		const pin = createPin({
			name: "val",
			friendlyName: "Value",
			description: "",
			pinType: IPinType.Input,
			dataType: IVariableType.String,
			defaultValue: "hello",
		});
		expect(pin.default_value).not.toBeNull();
		const decoded = new TextDecoder().decode(
			new Uint8Array(pin.default_value!),
		);
		expect(JSON.parse(decoded)).toBe("hello");
	});

	it("addExecPins adds in/out execution pins to node", () => {
		const node = createNode({
			name: "test",
			friendlyName: "Test",
			description: "",
			category: "Test",
			x: 0,
			y: 0,
		});
		const { inPin, outPin } = addExecPins(node);
		expect(inPin.name).toBe("exec_in");
		expect(outPin.name).toBe("exec_out");
		expect(inPin.data_type).toBe(IVariableType.Execution);
		expect(Object.keys(node.pins).length).toBe(2);
	});

	it("connectPins wires connected_to and depends_on", () => {
		const n1 = createNode({
			name: "a",
			friendlyName: "A",
			description: "",
			category: "",
			x: 0,
			y: 0,
		});
		const n2 = createNode({
			name: "b",
			friendlyName: "B",
			description: "",
			category: "",
			x: 0,
			y: 0,
		});
		const { outPin } = addExecPins(n1);
		const { inPin } = addExecPins(n2);
		connectPins(n1, outPin, n2, inPin);
		expect(outPin.connected_to).toContain(inPin.id);
		expect(inPin.depends_on).toContain(outPin.id);
	});

	it("findPinByName returns correct pin", () => {
		const node = createNode({
			name: "x",
			friendlyName: "X",
			description: "",
			category: "",
			x: 0,
			y: 0,
		});
		addExecPins(node);
		const found = findPinByName(node, "exec_in", IPinType.Input);
		expect(found).toBeDefined();
		expect(found!.name).toBe("exec_in");
		const notFound = findPinByName(node, "nonexistent", IPinType.Input);
		expect(notFound).toBeUndefined();
	});

	it("createVariable sets secret and exposed flags", () => {
		const v = createVariable({
			name: "api_key",
			dataType: IVariableType.String,
			secret: true,
			exposed: true,
		});
		expect(v.secret).toBe(true);
		expect(v.exposed).toBe(true);
		expect(v.name).toBe("api_key");
	});
});

// ---------------------------------------------------------------------------
// End-to-end: detectFormat → translate pipeline
// ---------------------------------------------------------------------------
describe("end-to-end pipeline", () => {
	it("detects and translates n8n AI chat workflow", () => {
		const raw = loadFixture("n8n-ai-chat.json");
		const detection = detectFormat(raw);
		expect(detection.format).toBe("n8n");

		const result = translateN8n(detection.parsed as N8nWorkflow);
		// partial: chatTrigger and ollamaModel are mapped, chainLlm is TODO
		expect(result.status).toBe("partial");
		expect(result.stats.totalNodes).toBe(3);
	});

	it("n8n AI chat with catalog: model connects to agent_from_model", () => {
		const raw = loadFixture("n8n-ai-chat.json");
		const parsed = JSON.parse(raw) as N8nWorkflow;
		const result = translateN8n(parsed, buildMockCatalog());

		// With catalog: chatTrigger (direct) + chainLlm (agent composition) + ollamaModel (direct→moved into layer)
		expect(result.stats.directMapped).toBe(2);
		expect(result.stats.composed).toBe(1);
		expect(result.stats.todo).toBe(0);

		const nodes = Object.values(result.board.nodes);
		const ollamaNode = nodes.find(
			(n) => n.name === "ai_generative_build_ollama",
		);
		const fromModel = nodes.find((n) => n.name === "agent_from_model");

		expect(ollamaNode).toBeDefined();
		expect(fromModel).toBeDefined();

		// Model builder placed in same layer as agent_from_model
		expect(ollamaNode!.layer).toBe(fromModel!.layer);

		// Direct connection: ollama model output → agent_from_model model input
		const modelOut = Object.values(ollamaNode!.pins).find(
			(p) => p.name === "model" && p.pin_type === IPinType.Output,
		);
		const modelIn = Object.values(fromModel!.pins).find(
			(p) => p.name === "model" && p.pin_type === IPinType.Input,
		);
		expect(modelOut).toBeDefined();
		expect(modelIn).toBeDefined();
		expect(modelOut!.connected_to).toContain(modelIn!.id);
	});

	it("detects and translates n8n support pipeline", () => {
		const raw = loadFixture("n8n-support-pipeline.json");
		const detection = detectFormat(raw);
		expect(detection.format).toBe("n8n");

		const result = translateN8n(detection.parsed as N8nWorkflow);
		// partial because slack is unmapped
		expect(result.status).toBe("partial");
		expect(result.stats.totalNodes).toBe(8);
	});

	it("detects and translates Dify FAQ bot", () => {
		const raw = loadFixture("dify-faq-bot.json");
		const detection = detectFormat(raw);
		expect(detection.format).toBe("dify");

		const result = translateDify(detection.parsed as DifyWorkflow);
		expect(result.stats.totalNodes).toBe(6);
		expect(result.stats.variables).toBeGreaterThan(0);
	});

	it("detects and translates Dify data pipeline", () => {
		const raw = loadFixture("dify-data-pipeline.json");
		const detection = detectFormat(raw);
		expect(detection.format).toBe("dify");

		const result = translateDify(detection.parsed as DifyWorkflow);
		expect(result.stats.totalNodes).toBe(8);
		expect(result.stats.todo).toBe(1);
	});

	it("all translated boards have unique node ids", () => {
		for (const fixture of ["n8n-ai-chat.json", "n8n-support-pipeline.json"]) {
			const parsed = JSON.parse(loadFixture(fixture)) as N8nWorkflow;
			const result = translateN8n(parsed);
			const ids = Object.keys(result.board.nodes);
			expect(new Set(ids).size).toBe(ids.length);
		}

		for (const fixture of ["dify-faq-bot.json", "dify-data-pipeline.json"]) {
			const parsed = JSON.parse(loadFixture(fixture)) as DifyWorkflow;
			const result = translateDify(parsed);
			const ids = Object.keys(result.board.nodes);
			expect(new Set(ids).size).toBe(ids.length);
		}
	});

	it("all translated boards have unique pin ids within each node", () => {
		const raw = loadFixture("n8n-support-pipeline.json");
		const parsed = JSON.parse(raw) as N8nWorkflow;
		const result = translateN8n(parsed);

		for (const node of Object.values(result.board.nodes)) {
			const pinIds = Object.keys(node.pins);
			expect(new Set(pinIds).size).toBe(pinIds.length);
		}
	});

	it("all impure nodes have at least one execution pin", () => {
		const raw = loadFixture("n8n-support-pipeline.json");
		const parsed = JSON.parse(raw) as N8nWorkflow;
		const result = translateN8n(parsed);

		const pureNodes = new Set([
			"http_make_request",
			"data_google_provider",
			"struct_get",
			"variable_get",
		]);
		for (const node of Object.values(result.board.nodes)) {
			if (pureNodes.has(node.name)) continue;
			const execPins = Object.values(node.pins).filter(
				(p) => p.data_type === "Execution",
			);
			expect(execPins.length).toBeGreaterThan(0);
		}
	});
});

// ---------------------------------------------------------------------------
// Catalog-driven translation
// ---------------------------------------------------------------------------

function makeCatalogNode(
	name: string,
	pins: Parameters<typeof createPin>[0][],
): INode {
	const node = createNode({
		name,
		friendlyName: name,
		description: `Catalog: ${name}`,
		category: "Catalog",
		x: 0,
		y: 0,
	});
	for (const pinDef of pins) {
		const pin = createPin(pinDef);
		node.pins[pin.id] = pin;
	}
	return node;
}

function buildMockCatalog(): INode[] {
	return [
		// http_fetch – impure, needs exec + request/response
		makeCatalogNode("http_fetch", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_error",
				friendlyName: "Error",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "request",
				friendlyName: "Request",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "response",
				friendlyName: "Response",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// http_make_request – pure companion
		makeCatalogNode("http_make_request", [
			{
				name: "method",
				friendlyName: "Method",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
				defaultValue: "GET",
			},
			{
				name: "url",
				friendlyName: "URL",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "request",
				friendlyName: "Request",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// http_response_to_json – impure, parses JSON body
		makeCatalogNode("http_response_to_json", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "response",
				friendlyName: "Response",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "struct",
				friendlyName: "Struct",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// struct_get – pure, access fields with dot notation
		makeCatalogNode("struct_get", [
			{
				name: "struct",
				friendlyName: "Struct",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "field",
				friendlyName: "Field",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "value",
				friendlyName: "Value",
				pinType: IPinType.Output,
				dataType: IVariableType.Generic,
			},
			{
				name: "found",
				friendlyName: "Found?",
				pinType: IPinType.Output,
				dataType: IVariableType.Boolean,
			},
		]),
		// control_branch
		makeCatalogNode("control_branch", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "condition",
				friendlyName: "Condition",
				pinType: IPinType.Input,
				dataType: IVariableType.Boolean,
			},
			{
				name: "true",
				friendlyName: "True",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "false",
				friendlyName: "False",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
		]),
		// events_simple
		makeCatalogNode("events_simple", [
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
		]),
		// control_sequence
		makeCatalogNode("control_sequence", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
		]),
		// email_smtp_connect
		makeCatalogNode("email_smtp_connect", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "host",
				friendlyName: "Host",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
				defaultValue: "smtp.example.com",
			},
			{
				name: "port",
				friendlyName: "Port",
				pinType: IPinType.Input,
				dataType: IVariableType.Integer,
				defaultValue: 587,
			},
			{
				name: "username",
				friendlyName: "Username",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "password",
				friendlyName: "Password",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "encryption",
				friendlyName: "Encryption",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
				defaultValue: "StartTls",
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "connection",
				friendlyName: "Connection",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// email_smtp_send
		makeCatalogNode("email_smtp_send", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "connection",
				friendlyName: "Connection",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "from",
				friendlyName: "From",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "to",
				friendlyName: "To",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "subject",
				friendlyName: "Subject",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "body_text",
				friendlyName: "Body (text)",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "body_html",
				friendlyName: "Body (HTML)",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "message_id",
				friendlyName: "Message ID",
				pinType: IPinType.Output,
				dataType: IVariableType.String,
			},
		]),
		// log_info (for Set node mapping)
		makeCatalogNode("log_info", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "message",
				friendlyName: "Message",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "toast",
				friendlyName: "Toast",
				pinType: IPinType.Input,
				dataType: IVariableType.Boolean,
			},
		]),
		// control_delay → actually named "delay" in catalog
		makeCatalogNode("delay", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "time",
				friendlyName: "Time",
				pinType: IPinType.Input,
				dataType: IVariableType.Float,
			},
		]),
		// control_for_each
		makeCatalogNode("control_for_each", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "loop_body",
				friendlyName: "Loop",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "items",
				friendlyName: "Items",
				pinType: IPinType.Input,
				dataType: IVariableType.Generic,
			},
			{
				name: "element",
				friendlyName: "Element",
				pinType: IPinType.Output,
				dataType: IVariableType.Generic,
			},
			{
				name: "index",
				friendlyName: "Index",
				pinType: IPinType.Output,
				dataType: IVariableType.Integer,
			},
		]),
		// agent_invoke – impure, invokes an agent
		makeCatalogNode("agent_invoke", [
			{
				name: "exec_in",
				friendlyName: "▶",
				pinType: IPinType.Input,
				dataType: IVariableType.Execution,
			},
			{
				name: "exec_out",
				friendlyName: "▶",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			},
			{
				name: "agent",
				friendlyName: "Agent",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "history",
				friendlyName: "History",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "response",
				friendlyName: "Response",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// agent_from_model – creates an agent from an LLM model
		makeCatalogNode("agent_from_model", [
			{
				name: "model",
				friendlyName: "Model",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "agent_out",
				friendlyName: "Agent",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// agent_set_system_prompt – sets system prompt on an agent
		makeCatalogNode("agent_set_system_prompt", [
			{
				name: "agent_in",
				friendlyName: "Agent In",
				pinType: IPinType.Input,
				dataType: IVariableType.Struct,
			},
			{
				name: "system_prompt",
				friendlyName: "System Prompt",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "agent_out",
				friendlyName: "Agent Out",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// ai_generative_build_gemini – builds a Gemini LLM model
		makeCatalogNode("ai_generative_build_gemini", [
			{
				name: "endpoint",
				friendlyName: "Endpoint",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "api_key",
				friendlyName: "API Key",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "model_id",
				friendlyName: "Model ID",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "model",
				friendlyName: "Model",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// ai_generative_build_ollama – builds an Ollama LLM model
		makeCatalogNode("ai_generative_build_ollama", [
			{
				name: "endpoint",
				friendlyName: "Endpoint",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "model_id",
				friendlyName: "Model ID",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "model",
				friendlyName: "Model",
				pinType: IPinType.Output,
				dataType: IVariableType.Struct,
			},
		]),
		// variable_get – reads a variable value
		makeCatalogNode("variable_get", [
			{
				name: "var_ref",
				friendlyName: "Variable Reference",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			},
			{
				name: "value_ref",
				friendlyName: "Value",
				pinType: IPinType.Output,
				dataType: IVariableType.Generic,
			},
		]),
	];
}

describe("catalog-driven translation", () => {
	const mockCatalog = buildMockCatalog();

	it("uses catalog pins instead of manually-created pins", () => {
		const workflow: N8nWorkflow = {
			name: "Catalog test",
			nodes: [
				{
					id: "n1",
					name: "HTTP Request",
					type: "n8n-nodes-base.httpRequest",
					position: [100, 200],
					parameters: { url: "https://example.com", method: "POST" },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog);
		expect(result.status).not.toBe("error");

		const httpFetch = Object.values(result.board.nodes).find(
			(n) => n.name === "http_fetch",
		);
		expect(httpFetch).toBeDefined();

		// The pins should come from the catalog template
		const pinNames = Object.values(httpFetch!.pins).map((p) => p.name);
		expect(pinNames).toContain("request");
		expect(pinNames).toContain("response");
		expect(pinNames).toContain("exec_in");
		expect(pinNames).toContain("exec_out");
		expect(pinNames).toContain("exec_error");
	});

	it("composes companion nodes from catalog", () => {
		const workflow: N8nWorkflow = {
			name: "Companion test",
			nodes: [
				{
					id: "n1",
					name: "HTTP Request",
					type: "n8n-nodes-base.httpRequest",
					position: [400, 200],
					parameters: { url: "https://api.test.com", method: "PUT" },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog);
		const nodes = Object.values(result.board.nodes);

		const makeReq = nodes.find((n) => n.name === "http_make_request");
		expect(makeReq).toBeDefined();

		// Companion should have catalog pins
		const companionPinNames = Object.values(makeReq!.pins).map((p) => p.name);
		expect(companionPinNames).toContain("method");
		expect(companionPinNames).toContain("url");
		expect(companionPinNames).toContain("request");

		// Downstream: to_json and get_field should also be composed
		const toJson = nodes.find((n) => n.name === "http_response_to_json");
		expect(toJson).toBeDefined();
		const getField = nodes.find((n) => n.name === "struct_get");
		expect(getField).toBeDefined();
	});

	it("companion output is wired to main node input", () => {
		const workflow: N8nWorkflow = {
			name: "Wiring test",
			nodes: [
				{
					id: "n1",
					name: "HTTP Request",
					type: "n8n-nodes-base.httpRequest",
					position: [400, 200],
					parameters: { url: "https://api.test.com" },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog);
		const nodes = Object.values(result.board.nodes);

		const httpFetch = nodes.find((n) => n.name === "http_fetch")!;
		const makeReq = nodes.find((n) => n.name === "http_make_request")!;

		// The make_request "request" output should connect to http_fetch "request" input
		const makeReqOutputPin = Object.values(makeReq.pins).find(
			(p) => p.name === "request" && p.pin_type === IPinType.Output,
		);
		const httpFetchInputPin = Object.values(httpFetch.pins).find(
			(p) => p.name === "request" && p.pin_type === IPinType.Input,
		);

		expect(makeReqOutputPin).toBeDefined();
		expect(httpFetchInputPin).toBeDefined();
		expect(makeReqOutputPin!.connected_to).toContain(httpFetchInputPin!.id);
		expect(httpFetchInputPin!.depends_on).toContain(makeReqOutputPin!.id);
	});

	it("sets default values from n8n parameters on catalog pins", () => {
		const workflow: N8nWorkflow = {
			name: "Defaults test",
			nodes: [
				{
					id: "n1",
					name: "Wait",
					type: "n8n-nodes-base.wait",
					position: [100, 100],
					parameters: { amount: 5 },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog);
		const delayNode = Object.values(result.board.nodes).find(
			(n) => n.name === "delay",
		);
		expect(delayNode).toBeDefined();

		const timePin = Object.values(delayNode!.pins).find(
			(p) => p.name === "time" && p.pin_type === IPinType.Input,
		);
		expect(timePin).toBeDefined();
		expect(decodePinDefault(timePin!)).toBe(5000);
	});

	it("lets manual mapping overrides replace a built-in mapping with a layer", () => {
		const overrides: N8nManualMappingOverrides = {
			"n8n-nodes-base.wait": {
				flow: {
					mode: "layer",
					skipExecPins: true,
					nodes: [
						{ id: "entry", catalog: "control_sequence", primary: true },
						{
							id: "log",
							catalog: "log_info",
							offset: [300, 0],
							nameSuffix: "(Mapped)",
						},
					],
					connections: [["entry:exec_out", "log:exec_in"]],
					defaults: {
						"log:message": "$time",
						"log:toast": true,
					},
				},
			},
		};

		const workflow: N8nWorkflow = {
			name: "Override wait mapping",
			nodes: [
				{
					id: "wait1",
					name: "Wait",
					type: "n8n-nodes-base.wait",
					position: [100, 100],
					parameters: { amount: 7 },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog, {
			mappingOverrides: overrides,
		});
		expect(result.status).not.toBe("error");
		expect(result.stats.composed).toBe(1);
		expect(result.stats.todo).toBe(0);

		const nodes = Object.values(result.board.nodes);
		expect(nodes.find((node) => node.name === "delay")).toBeUndefined();

		const primaryNode = nodes.find((node) => node.name === "control_sequence");
		const logNode = nodes.find((node) => node.name === "log_info");
		expect(primaryNode).toBeDefined();
		expect(logNode).toBeDefined();
		expect(Object.keys(result.board.layers)).toHaveLength(1);

		const messagePin = Object.values(logNode!.pins).find(
			(pin) => pin.name === "message" && pin.pin_type === IPinType.Input,
		);
		expect(messagePin).toBeDefined();
		expect(decodePinDefault(messagePin!)).toBe(7);
	});

	it("default Gmail override seeds Gmail SMTP defaults", () => {
		const workflow: N8nWorkflow = {
			name: "Gmail override",
			nodes: [
				{
					id: "gmail1",
					name: "Send Email Alert",
					type: "n8n-nodes-base.gmail",
					position: [100, 100],
					parameters: {
						sendTo: "support@example.com",
						subject: "Alert",
						message: "Attention required",
					},
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog);
		const smtpConnect = Object.values(result.board.nodes).find(
			(node) => node.name === "email_smtp_connect",
		);
		const smtpSend = Object.values(result.board.nodes).find(
			(node) => node.name === "email_smtp_send",
		);

		expect(smtpConnect).toBeDefined();
		expect(smtpSend).toBeDefined();
		expect(
			decodePinDefault(findPinByName(smtpConnect!, "host", IPinType.Input)!),
		).toBe("smtp.gmail.com");
		expect(
			decodePinDefault(findPinByName(smtpConnect!, "port", IPinType.Input)!),
		).toBe(587);
		expect(
			decodePinDefault(
				findPinByName(smtpConnect!, "encryption", IPinType.Input)!,
			),
		).toBe("StartTls");
		expect(
			decodePinDefault(findPinByName(smtpSend!, "to", IPinType.Input)!),
		).toBe("support@example.com");
	});

	it("default respondToWebhook override keeps the configured response body in fallback mode", () => {
		const workflow: N8nWorkflow = {
			name: "Respond override",
			nodes: [
				{
					id: "respond1",
					name: "Send Response",
					type: "n8n-nodes-base.respondToWebhook",
					position: [100, 100],
					parameters: {
						respondWith: "json",
						responseBody: "={{ $json }}",
					},
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow);
		const responseNode = Object.values(result.board.nodes).find(
			(node) => node.name === "log_info",
		);
		expect(responseNode).toBeDefined();
		expect(
			decodePinDefault(
				findPinByName(responseNode!, "message", IPinType.Input)!,
			),
		).toBe("={{ $json }}");
	});

	it("all catalog-placed nodes have unique pin IDs (cloned, not shared)", () => {
		const workflow: N8nWorkflow = {
			name: "Unique IDs test",
			nodes: [
				{
					id: "n1",
					name: "HTTP 1",
					type: "n8n-nodes-base.httpRequest",
					position: [100, 100],
					parameters: { url: "https://a.com" },
					typeVersion: 1,
				},
				{
					id: "n2",
					name: "HTTP 2",
					type: "n8n-nodes-base.httpRequest",
					position: [500, 100],
					parameters: { url: "https://b.com" },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog);
		const allPinIds = Object.values(result.board.nodes).flatMap((n) =>
			Object.keys(n.pins),
		);
		expect(new Set(allPinIds).size).toBe(allPinIds.length);
	});

	it("falls back to manual pins when catalog entry is missing", () => {
		// Provide catalog without http_fetch → should still work via fallback
		const partialCatalog = mockCatalog.filter((n) => n.name !== "http_fetch");

		const workflow: N8nWorkflow = {
			name: "Fallback test",
			nodes: [
				{
					id: "n1",
					name: "HTTP Request",
					type: "n8n-nodes-base.httpRequest",
					position: [100, 100],
					parameters: { url: "https://fallback.com" },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, partialCatalog);
		expect(result.status).not.toBe("error");
		const httpNode = Object.values(result.board.nodes).find(
			(n) => n.name === "http_fetch",
		);
		expect(httpNode).toBeDefined();
	});

	it("catalog path works for multi-node support pipeline fixture", () => {
		const raw = loadFixture("n8n-support-pipeline.json");
		const parsed = JSON.parse(raw) as N8nWorkflow;
		const result = translateN8n(parsed, mockCatalog);

		expect(result.status).not.toBe("error");
		expect(result.stats.totalNodes).toBeGreaterThan(0);

		// All nodes should have unique IDs
		const nodeIds = Object.keys(result.board.nodes);
		expect(new Set(nodeIds).size).toBe(nodeIds.length);

		// All pin IDs should be unique across the board
		const allPinIds = Object.values(result.board.nodes).flatMap((n) =>
			Object.keys(n.pins),
		);
		expect(new Set(allPinIds).size).toBe(allPinIds.length);
	});

	it("HTTP pipeline: make_request → fetch → to_json → get_field", () => {
		const workflow: N8nWorkflow = {
			name: "HTTP pipeline test",
			nodes: [
				{
					id: "n1",
					name: "API Call",
					type: "n8n-nodes-base.httpRequest",
					position: [400, 200],
					parameters: { url: "https://api.example.com/users", method: "GET" },
					typeVersion: 1,
				},
			],
			connections: {},
		};

		const result = translateN8n(workflow, mockCatalog);
		const nodes = Object.values(result.board.nodes);

		const makeReq = nodes.find((n) => n.name === "http_make_request")!;
		const fetch = nodes.find((n) => n.name === "http_fetch")!;
		const toJson = nodes.find((n) => n.name === "http_response_to_json")!;
		const getField = nodes.find((n) => n.name === "struct_get")!;

		expect(makeReq).toBeDefined();
		expect(fetch).toBeDefined();
		expect(toJson).toBeDefined();
		expect(getField).toBeDefined();

		// make_request.request → fetch.request
		const makeReqOut = Object.values(makeReq.pins).find(
			(p) => p.name === "request" && p.pin_type === IPinType.Output,
		)!;
		const fetchReqIn = Object.values(fetch.pins).find(
			(p) => p.name === "request" && p.pin_type === IPinType.Input,
		)!;
		expect(makeReqOut.connected_to).toContain(fetchReqIn.id);

		// fetch.response → to_json.response
		const fetchRespOut = Object.values(fetch.pins).find(
			(p) => p.name === "response" && p.pin_type === IPinType.Output,
		)!;
		const toJsonRespIn = Object.values(toJson.pins).find(
			(p) => p.name === "response" && p.pin_type === IPinType.Input,
		)!;
		expect(fetchRespOut.connected_to).toContain(toJsonRespIn.id);

		// fetch.exec_out → to_json.exec_in
		const fetchExecOut = Object.values(fetch.pins).find(
			(p) => p.name === "exec_out" && p.pin_type === IPinType.Output,
		)!;
		const toJsonExecIn = Object.values(toJson.pins).find(
			(p) => p.name === "exec_in" && p.pin_type === IPinType.Input,
		)!;
		expect(fetchExecOut.connected_to).toContain(toJsonExecIn.id);

		// to_json.struct → get_field.struct
		const toJsonStructOut = Object.values(toJson.pins).find(
			(p) => p.name === "struct" && p.pin_type === IPinType.Output,
		)!;
		const getFieldStructIn = Object.values(getField.pins).find(
			(p) => p.name === "struct" && p.pin_type === IPinType.Input,
		)!;
		expect(toJsonStructOut.connected_to).toContain(getFieldStructIn.id);

		// get_field should have "data" as default field value
		const fieldPin = Object.values(getField.pins).find(
			(p) => p.name === "field" && p.pin_type === IPinType.Input,
		)!;
		expect(decodePinDefault(fieldPin)).toBe("data");
	});

	it("composition nodes are grouped into a named layer", () => {
		const workflow: N8nWorkflow = {
			name: "Layer test",
			nodes: [
				{
					id: "http1",
					name: "HTTP Request",
					type: "n8n-nodes-base.httpRequest",
					position: [400, 200],
					typeVersion: 1,
					parameters: {
						url: "https://example.com",
						method: "GET",
						responseFormat: "json",
					},
				},
			],
			connections: {},
		};
		const result = translateN8n(workflow, mockCatalog);
		const layers = Object.values(result.board.layers);
		const httpLayer = layers.find((l) => l.name === "HTTP Request");
		expect(httpLayer).toBeDefined();

		const nodesInLayer = Object.values(result.board.nodes).filter(
			(n) => n.layer === httpLayer!.id,
		);
		expect(nodesInLayer.length).toBeGreaterThanOrEqual(2);
		const nodeNames = nodesInLayer.map((n) => n.name);
		expect(nodeNames).toContain("http_fetch");
		expect(nodeNames).toContain("http_make_request");
	});

	it("credentials create variable_get nodes with var_ref pin", () => {
		const workflow: N8nWorkflow = {
			name: "Cred test",
			nodes: [
				{
					id: "http1",
					name: "My HTTP",
					type: "n8n-nodes-base.httpRequest",
					position: [400, 200],
					typeVersion: 1,
					parameters: { url: "https://example.com", method: "GET" },
					credentials: {
						httpHeaderAuth: { id: "cred1", name: "My API Key" },
					},
				},
			],
			connections: {},
		};
		const result = translateN8n(workflow, mockCatalog);
		const vars = Object.values(result.board.variables);
		const credVar = vars.find((v) => v.name === "credential_httpHeaderAuth");
		expect(credVar).toBeDefined();
		expect(credVar!.secret).toBe(true);

		const getVarNodes = Object.values(result.board.nodes).filter(
			(n) => n.name === "variable_get",
		);
		expect(getVarNodes.length).toBe(1);
		expect(getVarNodes[0].friendly_name).toBe("Get My API Key");

		const varRefPin = Object.values(getVarNodes[0].pins).find(
			(p) => p.name === "var_ref",
		);
		expect(varRefPin).toBeDefined();
		expect(decodePinDefault(varRefPin!)).toBe(credVar!.id);
	});

	it("AI agent: model builder connects to agent_from_model via ai_languageModel", () => {
		const workflow: N8nWorkflow = {
			name: "AI agent model wiring test",
			nodes: [
				{
					id: "trigger1",
					name: "Chat Trigger",
					type: "@n8n/n8n-nodes-langchain.chatTrigger",
					position: [200, 300],
					parameters: {},
					typeVersion: 1,
				},
				{
					id: "agent1",
					name: "My Agent",
					type: "@n8n/n8n-nodes-langchain.agent",
					position: [500, 300],
					parameters: { systemMessage: "You are a helpful assistant." },
					typeVersion: 1,
				},
				{
					id: "gemini1",
					name: "Gemini Model",
					type: "@n8n/n8n-nodes-langchain.lmChatGoogleGemini",
					position: [500, 600],
					parameters: { modelName: "gemini-2.0-flash" },
					typeVersion: 1,
				},
			],
			connections: {
				"Chat Trigger": {
					main: [[{ node: "My Agent", type: "main", index: 0 }]],
				},
				"Gemini Model": {
					ai_languageModel: [
						[{ node: "My Agent", type: "ai_languageModel", index: 0 }],
					],
				},
			},
		};

		const result = translateN8n(workflow, mockCatalog);

		// Agent should be composed: agent_invoke + agent_from_model + agent_set_system_prompt
		const nodes = Object.values(result.board.nodes);
		const agentInvoke = nodes.find((n) => n.name === "agent_invoke");
		const agentFromModel = nodes.find((n) => n.name === "agent_from_model");
		const setPrompt = nodes.find((n) => n.name === "agent_set_system_prompt");
		const geminiNode = nodes.find(
			(n) => n.name === "ai_generative_build_gemini",
		);

		expect(agentInvoke).toBeDefined();
		expect(agentFromModel).toBeDefined();
		expect(setPrompt).toBeDefined();
		expect(geminiNode).toBeDefined();

		// Model builder should be placed inside the same layer as agent_from_model
		expect(geminiNode!.layer).toBe(agentFromModel!.layer);
		expect(geminiNode!.layer).toBe(agentInvoke!.layer);

		// Gemini model output directly wired to agent_from_model model input (same layer, no bridge)
		const geminiModelOut = Object.values(geminiNode!.pins).find(
			(p) => p.name === "model" && p.pin_type === IPinType.Output,
		);
		const fromModelIn = Object.values(agentFromModel!.pins).find(
			(p) => p.name === "model" && p.pin_type === IPinType.Input,
		);
		expect(geminiModelOut).toBeDefined();
		expect(fromModelIn).toBeDefined();

		// Direct connection: gemini output → agent_from_model input
		expect(geminiModelOut!.connected_to).toContain(fromModelIn!.id);
		expect(fromModelIn!.depends_on).toContain(geminiModelOut!.id);

		// Model ID should be set on the gemini node
		const modelIdPin = Object.values(geminiNode!.pins).find(
			(p) => p.name === "model_id" && p.pin_type === IPinType.Input,
		);
		expect(modelIdPin).toBeDefined();
		expect(decodePinDefault(modelIdPin!)).toBe("gemini-2.0-flash");
	});
});
