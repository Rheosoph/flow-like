import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "layer",
	skipExecPins: true,
	nodes: [
		{
			id: "make_request",
			catalog: "http_make_request",
			offset: [-300, 0],
			nameSuffix: "(Request)",
		},
		{
			id: "fetch",
			catalog: "http_fetch",
			offset: [0, 0],
			primary: true,
		},
		{
			id: "to_json",
			catalog: "http_response_to_json",
			offset: [300, 0],
			nameSuffix: "(To Struct)",
		},
		{
			id: "get_field",
			catalog: "struct_get",
			offset: [600, 0],
			nameSuffix: "(Get Field)",
		},
	],
	connections: [
		["make_request:request", "fetch:request"],
		["fetch:response", "to_json:response"],
		["fetch:exec_out", "to_json:exec_in"],
		["to_json:struct", "get_field:struct"],
	],
	defaults: {
		"make_request:method": "$method",
		"make_request:url": "$url",
		"get_field:field": "data",
	},
};
export default def;
