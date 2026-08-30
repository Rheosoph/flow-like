import { Buffer } from "node:buffer";
import {
	type IBoardSyncResponse,
	applyBoardSync,
} from "@flow-like/flow-like-ui/lib/board-sync";
import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import { expect, test } from "vitest";
import { workflowBoardSyncResponse } from "../fixture";

test("workflow sync response carries laid-out coordinates and base64 pin values", () => {
	const board = {
		id: "flowscript-render-board",
		name: "Example",
		description: "",
		nodes: {
			node: {
				id: "node",
				name: "example",
				friendly_name: "Example",
				category: "",
				description: "",
				coordinates: [420, 240, 0],
				pins: {
					pin: {
						id: "pin",
						name: "value",
						friendly_name: "Value",
						description: "",
						pin_type: "Input",
						data_type: "String",
						value_type: "Normal",
						index: 1,
						depends_on: [],
						connected_to: [],
						default_value: [34, 111, 107, 34],
					},
				},
			},
		},
		layers: {},
		refs: {},
		comments: {},
		variables: {},
		viewport: [0, 0, 1],
		version: [0, 0, 1],
		stage: "Dev",
		log_level: "Info",
		execution_mode: "Local",
		page_ids: [],
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	} as unknown as IBoard;

	const response = workflowBoardSyncResponse(
		board,
	) as unknown as IBoardSyncResponse;
	const rootSegment = response.segments?.__root__;
	const syncedNode = rootSegment?.nodes.node;
	const syncedPin = syncedNode?.pins.pin;
	if (!rootSegment || !syncedNode || !syncedPin) {
		throw new Error("The root sync fixture is incomplete.");
	}
	expect(syncedNode.coordinates).toEqual([420, 240, 0]);
	expect(syncedPin.default_value).toBe(
		Buffer.from([34, 111, 107, 34]).toString("base64"),
	);
	expect(response.manifest?.segments.__root__).toBe(rootSegment.hash);

	const materialized = applyBoardSync(undefined, response, undefined).board;
	expect(materialized.nodes.node.coordinates).toEqual([420, 240, 0]);
	expect(materialized.nodes.node.pins.pin.default_value).toEqual([
		34, 111, 107, 34,
	]);
});
