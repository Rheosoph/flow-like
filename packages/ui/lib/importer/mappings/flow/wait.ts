import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "direct",
	catalog: "delay",
	defaults: {
		time: "$time",
	},
};
export default def;
