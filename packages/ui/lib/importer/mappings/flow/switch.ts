import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "direct",
	catalog: "control_branch",
	skipExecPins: true,
	defaults: {
		condition: true,
	},
};
export default def;
