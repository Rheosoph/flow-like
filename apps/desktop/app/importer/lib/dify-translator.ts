import type { INode, IPin } from "@tm9657/flow-like-ui";
import { IPinType, IValueType, IVariableType } from "@tm9657/flow-like-ui";
import {
	addExecPins,
	addLayerToBoard,
	addNodeToBoard,
	addVariableToBoard,
	computeLayerCoordinates,
	connectPins,
	createBridgePinsForBoard,
	createEmptyBoard,
	createNode,
	createPin,
	createTodoLayer,
	createVariable,
	diagError,
	findPinByName,
	info,
	warn,
} from "./board-builder";
import type {
	DifyNode,
	DifyWorkflow,
	NodeMappingType,
	TranslationDiagnostic,
	TranslationResult,
} from "./types";

interface DifyNodeMapping {
	catalog: string;
	category: string;
	type: NodeMappingType;
	setupPins?: (
		node: INode,
		difyNode: DifyNode,
		diag: TranslationDiagnostic[],
	) => void;
}

const DIFY_NODE_REGISTRY: Record<string, DifyNodeMapping> = {
	llm: {
		catalog: "invoke_llm",
		category: "AI/LLM",
		type: "direct",
		setupPins: (node, difyNode, diag) => {
			const data = difyNode.data;
			if (data.model && typeof data.model === "object") {
				const model = data.model as Record<string, unknown>;
				if (model.name) {
					const pin = createPin({
						name: "model_name",
						friendlyName: "Model",
						description: "LLM model name",
						pinType: IPinType.Input,
						dataType: IVariableType.String,
						defaultValue: model.name,
					});
					node.pins[pin.id] = pin;
				}
				if (model.provider) {
					const pin = createPin({
						name: "model_provider",
						friendlyName: "Provider",
						description: "Model provider",
						pinType: IPinType.Input,
						dataType: IVariableType.String,
						defaultValue: model.provider,
					});
					node.pins[pin.id] = pin;
				}
			}
			if (data.prompt_template) {
				const promptData = data.prompt_template;
				if (Array.isArray(promptData)) {
					const roleCounts = new Map<string, number>();
					for (const msg of promptData) {
						const m = msg as Record<string, unknown>;
						if (m.text) {
							const role = String(m.role ?? "user");
							const count = (roleCounts.get(role) ?? 0) + 1;
							roleCounts.set(role, count);
							const promptName =
								count === 1 ? `prompt_${role}` : `prompt_${role}_${count}`;
							const pin = createPin({
								name: promptName,
								friendlyName:
									count === 1 ? `${role} prompt` : `${role} prompt ${count}`,
								description:
									count === 1
										? `Prompt message (${role})`
										: `Prompt message (${role}, ${count})`,
								pinType: IPinType.Input,
								dataType: IVariableType.String,
								defaultValue: m.text,
							});
							node.pins[pin.id] = pin;
						}
					}
				}
			}
			const outputPin = createPin({
				name: "llm_output",
				friendlyName: "Output",
				description: "LLM response text",
				pinType: IPinType.Output,
				dataType: IVariableType.String,
			});
			node.pins[outputPin.id] = outputPin;
		},
	},
	"if-else": {
		catalog: "control_branch",
		category: "Flow/Control",
		type: "direct",
		setupPins: (node, difyNode, diag) => {
			const condPin = createPin({
				name: "condition",
				friendlyName: "Condition",
				description: "Branch condition",
				pinType: IPinType.Input,
				dataType: IVariableType.Boolean,
			});
			node.pins[condPin.id] = condPin;

			const truePin = createPin({
				name: "exec_true",
				friendlyName: "True",
				description: "Execute when condition is true",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			});
			node.pins[truePin.id] = truePin;

			const falsePin = createPin({
				name: "exec_false",
				friendlyName: "False",
				description: "Execute when condition is false",
				pinType: IPinType.Output,
				dataType: IVariableType.Execution,
			});
			node.pins[falsePin.id] = falsePin;

			if (difyNode.data.conditions) {
				info(
					diag,
					"If-else conditions imported. Review branch logic manually.",
					difyNode.id,
					difyNode.data.title,
				);
			}
		},
	},
	iteration: {
		catalog: "control_for_each",
		category: "Flow/Control",
		type: "direct",
		setupPins: (node, difyNode, diag) => {
			const inputPin = createPin({
				name: "input_array",
				friendlyName: "Items",
				description: "Array to iterate over",
				pinType: IPinType.Input,
				dataType: IVariableType.Generic,
				valueType: IValueType.Array,
			});
			node.pins[inputPin.id] = inputPin;

			const itemPin = createPin({
				name: "current_item",
				friendlyName: "Current Item",
				description: "Current iteration item",
				pinType: IPinType.Output,
				dataType: IVariableType.Generic,
			});
			node.pins[itemPin.id] = itemPin;

			const indexPin = createPin({
				name: "current_index",
				friendlyName: "Index",
				description: "Current iteration index",
				pinType: IPinType.Output,
				dataType: IVariableType.Integer,
			});
			node.pins[indexPin.id] = indexPin;
		},
	},
	loop: {
		catalog: "control_while_loop",
		category: "Flow/Control",
		type: "direct",
	},
	"http-request": {
		catalog: "http_fetch",
		category: "Web/HTTP",
		type: "direct",
		setupPins: (node, difyNode, diag) => {
			const data = difyNode.data;
			if (data.url) {
				const urlPin = createPin({
					name: "url",
					friendlyName: "URL",
					description: "Request URL",
					pinType: IPinType.Input,
					dataType: IVariableType.String,
					defaultValue: data.url,
				});
				node.pins[urlPin.id] = urlPin;
			}
			if (data.method) {
				const methodPin = createPin({
					name: "method",
					friendlyName: "Method",
					description: "HTTP method",
					pinType: IPinType.Input,
					dataType: IVariableType.String,
					defaultValue: data.method,
				});
				node.pins[methodPin.id] = methodPin;
			}
			const responsePin = createPin({
				name: "response_body",
				friendlyName: "Response",
				description: "HTTP response body",
				pinType: IPinType.Output,
				dataType: IVariableType.String,
			});
			node.pins[responsePin.id] = responsePin;
		},
	},
	"knowledge-retrieval": {
		catalog: "vector_search",
		category: "Data/Vector",
		type: "composition",
		setupPins: (node, difyNode, diag) => {
			const queryPin = createPin({
				name: "search_query",
				friendlyName: "Query",
				description: "Search query text",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			});
			node.pins[queryPin.id] = queryPin;

			const resultsPin = createPin({
				name: "search_results",
				friendlyName: "Results",
				description: "Retrieved documents",
				pinType: IPinType.Output,
				dataType: IVariableType.String,
				valueType: IValueType.Array,
			});
			node.pins[resultsPin.id] = resultsPin;

			warn(
				diag,
				"Knowledge retrieval requires: embed query → vector search → format results. Wire these nodes.",
				difyNode.id,
				difyNode.data.title,
			);
		},
	},
	"question-classifier": {
		catalog: "invoke_llm",
		category: "AI/LLM",
		type: "composition",
		setupPins: (node, difyNode, diag) => {
			const inputPin = createPin({
				name: "classification_input",
				friendlyName: "Input",
				description: "Text to classify",
				pinType: IPinType.Input,
				dataType: IVariableType.String,
			});
			node.pins[inputPin.id] = inputPin;

			const outputPin = createPin({
				name: "classification_result",
				friendlyName: "Class",
				description: "Classification result",
				pinType: IPinType.Output,
				dataType: IVariableType.String,
			});
			node.pins[outputPin.id] = outputPin;

			warn(
				diag,
				"Question classifier: Compose with LLM + Branch. Add classification prompt and match classes.",
				difyNode.id,
				difyNode.data.title,
			);
		},
	},
	"parameter-extractor": {
		catalog: "llm_extract",
		category: "AI/LLM",
		type: "direct",
	},
	"variable-assigner": {
		catalog: "variable_set",
		category: "Data/Variable",
		type: "direct",
	},
	"variable-aggregator": {
		catalog: "gather",
		category: "Flow/Control",
		type: "composition",
	},
	agent: {
		catalog: "simple_agent",
		category: "AI/Agent",
		type: "direct",
	},
	tool: {
		catalog: "mcp_tool",
		category: "AI/Tool",
		type: "composition",
	},
	"doc-extractor": {
		catalog: "markitdown_convert",
		category: "Processing",
		type: "direct",
	},
	"list-operator": {
		catalog: "array_filter",
		category: "Data/Array",
		type: "composition",
	},
	code: {
		catalog: "dify_code",
		category: "Code/Dify",
		type: "direct",
		setupPins: (node, difyNode, diag) => {
			const data = difyNode.data;
			if (data.code && typeof data.code === "string") {
				const codePin = createPin({
					name: "code",
					friendlyName: "Code",
					description: "Python source code",
					pinType: IPinType.Input,
					dataType: IVariableType.String,
					defaultValue: data.code,
				});
				node.pins[codePin.id] = codePin;
			}
			if (data.variables && Array.isArray(data.variables)) {
				const inputSchema: Record<string, string> = {};
				for (const v of data.variables as Array<Record<string, unknown>>) {
					if (v.variable && typeof v.variable === "string") {
						inputSchema[v.variable] = "string";
					}
				}
				if (Object.keys(inputSchema).length > 0) {
					const pin = createPin({
						name: "input_schema",
						friendlyName: "Input Schema",
						description: "Auto-generated from Dify variables",
						pinType: IPinType.Input,
						dataType: IVariableType.String,
						defaultValue: JSON.stringify(inputSchema),
					});
					node.pins[pin.id] = pin;
				}
			}
			if (
				data.outputs &&
				typeof data.outputs === "object" &&
				!Array.isArray(data.outputs)
			) {
				const outputSchema: Record<string, string> = {};
				const outputs = data.outputs as Record<string, unknown>;
				for (const [key, val] of Object.entries(outputs)) {
					const typed = val as Record<string, unknown>;
					outputSchema[key] =
						typeof typed.type === "string" ? typed.type : "string";
				}
				if (Object.keys(outputSchema).length > 0) {
					const pin = createPin({
						name: "output_schema",
						friendlyName: "Output Schema",
						description: "Auto-generated from Dify outputs",
						pinType: IPinType.Input,
						dataType: IVariableType.String,
						defaultValue: JSON.stringify(outputSchema),
					});
					node.pins[pin.id] = pin;
				}
			}
		},
	},
	"template-transform": {
		catalog: "string_format",
		category: "Data/String",
		type: "composition",
	},
};

