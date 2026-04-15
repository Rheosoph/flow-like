import type { N8nNodeDef } from "../types";

const def: N8nNodeDef = {
	type: "n8n-nodes-base.gmail",
	parameters: {
		to: "parameters.sendTo",
		subject: "parameters.subject",
		body_text: "parameters.message",
	},
	warnings: [
		"Gmail mapped to SMTP send. The SMTP connect node needs host/port/credentials configured.",
	],
};
export default def;
