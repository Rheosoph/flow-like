import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.webhook",
	isEvent: true,
	warnings: [
		"Webhook trigger mapped to generic event. Use the event's payload pin for incoming request data.",
	],
};
export default def;
