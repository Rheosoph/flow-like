import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.scheduleTrigger",
	isEvent: true,
	warnings: [
		"Schedule trigger mapped to simple event. Flow-Like does not have a built-in cron trigger; use an external scheduler to invoke this flow.",
	],
};
export default def;
