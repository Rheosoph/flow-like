import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.httpRequest",
	parameters: {
		method: {
			path: "parameters.method",
			default: "GET",
			transform: "uppercase",
		},
		url: {
			path: "parameters.url",
			default: "",
		},
	},
};
export default def;
