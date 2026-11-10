import type { BotCapability } from "./bot-docs-data";

export interface CapabilityDetail {
	/** Tool names the capability offers the model. */
	readonly tools: readonly string[];
	/** When the bot stops and asks its owner first; absent means it never asks. */
	readonly asks?: string;
	readonly note?: string;
}

const APP_TOOLS = [
	"list_apps",
	"describe_app_interface",
	"get_app_detail",
	"call_app_event",
	"call_app_chat",
	"open_app_page",
	"interact_app_page",
] as const;

export const CAPABILITY_DETAILS: Readonly<
	Record<BotCapability, CapabilityDetail>
> = {
	internet: {
		tools: ["research_agent"],
		note: "A separate, sealed researcher searches and reads public pages. It only gets the question, never the bot's memory, files or secrets, so instructions hidden in a page can't reach them.",
	},
	web_requests: {
		tools: ["web_request"],
		asks: "Before each new site. Reading and writing are approved separately, and so is every secret it sends.",
		note: "Secrets go out over https only and are scrubbed from responses. Redirects are not followed. Large or binary responses are saved as workspace files.",
	},
	profile_apps: {
		tools: APP_TOOLS,
		asks: "Before running an app's events or pages, unless you allowed it.",
		note: "Every app in one of your profiles, with your role in each.",
	},
	selected_apps: {
		tools: APP_TOOLS,
		asks: "Before running an app's events or pages, unless you allowed it.",
		note: "Only the apps you attach. You can narrow what it may do in each app.",
	},
	code_execution: {
		tools: ["code_run"],
		asks: "When the code needs to reach a host on the network.",
		note: "Python 3.12, scientific Python (numpy, pandas, scikit-learn and more) or JavaScript in a WebAssembly sandbox. The network is off unless you approve named hosts.",
	},
	computer_use: {
		tools: [
			"computer_screenshot",
			"computer_act",
			"computer_wait",
			"computer_windows",
			"computer_launch",
		],
		asks: "Before it takes the screen and before it launches an app.",
		note: "Watch, take over or stop it from the bot's Computer tab.",
	},
	browser_use: {
		tools: [
			"browser_open",
			"browser_snapshot",
			"browser_act",
			"browser_screenshot",
			"browser_close",
		],
		asks: "Before it opens a new site.",
		note: "A real browser on the bot's desktop or device. Watch or take over from the Computer tab.",
	},
	email: {
		tools: ["email_send", "email_reply"],
		asks: "Before it mails a new recipient domain. Always allow on replies turns on automatic replies.",
		note: "Its own flow-like.me address. Mail it receives wakes it in your 1:1 chat with the bot.",
	},
	push_notifications: {
		tools: ["message_owner"],
		note: "Push notifications to your phone and desktop.",
	},
	scheduling: {
		tools: ["schedule_task", "list_tasks", "update_task", "cancel_task"],
		note: "One-time runs and cron schedules, at most every 5 minutes, 50 tasks per bot.",
	},
	storage: {
		tools: ["file_list", "file_read", "file_save", "file_write"],
		note: "Files in the bot's own workspace, stored like app storage.",
	},
	data_studio: {
		tools: ["database_tool"],
		asks: "Before it deletes a table.",
		note: "Tables in the bot's own workspace, queried like an app's Data Studio.",
	},
	memory: {
		tools: [
			"memory_save",
			"memory_search",
			"memory_forget",
			"history_search",
			"history_read",
		],
		note: "Needs an embedding model. Review and forget single facts in the Memory tab.",
	},
	documents: {
		tools: [
			"file_summarize",
			"file_extract",
			"file_extract_content",
			"pdf_render",
			"excel_inspect",
			"document_create",
			"document_fill",
		],
		asks: "Before processing very large documents.",
		note: "Creates Word, Excel, PowerPoint and PDF files in its workspace.",
	},
	barcodes: {
		tools: ["barcode_read"],
		note: "QR, Data Matrix, Aztec, PDF417, EAN/UPC, Code 128/39/93, ITF, Codabar and GS1 DataBar. A quick search that finds nothing is retried more thoroughly. Photos above 16 megapixels are read scaled down.",
	},
	data_connections: {
		tools: ["data_sources", "data_query"],
		asks: "Before it attaches a new database.",
		note: "Postgres, MySQL, ODBC and other external databases.",
	},
	knowledge: {
		tools: [
			"knowledge_create",
			"knowledge_add",
			"knowledge_search",
			"knowledge_list",
			"knowledge_drop",
		],
		asks: "Before it drops an index.",
		note: "Searchable indexes over your files.",
	},
	connectors: {
		tools: ["connector_tools", "connector_call", "connector_request"],
		asks: "Per connector: never, ask, ask every time, or always allow reads.",
		note: "Your connected services and MCP servers.",
	},
	channels: {
		tools: ["list_people", "message_person"],
		asks: "Before its first message to each person.",
		note: "Messages people you chat with. Slack, Teams, Telegram and Discord come later.",
	},
	skills: {
		tools: ["skill_load", "skill_call", "skill_run", "skill_propose"],
		asks: "Before it runs a skill's code.",
		note: "It only sees skills you approved. It can propose new ones for you to review.",
	},
	image_generation: {
		tools: ["image_generate"],
		note: "Uses the bot's image model. Up to 4 images per call and 50 per day.",
	},
	delegation: {
		tools: ["spawn_subtasks"],
		note: "Parallel workers on the subtask model. Workers can't ask you, schedule, send mail or message people.",
	},
	command_line: {
		tools: ["shell_run"],
		asks: "Before each new program it runs.",
		note: "Secrets it passes in are scrubbed from the output.",
	},
	encryption: {
		tools: [],
		note: "No tools. The bot's host keeps its private key, so only you and that host can read its messages and secrets.",
	},
	webhook: {
		tools: [],
		note: "No tools. Lets you create webhook URLs that wake the bot.",
	},
	user_context: {
		tools: [
			"user_context_available",
			"user_context_request",
			"user_context_set_status",
		],
		note: "The person chooses what to share: location, preferences or what they are doing.",
	},
	flowpilot: {
		tools: ["FlowPilot's full toolset"],
		note: "Builds, edits and debugs apps and flows, FlowScript included. Only Flux holds it, and it can't be switched.",
	},
};
