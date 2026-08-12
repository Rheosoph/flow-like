import Dexie, { type EntityTable } from "dexie";
import type { SurfaceComponent } from "../components/a2ui/types";
import type {
	FlowPilotProcessEvent,
	UnifiedPlanStep,
} from "../components/flowpilot/types";
import type { BoardCommand } from "./schema/flow/copilot";

/**
 * Represents an attached image in a message
 */
export interface IFlowPilotImage {
	data: string;
	mediaType: string;
}

/**
 * Represents a single message in a FlowPilot conversation
 */
export interface IFlowPilotMessage {
	id: string;
	conversationId: string;
	role: "user" | "assistant";
	content: string;
	images?: IFlowPilotImage[];
	contextNodeIds?: string[];
	appliedComponents?: SurfaceComponent[];
	executedCommands?: BoardCommand[];
	flowscriptWorkspace?: string;
	/** Process timeline for the turn (tool calls, FlowScript edits, queued changes). */
	processEvents?: FlowPilotProcessEvent[];
	/** Completed plan steps for the turn. */
	planSteps?: UnifiedPlanStep[];
	createdAt: string;
}

/**
 * Represents a FlowPilot conversation session
 */
export interface IFlowPilotConversation {
	id: string;
	/** Display title (first user message or auto-generated) */
	title: string;
	/** Agent mode: board, ui, or both */
	mode: "board" | "ui" | "both";
	/** Associated board ID if in board mode */
	boardId?: string;
	/** Associated app ID */
	appId?: string;
	/** Number of messages */
	messageCount: number;
	/** When the conversation was created */
	createdAt: string;
	/**
	 * ISO timestamp of when the user pinned this conversation; absent means unpinned. A timestamp
	 * rather than a boolean because booleans are not valid IndexedDB keys.
	 */
	pinnedAt?: string;
	/** When the conversation was last updated */
	updatedAt: string;
}

/**
 * Dexie database for FlowPilot history
 */
const flowpilotDB = new Dexie("FlowPilotHistory") as Dexie & {
	conversations: EntityTable<IFlowPilotConversation, "id">;
	messages: EntityTable<IFlowPilotMessage, "id">;
};

flowpilotDB.version(1).stores({
	conversations: "id, mode, boardId, appId, updatedAt",
	messages: "id, conversationId, createdAt",
});

flowpilotDB.version(2).stores({
	conversations: "id, mode, boardId, appId, updatedAt, pinnedAt",
	messages: "id, conversationId, createdAt",
});

/**
 * Create a new conversation
 */
export async function createConversation(
	mode: "board" | "ui" | "both",
	boardId?: string,
	appId?: string,
): Promise<IFlowPilotConversation> {
	const now = new Date().toISOString();
	const conversation: IFlowPilotConversation = {
		id: crypto.randomUUID(),
		title: "New conversation",
		mode,
		boardId,
		appId,
		messageCount: 0,
		createdAt: now,
		updatedAt: now,
	};
	await flowpilotDB.conversations.add(conversation);
	return conversation;
}

/**
 * Update conversation title and updatedAt
 */
export async function updateConversation(
	id: string,
	updates: Partial<Pick<IFlowPilotConversation, "title" | "messageCount">>,
): Promise<void> {
	await flowpilotDB.conversations.update(id, {
		...updates,
		updatedAt: new Date().toISOString(),
	});
}

/**
 * Delete a conversation and all its messages
 */
export async function deleteConversation(id: string): Promise<void> {
	await flowpilotDB.messages.where("conversationId").equals(id).delete();
	await flowpilotDB.conversations.delete(id);
}

/**
 * Get recent conversations, sorted by updatedAt descending.
 *
 * Pinned conversations are fetched separately and merged in front, so a pinned-but-old
 * conversation is never cut by the recency window.
 */
export async function getRecentConversations(
	limit = 20,
	mode?: "board" | "ui" | "both",
): Promise<IFlowPilotConversation[]> {
	const matchesMode = (c: IFlowPilotConversation) => !mode || c.mode === mode;

	const pinned = (
		await flowpilotDB.conversations.where("pinnedAt").above("").toArray()
	)
		.filter(matchesMode)
		.sort((a, b) => (b.pinnedAt ?? "").localeCompare(a.pinnedAt ?? ""));

	const pinnedIds = new Set(pinned.map((c) => c.id));
	const recent = await flowpilotDB.conversations
		.orderBy("updatedAt")
		.reverse()
		.filter((c) => matchesMode(c) && !pinnedIds.has(c.id))
		.limit(limit)
		.toArray();

	return [...pinned, ...recent];
}

/**
 * Pin/unpin a conversation. Writes the row directly rather than going through
 * `updateConversation`, which stamps `updatedAt` — pinning must not reorder history.
 */
export async function setConversationPinned(
	id: string,
	pinned: boolean,
): Promise<void> {
	await flowpilotDB.conversations.update(id, {
		pinnedAt: pinned ? new Date().toISOString() : undefined,
	});
}

/** Rename a conversation without touching `updatedAt`. Empty titles are ignored. */
export async function renameConversation(
	id: string,
	title: string,
): Promise<void> {
	const next = title.trim().slice(0, 80);
	if (!next) return;
	await flowpilotDB.conversations.update(id, { title: next });
}

/**
 * Get a specific conversation
 */
export async function getConversation(
	id: string,
): Promise<IFlowPilotConversation | undefined> {
	return flowpilotDB.conversations.get(id);
}

/**
 * Add a message to a conversation
 */
export async function addMessage(
	conversationId: string,
	message: Omit<IFlowPilotMessage, "id" | "conversationId" | "createdAt">,
): Promise<IFlowPilotMessage> {
	const fullMessage: IFlowPilotMessage = {
		...message,
		id: crypto.randomUUID(),
		conversationId,
		createdAt: new Date().toISOString(),
	};
	await flowpilotDB.messages.add(fullMessage);

	// Update conversation
	const conversation = await flowpilotDB.conversations.get(conversationId);
	if (conversation) {
		const updates: Partial<IFlowPilotConversation> = {
			messageCount: conversation.messageCount + 1,
			updatedAt: new Date().toISOString(),
		};

		// Update title from first user message
		if (
			conversation.messageCount === 0 &&
			message.role === "user" &&
			message.content
		) {
			updates.title =
				message.content.slice(0, 50) +
				(message.content.length > 50 ? "..." : "");
		}

		await flowpilotDB.conversations.update(conversationId, updates);
	}

	return fullMessage;
}

/**
 * Update a message (e.g., update assistant message content as it streams)
 */
export async function updateMessage(
	id: string,
	updates: Partial<
		Pick<
			IFlowPilotMessage,
			| "content"
			| "appliedComponents"
			| "executedCommands"
			| "flowscriptWorkspace"
			| "processEvents"
			| "planSteps"
		>
	>,
): Promise<void> {
	await flowpilotDB.messages.update(id, updates);
}

/**
 * Get all messages for a conversation
 */
export async function getMessages(
	conversationId: string,
): Promise<IFlowPilotMessage[]> {
	return flowpilotDB.messages
		.where("conversationId")
		.equals(conversationId)
		.sortBy("createdAt");
}

/**
 * Clear all history
 */
export async function clearAllHistory(): Promise<void> {
	await flowpilotDB.messages.clear();
	await flowpilotDB.conversations.clear();
}

export { flowpilotDB };
