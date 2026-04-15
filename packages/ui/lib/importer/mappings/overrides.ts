import type { N8nManualMappingOverrides } from "./types";

export const N8N_MAPPING_OVERRIDES: N8nManualMappingOverrides = {
	"n8n-nodes-base.gmail": {
		flow: {
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
				"smtp_connect:host": "smtp.gmail.com",
				"smtp_connect:port": 587,
				"smtp_connect:encryption": "StartTls",
				"smtp_send:to": "$to",
				"smtp_send:subject": "$subject",
				"smtp_send:body_text": "$body_text",
			},
		},
	},
	"n8n-nodes-base.respondToWebhook": {
		n8n: {
			parameters: {
				response_body: "parameters.responseBody",
				respond_with: "parameters.respondWith",
			},
		},
		flow: {
			mode: "direct",
			catalog: "log_info",
			defaults: {
				message: "$response_body",
				toast: false,
			},
		},
	},
	// "n8n-nodes-base.wait": {
	// 	flow: {
	// 		mode: "layer",
	// 		skipExecPins: true,
	// 		nodes: [
	// 			{ id: "entry", catalog: "control_sequence", primary: true },
	// 			{
	// 				id: "log",
	// 				catalog: "log_info",
	// 				offset: [300, 0],
	// 				nameSuffix: "(Mapped)",
	// 			},
	// 		],
	// 		connections: [["entry:exec_out", "log:exec_in"]],
	// 		defaults: {
	// 			"log:message": "$time",
	// 			"log:toast": true,
	// 		},
	// 	},
	// },
};