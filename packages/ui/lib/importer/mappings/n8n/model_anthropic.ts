import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "@n8n/n8n-nodes-langchain.lmChatAnthropic",
	parameters: {
		model_id: {
			path: "parameters.modelName",
			fallback: "parameters.model",
		},
	},
};
export default def;
