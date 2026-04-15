import type { FlowNodeDef } from "../types";

const def: FlowNodeDef = {
	mode: "layer",
	nodes: [
		{
			id: "smtp_connect",
			catalog: "email_smtp_connect",
			offset: [-300, 0],
			nameSuffix: "SMTP Connect",
		},
		{
			id: "smtp_send",
			catalog: "email_smtp_send",
			offset: [0, 0],
			primary: true,
		},
	],
	connections: [["smtp_connect:connection", "smtp_send:connection"]],
	defaults: {
		"smtp_send:to": "$to",
		"smtp_send:subject": "$subject",
		"smtp_send:body_text": "$body_text",
	},
};
export default def;