function mapDifyVarType(difyType: string): IVariableType {
	switch (difyType) {
		case "string":
		case "text":
		case "paragraph":
		case "select":
			return IVariableType.String;
		case "number":
			return IVariableType.Float;
		case "boolean":
			return IVariableType.Boolean;
		default:
			return IVariableType.String;
	}
}

export function translateDify(workflow: DifyWorkflow): TranslationResult {
	const diagnostics: TranslationDiagnostic[] = [];
	const appName = workflow.app?.name || "Imported Dify Workflow";
	const board = createEmptyBoard(
		appName,
		`Imported from Dify ${workflow.app?.mode ?? "workflow"} app${workflow.app?.description ? `: ${workflow.app.description}` : ""}`,
	);

	info(
		diagnostics,
		`Starting translation of Dify ${workflow.app?.mode ?? "workflow"}: ${appName}`,
	);

	const graph = workflow.workflow?.graph;
	if (!graph) {
		diagError(diagnostics, "No workflow graph found in Dify DSL");
		return {
			format: "dify",
			status: "error",
			board,
			diagnostics,
			stats: {
				totalNodes: 0,
				directMapped: 0,
				composed: 0,
				todo: 0,
				connections: 0,
				variables: 0,
			},
		};
	}

	// Set viewport
	if (graph.viewport) {
		board.viewport = [graph.viewport.x, graph.viewport.y, graph.viewport.zoom];
	}

	// Phase 1: Environment variables
	let variableCount = 0;
	const envVars = workflow.workflow?.environment_variables ?? [];
	for (const envVar of envVars) {
		const variable = createVariable({
			name: envVar.name,
			description:
				envVar.description || `Dify environment variable: ${envVar.name}`,
			dataType: mapDifyVarType(envVar.value_type),
			defaultValue: envVar.value,
			secret: envVar.value_type === "secret",
			exposed: true,
			editable: true,
		});
		addVariableToBoard(board, variable);
		variableCount++;
	}

	const convVars = workflow.workflow?.conversation_variables ?? [];
	for (const convVar of convVars) {
		const variable = createVariable({
			name: `conv_${convVar.name}`,
			description: `Dify conversation variable: ${convVar.name}`,
			dataType: mapDifyVarType(convVar.value_type),
			defaultValue: convVar.value,
		});
		addVariableToBoard(board, variable);
		variableCount++;
	}

	// Phase 2: Translate nodes
	const difyIdToFlowNode = new Map<string, INode>();
	let directCount = 0;
	let composedCount = 0;
	let todoCount = 0;

	for (const difyNode of graph.nodes) {
		const nodeType = difyNode.data?.type ?? difyNode.type;

		// Start and end nodes map to board I/O, not actual nodes
		if (nodeType === "start") {
			translateStartNode(board, difyNode, diagnostics);
			info(
				diagnostics,
				"Start node mapped to board input variables",
				difyNode.id,
				difyNode.data?.title,
			);
			continue;
		}
		if (nodeType === "end" || nodeType === "answer") {
			translateEndNode(board, difyNode, diagnostics);
			info(
				diagnostics,
				`${nodeType} node mapped to board output`,
				difyNode.id,
				difyNode.data?.title,
			);
			continue;
		}

		const mapping = DIFY_NODE_REGISTRY[nodeType];

		if (mapping) {
			const flowNode = createNode({
				name: mapping.catalog,
				friendlyName: difyNode.data?.title || nodeType,
				description: difyNode.data?.desc || `Imported from Dify: ${nodeType}`,
				category: mapping.category,
				x: difyNode.position?.x ?? 0,
				y: difyNode.position?.y ?? 0,
				comment:
					mapping.type === "composition"
						? `Composed from Dify ${nodeType}. Review and adjust pin connections.`
						: undefined,
			});

			addExecPins(flowNode);

			if (mapping.setupPins) {
				mapping.setupPins(flowNode, difyNode, diagnostics);
			} else {
				addDefaultDifyPins(flowNode, difyNode, diagnostics);
			}

			addNodeToBoard(board, flowNode);
			difyIdToFlowNode.set(difyNode.id, flowNode);

			if (mapping.type === "direct") {
				directCount++;
				info(
					diagnostics,
					`Direct mapping: ${nodeType} → ${mapping.catalog}`,
					difyNode.id,
					difyNode.data?.title,
				);
			} else {
				composedCount++;
				warn(
					diagnostics,
					`Composition needed: ${nodeType} → ${mapping.catalog}. Review and adjust.`,
					difyNode.id,
					difyNode.data?.title,
				);
			}
		} else {
			// Unknown → TODO layer
			todoCount++;
			const layer = createTodoLayer({
				name: `TODO: ${difyNode.data?.title || nodeType}`,
				comment: `TODO: Dify node "${nodeType}" has no flow-like equivalent.\n\nOriginal config: ${JSON.stringify(difyNode.data, null, 2)}`,
				x: difyNode.position?.x ?? 0,
				y: difyNode.position?.y ?? 0,
			});

			const placeholder = createNode({
				name: "todo_placeholder",
				friendlyName: difyNode.data?.title || nodeType,
				description: `TODO: Implement Dify node "${nodeType}"`,
				category: "TODO",
				x: difyNode.position?.x ?? 0,
				y: difyNode.position?.y ?? 0,
				comment: `Original Dify type: ${nodeType}\nConfig: ${JSON.stringify(difyNode.data, null, 2)}`,
				layer: layer.id,
			});

			addExecPins(placeholder);
			addDefaultDifyPins(placeholder, difyNode, diagnostics);
			addNodeToBoard(board, placeholder);

			addLayerToBoard(board, layer);
			difyIdToFlowNode.set(difyNode.id, placeholder);

			diagError(
				diagnostics,
				`No mapping for Dify node type "${nodeType}". Created TODO layer.`,
				difyNode.id,
				difyNode.data?.title,
			);
		}
	}

	// Phase 3: Translate edges
	let connectionCount = 0;

	for (const edge of graph.edges) {
		const sourceNode = difyIdToFlowNode.get(edge.source);
		const targetNode = difyIdToFlowNode.get(edge.target);

		if (!sourceNode || !targetNode) {
			// Source/target might be start/end nodes
			continue;
		}

		// Determine which output pin to use based on sourceHandle
		let sourcePin: IPin | undefined;
		if (edge.sourceHandle === "true") {
			sourcePin = findPinByName(sourceNode, "exec_true", IPinType.Output);
		} else if (edge.sourceHandle === "false") {
			sourcePin = findPinByName(sourceNode, "exec_false", IPinType.Output);
		} else {
			sourcePin = findPinByName(sourceNode, "exec_out", IPinType.Output);
		}

		const targetPin = findPinByName(targetNode, "exec_in", IPinType.Input);

		if (sourcePin && targetPin) {
			connectPins(sourceNode, sourcePin, targetNode, targetPin);
			connectionCount++;
		} else {
			warn(
				diagnostics,
				`Could not wire edge from "${edge.source}" (${edge.sourceHandle}) to "${edge.target}". Add connection manually.`,
			);
		}
	}

	// Phase 3.5: Create bridge pins and compute layer coordinates
	createBridgePinsForBoard(board);
	computeLayerCoordinates(board);

	const hasErrors = diagnostics.some((d) => d.level === "error");
	const status = hasErrors
		? directCount > 0
			? "partial"
			: "error"
		: "success";

	info(
		diagnostics,
		`Translation complete: ${directCount} direct, ${composedCount} composed, ${todoCount} TODO, ${connectionCount} connections, ${variableCount} variables`,
	);

	return {
		format: "dify",
		status,
		board,
		diagnostics,
		stats: {
			totalNodes: graph.nodes.filter((n) => {
				const t = n.data?.type ?? n.type;
				return t !== "start" && t !== "end" && t !== "answer";
			}).length,
			directMapped: directCount,
			composed: composedCount,
			todo: todoCount,
			connections: connectionCount,
			variables: variableCount,
		},
	};
}

