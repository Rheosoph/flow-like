import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "layer",
	nodes: [
		{
			id: "to_session",
			catalog: "telegram_to_session",
			offset: [-300, 0],
			nameSuffix: "Telegram Session",
		},
		{
			id: "send",
			catalog: "telegram_send_message",
			offset: [0, 0],
			primary: true,
		},
	],
	connections: [["to_session:session", "send:session"]],
	defaults: {
		"send:message": "$message",
	},
};
export default def;
