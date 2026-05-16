import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.googleSheets",
	parameters: {
		spreadsheet_id: {
			path: "parameters.documentId.value",
			default: "",
		},
		range: "parameters.range",
		operation: "parameters.operation",
	},
};
export default def;
