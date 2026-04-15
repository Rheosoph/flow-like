import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "direct",
	catalog: "delay",
	defaults: {
		time: "$time_ms",
	},
};
export default def;
