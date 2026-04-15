import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.respondToWebhook",
	warnings: [
		"Respond to Webhook has no direct equivalent. Mapped to log_info as placeholder.",
	],
};
export default def;
