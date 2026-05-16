import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "layer",
	nodes: [
		{
			id: "to_session",
			catalog: "discord_to_session",
			offset: [-300, 0],
			nameSuffix: "Discord Session",
		},
		{
			id: "send",
			catalog: "discord_send_message",
			offset: [0, 0],
			primary: true,
		},
	],
	connections: [["to_session:session", "send:session"]],
	defaults: {
		"send:content": "$content",
		"send:channel_id": "$channel_id",
	},
};
export default def;
