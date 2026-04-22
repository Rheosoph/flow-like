import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.code",
	parameters: {
		code: "parameters.jsCode",
	},
	warnings: [
		"n8n Code node contains JavaScript; manual conversion to Python is required.",
	],
};
export default def;
