import Dexie, { type EntityTable } from "dexie";
import type { SurfaceComponent } from "../components/a2ui/types";
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
 * Get recent conversations, sorted by updatedAt descending
 */
export async function getRecentConversations(
	limit = 20,
	mode?: "board" | "ui" | "both",
): Promise<IFlowPilotConversation[]> {
	let query = flowpilotDB.conversations.orderBy("updatedAt").reverse();
	if (mode) {
		query = query.filter((c) => c.mode === mode);
	}
	return query.limit(limit).toArray();
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
