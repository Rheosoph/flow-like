import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.switch",
	warnings: [
		"Switch mapped to branch (boolean). For multi-way routing, chain multiple branch nodes.",
	],
};
export default def;
