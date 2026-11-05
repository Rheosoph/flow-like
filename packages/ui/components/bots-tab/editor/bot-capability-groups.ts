import {
	AppWindow,
	BellRing,
	Brain,
	CalendarClock,
	CodeXml,
	Compass,
	Database,
	FileText,
	FolderOpen,
	Globe,
	ImagePlus,
	LayoutGrid,
	LibraryBig,
	LockKeyhole,
	Mail,
	MessagesSquare,
	MonitorSmartphone,
	Network,
	Plug,
	Sparkles,
	Split,
	SquareTerminal,
	Table2,
	Webhook,
	Workflow,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { BotCapability } from "../../../state/backend-state/bot-state";

export type BotCapabilityGroupId =
	| "web"
	| "apps"
	| "hands"
	| "talk"
	| "memory"
	| "security";

export const BOT_CAPABILITY_GROUPS: readonly {
	readonly id: BotCapabilityGroupId;
	readonly capabilities: readonly BotCapability[];
}[] = [
	{ id: "web", capabilities: ["internet", "web_requests"] },
	{ id: "apps", capabilities: ["profile_apps", "selected_apps"] },
	{
		id: "hands",
		capabilities: [
			"code_execution",
			"skills",
			"image_generation",
			"delegation",
			"computer_use",
			"browser_use",
			"command_line",
		],
	},
	{
		id: "talk",
		capabilities: [
			"email",
			"channels",
			"connectors",
			"push_notifications",
			"webhook",
		],
	},
	{
		id: "memory",
		capabilities: [
			"memory",
			"storage",
			"documents",
			"knowledge",
			"data_studio",
			"data_connections",
			"scheduling",
		],
	},
	{ id: "security", capabilities: ["encryption"] },
];

export const BOT_CAPABILITY_ICONS: Readonly<Record<BotCapability, LucideIcon>> =
	{
		internet: Globe,
		web_requests: Network,
		profile_apps: LayoutGrid,
		selected_apps: AppWindow,
		code_execution: CodeXml,
		computer_use: MonitorSmartphone,
		browser_use: Compass,
		email: Mail,
		push_notifications: BellRing,
		scheduling: CalendarClock,
		memory: Brain,
		documents: FileText,
		data_connections: Database,
		knowledge: LibraryBig,
		connectors: Plug,
		channels: MessagesSquare,
		skills: Sparkles,
		image_generation: ImagePlus,
		delegation: Split,
		storage: FolderOpen,
		data_studio: Table2,
		command_line: SquareTerminal,
		encryption: LockKeyhole,
		webhook: Webhook,
		flowpilot: Workflow,
	};
