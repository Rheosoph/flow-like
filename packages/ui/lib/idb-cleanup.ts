import { flowpilotDB } from "./flowpilot-db";
import { chatDb } from "../components/interfaces/chat-default/chat-db";
import { temporaryFilesDb } from "../db/temporary-files-db";
import { viewportDb } from "../db/viewport-db";
import { offlineSyncDB } from "./sync-db";

const DAY_MS = 24 * 60 * 60 * 1000;

interface CleanupOptions {
	chatMessageMaxAgeDays?: number;
	flowpilotMaxAgeDays?: number;
	tempFilesMaxAgeDays?: number;
	viewportMaxAgeDays?: number;
	offlineSyncMaxAgeDays?: number;
}

const DEFAULTS: Required<CleanupOptions> = {
	chatMessageMaxAgeDays: 30,
	flowpilotMaxAgeDays: 30,
	tempFilesMaxAgeDays: 7,
	viewportMaxAgeDays: 30,
	offlineSyncMaxAgeDays: 7,
};

async function pruneOldChatMessages(maxAgeDays: number): Promise<number> {
	const cutoff = Date.now() - maxAgeDays * DAY_MS;
	const old = await chatDb.messages.filter((m) => m.timestamp < cutoff).primaryKeys();
	if (old.length > 0) await chatDb.messages.bulkDelete(old);

	// Clean up sessions that have no remaining messages
	const allSessions = await chatDb.sessions.toArray();
	const orphanIds: string[] = [];
	for (const session of allSessions) {
		const count = await chatDb.messages.where("sessionId").equals(session.id).count();
		if (count === 0) orphanIds.push(session.id);
	}
	if (orphanIds.length > 0) await chatDb.sessions.bulkDelete(orphanIds);

	return old.length + orphanIds.length;
}

async function pruneOldFlowpilotHistory(maxAgeDays: number): Promise<number> {
	const cutoff = new Date(Date.now() - maxAgeDays * DAY_MS).toISOString();
	const oldConversations = await flowpilotDB.conversations
		.filter((c) => c.updatedAt < cutoff)
		.primaryKeys();

	let deletedMessages = 0;
	for (const convId of oldConversations) {
		const msgs = await flowpilotDB.messages
			.where("conversationId")
			.equals(convId)
			.primaryKeys();
		if (msgs.length > 0) await flowpilotDB.messages.bulkDelete(msgs);
		deletedMessages += msgs.length;
	}
	if (oldConversations.length > 0)
		await flowpilotDB.conversations.bulkDelete(oldConversations);

	return oldConversations.length + deletedMessages;
}

async function pruneOldTempFiles(maxAgeDays: number): Promise<number> {
	const cutoff = Date.now() - maxAgeDays * DAY_MS;
	const old = await temporaryFilesDb.temporaryFiles
		.filter((f) => f.createdAt < cutoff)
		.primaryKeys();
	if (old.length > 0) await temporaryFilesDb.temporaryFiles.bulkDelete(old);
	return old.length;
}

async function pruneOldViewports(maxAgeDays: number): Promise<number> {
	const cutoff = Date.now() - maxAgeDays * DAY_MS;
	const old = await viewportDb.viewports
		.filter((v) => v.updatedAt < cutoff)
		.primaryKeys();
	if (old.length > 0) await viewportDb.viewports.bulkDelete(old);
	return old.length;
}

async function pruneOldOfflineSync(maxAgeDays: number): Promise<number> {
	const cutoff = new Date(Date.now() - maxAgeDays * DAY_MS);
	const old = await offlineSyncDB.commands
		.filter((c) => c.createdAt < cutoff)
		.primaryKeys();
	if (old.length > 0) await offlineSyncDB.commands.bulkDelete(old);
	return old.length;
}

/**
 * Run periodic cleanup of all IndexedDB stores to prevent unbounded growth.
 * Safe to call on every app startup — operations are idempotent and fast
 * when there is nothing to prune.
 */
export async function runIDBCleanup(
	options: CleanupOptions = {},
): Promise<void> {
	const opts = { ...DEFAULTS, ...options };

	const results = await Promise.allSettled([
		pruneOldChatMessages(opts.chatMessageMaxAgeDays),
		pruneOldFlowpilotHistory(opts.flowpilotMaxAgeDays),
		pruneOldTempFiles(opts.tempFilesMaxAgeDays),
		pruneOldViewports(opts.viewportMaxAgeDays),
		pruneOldOfflineSync(opts.offlineSyncMaxAgeDays),
	]);

	for (const result of results) {
		if (result.status === "rejected") {
			console.warn("[IDB Cleanup] partial failure:", result.reason);
		}
	}
}
