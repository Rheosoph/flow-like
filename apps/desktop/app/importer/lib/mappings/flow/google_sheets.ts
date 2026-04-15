import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "layer",
	skipExecPins: true,
	nodes: [
		{
			id: "provider",
			catalog: "data_google_provider",
			offset: [-300, 0],
			nameSuffix: "Google",
		},
		{
			id: "sheets",
			catalog: "data_google_sheets_read_range",
			offset: [0, 0],
			primary: true,
		},
	],
	connections: [["provider:provider", "sheets:provider"]],
	defaults: {
		"sheets:spreadsheet_id": "$spreadsheet_id",
		"sheets:range": "$range",
	},
};
export default def;
