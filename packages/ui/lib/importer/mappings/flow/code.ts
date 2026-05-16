import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "direct",
	catalog: "python_interpreter",
	defaults: {
		code: "$code",
	},
};
export default def;
