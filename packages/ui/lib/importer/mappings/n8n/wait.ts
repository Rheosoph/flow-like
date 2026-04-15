import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.wait",
	parameters: {
		time: {
			path: "parameters.amount",
			default: 1,
			transform: "number",
		},
	},
};
export default def;