function translateStartNode(
	board: ReturnType<typeof createEmptyBoard>,
	difyNode: DifyNode,
	diag: TranslationDiagnostic[],
): void {
	const data = difyNode.data;
	const variables = data.variables as
		| Array<Record<string, unknown>>
		| undefined;
	if (!variables) return;

	for (const v of variables) {
		const variable = createVariable({
			name: String(v.variable ?? v.label ?? "input"),
			description: String(v.label ?? ""),
			dataType: mapDifyVarType(String(v.type ?? "string")),
			defaultValue: v.default ?? undefined,
			exposed: true,
			editable: true,
		});
		addVariableToBoard(board, variable);
	}
}

function translateEndNode(
	board: ReturnType<typeof createEmptyBoard>,
	difyNode: DifyNode,
	diag: TranslationDiagnostic[],
): void {
	const data = difyNode.data;
	const outputs = data.outputs as Array<Record<string, unknown>> | undefined;
	if (!outputs) return;

	for (const out of outputs) {
		const variable = createVariable({
			name: `output_${String(out.variable ?? out.label ?? "output")}`,
			description: `Output: ${String(out.label ?? "")}`,
			dataType: mapDifyVarType(String(out.value_selector_type ?? "string")),
			exposed: true,
		});
		addVariableToBoard(board, variable);
	}
}

function addDefaultDifyPins(
	node: INode,
	difyNode: DifyNode,
	diag: TranslationDiagnostic[],
): void {
	const data = difyNode.data;
	for (const [key, value] of Object.entries(data)) {
		if (key === "type" || key === "title" || key === "desc") continue;
		if (value === undefined || value === null) continue;
		if (typeof value === "object") continue;

		const dataType =
			typeof value === "number"
				? IVariableType.Float
				: typeof value === "boolean"
					? IVariableType.Boolean
					: IVariableType.String;

		const pin = createPin({
			name: `param_${key}`,
			friendlyName: key,
			description: `Dify parameter: ${key}`,
			pinType: IPinType.Input,
			dataType,
			defaultValue: value,
		});
		node.pins[pin.id] = pin;
	}
}
