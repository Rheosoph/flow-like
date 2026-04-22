import type { UnifiedChatMessage } from "../../lib/schema/copilot";
import type { AgentMode, CopilotMessage } from "./types";

interface HistoryBudgetOptions {
	agentMode: AgentMode;
	messages: CopilotMessage[];
	selectedNodeIds: string[];
	selectedComponentIds: string[];
	boardId?: string;
	boardName?: string;
	currentComponentsCount: number;
	runContext?: {
		run_id: string;
		app_id: string;
		board_id: string;
		event_id?: string;
	};
	maxRecentMessages?: number;
	maxTotalChars?: number;
}

const DEFAULT_MAX_RECENT_MESSAGES = 8;
const DEFAULT_MAX_TOTAL_CHARS = 9000;
const MAX_MESSAGE_CHARS = 1200;
const MAX_OLDER_BULLETS = 8;
const MAX_BULLET_CHARS = 220;

function clipText(value: string, maxChars: number): string {
	const normalized = value.replace(/\s+/g, " ").trim();
	if (normalized.length <= maxChars) {
		return normalized;
	}
	return `${normalized.slice(0, maxChars - 1).trimEnd()}…`;
}

function summarizeAcceptedWork(messages: CopilotMessage[]): string[] {
	const commandSummaries = messages
		.flatMap((message) => message.executedCommands ?? [])
		.slice(-6)
		.map((command) => {
			switch (command.command_type) {
				case "AddNode":
					return `Added node ${command.node_type}`;
				case "ConnectPins":
					return `Connected ${command.from_node}.${command.from_pin} to ${command.to_node}.${command.to_pin}`;
				case "UpdateNodePin":
					return `Configured ${command.node_id}.${command.pin_id}`;
				case "AddPlaceholder":
					return `Added placeholder ${command.name}`;
				case "CreateComment":
					return "Added workflow comment";
				case "CreateLayer":
					return `Created layer ${command.name}`;
				case "RemoveNode":
					return `Removed node ${command.node_id}`;
				default:
					return command.summary || command.command_type;
			}
		});

	const componentSummaries = messages
		.flatMap((message) => message.appliedComponents ?? [])
		.slice(-6)
		.map((component) => {
			const componentType = component.component?.type ?? "component";
			return `Applied ${componentType} component ${component.id}`;
		});

	return [...commandSummaries, ...componentSummaries].slice(-8);
}

function summarizeOlderMessages(messages: CopilotMessage[]): string[] {
	return messages.slice(-MAX_OLDER_BULLETS).map((message) => {
		const prefix = message.role === "user" ? "User" : "Assistant";
		const artifacts: string[] = [];
		if ((message.executedCommands?.length ?? 0) > 0) {
			artifacts.push(
				`${message.executedCommands?.length ?? 0} workflow changes applied`,
			);
		}
		if ((message.appliedComponents?.length ?? 0) > 0) {
			artifacts.push(
				`${message.appliedComponents?.length ?? 0} UI components applied`,
			);
		}
		if ((message.images?.length ?? 0) > 0) {
			artifacts.push(`${message.images?.length ?? 0} image attachments`);
		}

		const suffix = artifacts.length > 0 ? ` (${artifacts.join(", ")})` : "";
		return `- ${prefix}: ${clipText(message.content, MAX_BULLET_CHARS)}${suffix}`;
	});
}

function buildStructuredSummary(options: HistoryBudgetOptions): string | null {
	const maxRecentMessages =
		options.maxRecentMessages ?? DEFAULT_MAX_RECENT_MESSAGES;
	const olderMessages = options.messages.slice(
		0,
		Math.max(0, options.messages.length - maxRecentMessages),
	);
	const acceptedWork = summarizeAcceptedWork(options.messages);
	const sections: string[] = [];

	const focusLines: string[] = [];
	focusLines.push(`Mode: ${options.agentMode}`);
	if (options.boardId) {
		focusLines.push(
			`Board: ${options.boardName ? `${options.boardName} (${options.boardId})` : options.boardId}`,
		);
	}
	if (options.selectedNodeIds.length > 0) {
		focusLines.push(`Selected nodes: ${options.selectedNodeIds.join(", ")}`);
	}
	if (options.selectedComponentIds.length > 0) {
		focusLines.push(
			`Selected components: ${options.selectedComponentIds.join(", ")}`,
		);
	}
	if (options.currentComponentsCount > 0 && options.agentMode !== "board") {
		focusLines.push(
			`Current UI component count: ${options.currentComponentsCount}`,
		);
	}
	if (options.runContext) {
		focusLines.push(
			`Run context: run ${options.runContext.run_id}, app ${options.runContext.app_id}, board ${options.runContext.board_id}`,
		);
	}
	sections.push(`## Current Focus\n${focusLines.join("\n")}`);

	if (acceptedWork.length > 0) {
		sections.push(
			`## Accepted Changes\n${acceptedWork.map((line) => `- ${line}`).join("\n")}`,
		);
	}

	if (olderMessages.length > 0) {
		sections.push(
			`## Older Conversation Summary\n${summarizeOlderMessages(olderMessages).join("\n")}`,
		);
	}

	const summary = sections.join("\n\n").trim();
	return summary.length > 0 ? summary : null;
}

function sanitizeRecentMessage(message: CopilotMessage): UnifiedChatMessage {
	return {
		role: message.role === "user" ? "User" : "Assistant",
		content: clipText(message.content, MAX_MESSAGE_CHARS),
		images: message.images?.map((image) => ({
			data: image.data,
			media_type: image.mediaType,
		})),
	};
}

export function buildBudgetedHistory(
	options: HistoryBudgetOptions,
): UnifiedChatMessage[] {
	const maxRecentMessages =
		options.maxRecentMessages ?? DEFAULT_MAX_RECENT_MESSAGES;
	const maxTotalChars = options.maxTotalChars ?? DEFAULT_MAX_TOTAL_CHARS;

	const recentMessages = options.messages
		.slice(-maxRecentMessages)
		.map(sanitizeRecentMessage);
	let history: UnifiedChatMessage[] = recentMessages;

	const summary = buildStructuredSummary({
		...options,
		maxRecentMessages,
	});
	if (summary) {
		history = [{ role: "Assistant", content: summary }, ...recentMessages];
	}

	while (
		history.length > 1 &&
		history.reduce((sum, message) => sum + message.content.length, 0) >
			maxTotalChars
	) {
		history = [history[0], ...history.slice(2)];
	}

	return history;
}
