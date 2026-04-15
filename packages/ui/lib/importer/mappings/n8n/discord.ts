import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.discord",
	parameters: {
		content: "parameters.content",
		channel_id: "parameters.channelId",
	},
};
export default def;
