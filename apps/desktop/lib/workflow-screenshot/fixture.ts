import { Buffer } from "node:buffer";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import type { INode } from "@flow-like/flow-like-ui/lib/schema/flow/node";
import type {
	DocScreenshotTauriFixture,
	JsonValue,
} from "../doc-screenshot/types";

export const WORKFLOW_SCREENSHOT_APP_ID = "flowscript-render-app";
export const WORKFLOW_SCREENSHOT_BOARD_ID = "flowscript-render-board";

const templatePath = resolve(
	dirname(fileURLToPath(import.meta.url)),
	"../doc-screenshot/fixtures/docs-studio.tauri.json",
);

function replaceTemplateIds(value: JsonValue): JsonValue {
	if (typeof value === "string") {
		return value
			.replaceAll("docs-app", WORKFLOW_SCREENSHOT_APP_ID)
			.replaceAll("docs-board", WORKFLOW_SCREENSHOT_BOARD_ID);
	}
	if (Array.isArray(value)) return value.map(replaceTemplateIds);
	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value).map(([key, entry]) => [
				key,
				replaceTemplateIds(entry),
			]),
		);
	}
	return value;
}

function syncPin(pin: Record<string, unknown>): Record<string, unknown> {
	const bytes = pin.default_value;
	return {
		...pin,
		default_value:
			Array.isArray(bytes) && bytes.every((value) => typeof value === "number")
				? Buffer.from(bytes).toString("base64")
				: null,
	};
}

function syncNode(node: INode): Record<string, unknown> {
	return {
		...node,
		pins: Object.fromEntries(
			Object.entries(node.pins ?? {}).map(([id, pin]) => [
				id,
				syncPin(pin as unknown as Record<string, unknown>),
			]),
		),
		h: false,
	};
}

/** Build the first-load full response expected by the desktop's incremental BoardSync client. */
export function workflowBoardSyncResponse(board: IBoard): JsonValue {
	const segments: Record<
		string,
		{ hash: string; nodes: Record<string, Record<string, unknown>> }
	> = Object.create(null);
	for (const node of Object.values(board.nodes)) {
		const segmentId = node.layer || "__root__";
		let segment = segments[segmentId];
		if (!segment) {
			segment = {
				hash: `workflow-render-segment-${segmentId}`,
				nodes: Object.create(null),
			};
			segments[segmentId] = segment;
		}
		segment.nodes[node.id] = syncNode(node);
	}

	return {
		manifest: {
			meta: "workflow-render-meta-v1",
			variables: "workflow-render-variables-v1",
			comments: "workflow-render-comments-v1",
			layers: Object.fromEntries(
				Object.keys(board.layers).map((id) => [
					id,
					`workflow-render-layer-${id}`,
				]),
			),
			segments: Object.fromEntries(
				Object.entries(segments).map(([id, segment]) => [id, segment.hash]),
			),
		},
		meta: {
			id: board.id,
			name: board.name,
			description: board.description,
			viewport: board.viewport,
			version: board.version,
			stage: board.stage,
			log_level: board.log_level,
			execution_mode: board.execution_mode,
			page_ids: board.page_ids,
			hash: board.hash ?? null,
			created_at: board.created_at,
			updated_at: board.updated_at,
		},
		variables: board.variables,
		comments: board.comments,
		layers: board.layers,
		refs: board.refs,
		segments,
	} as JsonValue;
}

export async function buildWorkflowScreenshotFixture(
	board: IBoard,
	catalog: INode[],
): Promise<DocScreenshotTauriFixture> {
	const template = JSON.parse(
		await readFile(templatePath, "utf8"),
	) as DocScreenshotTauriFixture;
	const fixture = replaceTemplateIds(
		template as unknown as JsonValue,
	) as unknown as DocScreenshotTauriFixture | undefined;
	if (!fixture?.responses) {
		throw new Error("Workflow screenshot Tauri fixture template is invalid.");
	}

	const app = fixture.responses.get_app as Record<string, JsonValue>;
	app.id = WORKFLOW_SCREENSHOT_APP_ID;
	app.boards = [WORKFLOW_SCREENSHOT_BOARD_ID];
	app.visibility = "Offline";
	const appMeta = fixture.responses.get_app_meta as Record<string, JsonValue>;
	appMeta.name = board.name;
	appMeta.description = board.description;

	fixture.responses.get_app_boards = [board as unknown as JsonValue];
	fixture.responses.get_board = board as unknown as JsonValue;
	fixture.responses.sync_board = workflowBoardSyncResponse(board);
	fixture.responses.get_catalog = catalog as unknown as JsonValue;
	fixture.responses.get_board_versions = [board.version as JsonValue];
	fixture.responses.list_runs = [];
	fixture.responses.query_run = [];
	return fixture;
}
