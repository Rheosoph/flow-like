import {
	CircleDotIcon,
	EditIcon,
	LayoutGridIcon,
	LinkIcon,
	MessageSquareIcon,
	MoveIcon,
	PencilIcon,
	PlusCircleIcon,
	SquarePenIcon,
	Unlink2Icon,
	XCircleIcon,
} from "lucide-react";

import type { BoardCommand } from "../../lib/schema/flow/copilot";
import type { SurfaceComponent } from "../a2ui/types";

// === Board Command Utilities ===

export function getCommandIcon(cmd: BoardCommand, size = "w-4 h-4") {
	switch (cmd.command_type) {
		case "AddNode":
			return <PlusCircleIcon className={size} />;
		case "RemoveNode":
			return <XCircleIcon className={size} />;
		case "ConnectPins":
			return <LinkIcon className={size} />;
		case "DisconnectPins":
			return <Unlink2Icon className={size} />;
		case "UpdateNodePin":
			return <PencilIcon className={size} />;
		case "MoveNode":
			return <MoveIcon className={size} />;
		case "CreateVariable":
		case "UpdateVariable":
		case "DeleteVariable":
			return <EditIcon className={size} />;
		case "CreateComment":
		case "UpdateComment":
		case "DeleteComment":
			return <MessageSquareIcon className={size} />;
		case "CreateLayer":
		case "UpdateLayerCache":
		case "AddNodesToLayer":
		case "RemoveNodesFromLayer":
			return <SquarePenIcon className={size} />;
		default:
			return <CircleDotIcon className={size} />;
	}
}

export function getCommandSummary(cmd: BoardCommand): string {
	if (cmd.summary) return cmd.summary;
	switch (cmd.command_type) {
		case "AddNode":
			return `Add ${cmd.friendly_name || cmd.node_type.split("::").pop() || "node"}`;
		case "RemoveNode":
			return "Remove node";
		case "ConnectPins":
			return `Connect ${cmd.from_pin} → ${cmd.to_pin}`;
		case "DisconnectPins":
			return "Disconnect pins";
		case "UpdateNodePin":
			return `Set ${cmd.pin_id}`;
		case "MoveNode":
			return "Move node";
		case "UpdateLayerCache":
			return cmd.cache?.enabled
				? "Configure function cache"
				: "Disable function cache";
		default:
			return cmd.command_type.replace(/([A-Z])/g, " $1").trim();
	}
}

export function getCommandColor(cmd: BoardCommand): string {
	switch (cmd.command_type) {
		case "AddNode":
			return "from-green-500/20 to-emerald-500/10 border-green-500/30";
		case "RemoveNode":
			return "from-red-500/20 to-rose-500/10 border-red-500/30";
		case "ConnectPins":
			return "from-blue-500/20 to-cyan-500/10 border-blue-500/30";
		case "DisconnectPins":
			return "from-orange-500/20 to-amber-500/10 border-orange-500/30";
		case "UpdateNodePin":
			return "from-violet-500/20 to-purple-500/10 border-violet-500/30";
		default:
			return "from-primary/20 to-primary/10 border-primary/30";
	}
}

// === UI Component Utilities ===

export function getComponentIcon(size = "w-4 h-4") {
	return <LayoutGridIcon className={size} />;
}

export function getComponentSummary(component: SurfaceComponent): string {
	const type = component.component?.type ?? "Unknown";
	return type;
}

export function getComponentCounts(
	components: SurfaceComponent[],
): Record<string, number> {
	const counts: Record<string, number> = {};
	for (const comp of components) {
		const type = comp.component?.type ?? "Unknown";
		counts[type] = (counts[type] || 0) + 1;
	}
	return counts;
}

// === Smart Mode Detection ===

/** Keywords that indicate a workflow/board-related request */
const BOARD_KEYWORDS = [
	"node",
	"nodes",
	"workflow",
	"flow",
	"pipeline",
	"api",
	"endpoint",
	"connect",
	"connection",
	"pin",
	"pins",
	"trigger",
	"execute",
	"run",
	"loop",
	"condition",
	"branch",
	"variable",
	"data",
	"transform",
	"fetch",
	"http",
	"request",
	"response",
	"database",
	"query",
	"llm",
	"ai",
	"model",
	"embedding",
	"inference",
	"automation",
	"schedule",
	"cron",
	"webhook",
	"event",
	"listener",
	"handler",
];

/** Keywords that indicate a UI-related request */
const UI_KEYWORDS = [
	"ui",
	"interface",
	"component",
	"components",
	"widget",
	"widgets",
	"button",
	"form",
	"input",
	"text",
	"image",
	"icon",
	"card",
	"table",
	"list",
	"grid",
	"row",
	"column",
	"stack",
	"layout",
	"design",
	"style",
	"color",
	"size",
	"padding",
	"margin",
	"border",
	"shadow",
	"modal",
	"dialog",
	"popup",
	"navbar",
	"header",
	"footer",
	"sidebar",
	"dashboard",
	"chart",
	"graph",
	"display",
	"view",
	"screen",
	"page",
];

/** Keywords that indicate both workflow and UI together */
const UNIFIED_KEYWORDS = [
	"dashboard with",
	"ui with workflow",
	"interface with api",
	"display data from",
	"show results of",
	"build app",
	"create application",
	"full stack",
	"end to end",
	"frontend and backend",
	"ui that triggers",
	"button that runs",
];

/**
 * Analyzes a prompt to detect whether it's about workflows, UI, or both.
 * Returns the recommended agent mode.
 */
export function detectAgentMode(prompt: string): "board" | "ui" | "both" {
	const lowerPrompt = prompt.toLowerCase();

	// Check for unified keywords first (highest priority)
	for (const keyword of UNIFIED_KEYWORDS) {
		if (lowerPrompt.includes(keyword)) {
			return "both";
		}
	}

	// Count keyword matches
	let boardScore = 0;
	let uiScore = 0;

	for (const keyword of BOARD_KEYWORDS) {
		if (lowerPrompt.includes(keyword)) {
			boardScore++;
		}
	}

	for (const keyword of UI_KEYWORDS) {
		if (lowerPrompt.includes(keyword)) {
			uiScore++;
		}
	}

	// If both have significant matches, use both mode
	if (boardScore >= 2 && uiScore >= 2) {
		return "both";
	}

	// Otherwise, use the mode with more matches
	if (boardScore > uiScore) {
		return "board";
	}
	if (uiScore > boardScore) {
		return "ui";
	}

	// Default to "both" if no clear signal (let AI decide)
	return "both";
}

// === Tool Result Compaction ===

/** Cap a JSON-serializable value for tool results so huge payloads don't flood the model. */
export function compactJson(value: unknown, maxChars = 12_000): unknown {
	try {
		const text = JSON.stringify(value);
		if (text.length <= maxChars) return value;
		return {
			truncated: true,
			chars: text.length,
			preview: text.slice(0, maxChars),
		};
	} catch {
		return String(value);
	}
}

/** Keep only the tail of an intercom event stream, each event size-capped. */
export function compactLogEvents(events: unknown[], maxEvents = 80): unknown[] {
	return events.slice(-maxEvents).map((event) => {
		if (!event || typeof event !== "object") return event;
		const object = event as Record<string, unknown>;
		return {
			event_type: object.event_type,
			payload: compactJson(object.payload, 3000),
		};
	});
}
