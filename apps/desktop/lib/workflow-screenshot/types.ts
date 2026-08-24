import type { LayoutStyle } from "@flow-like/flow-like-ui/lib/flow-auto-layout";
import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import type { INode } from "@flow-like/flow-like-ui/lib/schema/flow/node";
import type {
	DocScreenshotResult,
	DocScreenshotTheme,
} from "../doc-screenshot/types";

export const WORKFLOW_RENDER_DATA_SCHEMA =
	"flow-like.flowscript-render-data/v1" as const;
export const WORKFLOW_SCREENSHOT_RESULT_SCHEMA =
	"flow-like.workflow-screenshot-result/v1" as const;
export const WORKFLOW_NODE_LIST_SCHEMA =
	"flow-like.workflow-screenshot-nodes/v1" as const;

export type WorkflowScreenshotFormat = "webp" | "png" | "jpeg";

export interface WorkflowRenderData {
	schema: typeof WORKFLOW_RENDER_DATA_SCHEMA;
	board: IBoard;
	catalog: INode[];
	canonical_flowscript: string;
}

export interface WorkflowScreenshotCliOptions {
	input?: string;
	output?: string;
	name?: string;
	layout: LayoutStyle;
	focusNode?: string;
	listNodes: boolean;
	viewport: { width: number; height: number };
	dpr: number;
	theme: DocScreenshotTheme;
	quality?: number;
	frontendUrl?: string;
	port?: number;
	timeoutMs: number;
	settleMs: number;
	json: boolean;
	help: boolean;
}

export interface WorkflowFocusTarget {
	id: string;
	kind: "node" | "layer";
	label: string;
	matchedBy: "id" | "anchor" | "name" | "default";
}

export interface WorkflowNodeDescriptor {
	id: string;
	kind: "node" | "layer";
	name: string;
	friendlyName?: string;
	layer?: string;
}

export interface WorkflowScreenshotResult {
	schema: typeof WORKFLOW_SCREENSHOT_RESULT_SCHEMA;
	passed: boolean;
	input: string;
	output: string;
	layout: LayoutStyle;
	focus?: WorkflowFocusTarget;
	board: {
		id: string;
		name: string;
		nodes: number;
		layers: number;
	};
	screenshot: DocScreenshotResult;
}
