import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "direct",
	catalog: "ai_generative_build_huggingface",
	defaults: {
		model_id: "$model_id",
	},
};
export default def;
