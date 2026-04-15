import type { IBoard, INode } from "../schema";

export type ImportFormat = "n8n" | "dify" | "unknown";

export type TranslationStatus = "pending" | "success" | "partial" | "error";

export type NodeMappingType = "direct" | "composition" | "todo";

export interface TranslationDiagnostic {
	level: "info" | "warn" | "error";
	nodeId?: string;
	nodeName?: string;
	message: string;
}

export interface TranslatedNode {
	originalId: string;
	originalType: string;
	originalName: string;
	mappingType: NodeMappingType;
	flowLikeNodes: INode[];
	diagnostics: TranslationDiagnostic[];
}

export interface TranslationResult {
	format: ImportFormat;
	status: TranslationStatus;
	board: IBoard;
	diagnostics: TranslationDiagnostic[];
	stats: {
		totalNodes: number;
		directMapped: number;
		composed: number;
		todo: number;
		connections: number;
		variables: number;
	};
}

// ---- n8n Types ----

export interface N8nWorkflow {
	id?: string;
	name: string;
	active?: boolean;
	nodes: N8nNode[];
	connections: N8nConnections;
	settings?: N8nWorkflowSettings;
	pinData?: Record<string, unknown>;
	versionId?: string;
	meta?: Record<string, unknown>;
}

export interface N8nNode {
	id: string;
	name: string;
	type: string;
	typeVersion?: number;
	position: [number, number];
	disabled?: boolean;
	parameters: Record<string, unknown>;
	credentials?: Record<string, { id: string; name: string }>;
	onError?: "stopWorkflow" | "continueRegularOutput" | "continueErrorOutput";
}

export interface N8nConnections {
	[nodeName: string]: {
		[connectionType: string]: Array<
			Array<{ node: string; type: string; index: number }>
		>;
	};
}

export interface N8nWorkflowSettings {
	executionOrder?: "v0" | "v1";
	timezone?: string;
	saveExecutionProgress?: boolean;
	saveManualExecutions?: boolean;
	callerPolicy?: string;
	errorWorkflow?: string;
	maxRunTime?: number;
}

// ---- Dify Types ----

export interface DifyWorkflow {
	version?: string;
	kind?: string;
	app: {
		name: string;
		mode: "workflow" | "advanced-chat";
		description?: string;
		icon?: string;
		icon_background?: string;
		icon_type?: string;
	};
	dependencies?: unknown[];
	workflow: {
		environment_variables?: DifyVariable[];
		conversation_variables?: DifyVariable[];
		features?: Record<string, unknown>;
		graph: {
			nodes: DifyNode[];
			edges: DifyEdge[];
			viewport?: { x: number; y: number; zoom: number };
		};
	};
}

export interface DifyNode {
	id: string;
	type: string;
	position: { x: number; y: number };
	data: {
		type: string;
		title: string;
		desc?: string;
		[key: string]: unknown;
	};
}

export interface DifyEdge {
	id: string;
	source: string;
	sourceHandle: string;
	target: string;
	targetHandle: string;
	type?: string;
	data?: {
		sourceType?: string;
		targetType?: string;
		[key: string]: unknown;
	};
}

export interface DifyVariable {
	id?: string;
	name: string;
	value_type: string;
	value?: unknown;
	description?: string;
}
