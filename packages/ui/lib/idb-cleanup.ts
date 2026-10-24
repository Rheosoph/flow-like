import { chatDb } from "../components/interfaces/chat-default/chat-db";
import { boardLastSeenDb } from "../db/board-last-seen-db";
import { temporaryFilesDb } from "../db/temporary-files-db";
import { viewportDb } from "../db/viewport-db";
import { globalChatDb } from "../state/global-chat/global-chat-db";
import {
	RETENTION_KINDS,
	type RetentionKind,
	type StorageRetentionPolicy,
	readRetentionPolicy,
	retentionCutoff,
	writeLastRetentionRun,
} from "./browser-storage/retention";
import { pruneBoardHistories } from "./flow-history-db";
import { flowpilotDB } from "./flowpilot-db";
import { type ICommandSync, offlineSyncDB } from "./sync-db";

const OFFLINE_SYNC_MAX_AGE_DAYS = 7;

const ALL = Number.POSITIVE_INFINITY;

/** Unreadable timestamps come out as NaN, which is never older than a cutoff. */
const timeOf = (value: Date | string | number) => new Date(value).getTime();

async function pruneOldChatMessages(cutoff: number): Promise<number> {
	// A pinned conversation is an explicit "keep this" from the user, so neither its messages nor
	// its session row may be aged out — otherwise pinning quietly guarantees deletion instead.
	const pinnedSessionIds = new Set(
		(await chatDb.sessions.where("pinnedAt").above(0).primaryKeys()).map(
			String,
		),
	);
	const isStale = (row: { timestamp: number; sessionId: string }) =>
		row.timestamp < cutoff && !pinnedSessionIds.has(row.sessionId);
	const old = await chatDb.messages.filter(isStale).primaryKeys();
	if (old.length > 0) await chatDb.messages.bulkDelete(old);
	const oldDrafts = await chatDb.drafts.filter(isStale).primaryKeys();
	if (oldDrafts.length > 0) await chatDb.drafts.bulkDelete(oldDrafts);

	const allSessions = await chatDb.sessions.toArray();
	const orphanIds: string[] = [];
	for (const session of allSessions) {
		if (session.pinnedAt) continue;
		const count = await chatDb.messages
			.where("sessionId")
			.equals(session.id)
			.count();
		if (count === 0) orphanIds.push(session.id);
	}
	if (orphanIds.length > 0) await chatDb.sessions.bulkDelete(orphanIds);

	return old.length + oldDrafts.length + orphanIds.length;
}

/** Whole conversations, by last activity: a global chat is one thread, not loose messages. */
async function pruneOldGlobalChat(cutoff: number): Promise<number> {
	const stale = await globalChatDb.sessions
		.filter((session) => session.updatedAt < cutoff && !session.pinnedAt)
		.primaryKeys();
	if (stale.length === 0) return 0;
	let deletedMessages = 0;
	await globalChatDb.transaction(
		"rw",
		globalChatDb.sessions,
		globalChatDb.messages,
		async () => {
			for (const sessionId of stale) {
				deletedMessages += await globalChatDb.messages
					.where("sessionId")
					.equals(sessionId)
					.delete();
			}
			await globalChatDb.sessions.bulkDelete(stale);
		},
	);
	return stale.length + deletedMessages;
}

async function pruneOldFlowpilotHistory(cutoff: number): Promise<number> {
	const oldConversations = await flowpilotDB.conversations
		.filter((c) => timeOf(c.updatedAt) < cutoff && !c.pinnedAt)
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

async function pruneOldTempFiles(cutoff: number): Promise<number> {
	const old = await temporaryFilesDb.temporaryFiles
		.filter((f) => f.createdAt < cutoff)
		.primaryKeys();
	if (old.length > 0) await temporaryFilesDb.temporaryFiles.bulkDelete(old);
	return old.length;
}

async function pruneOldViewports(cutoff: number): Promise<number> {
	const old = await viewportDb.viewports
		.filter((v) => v.updatedAt < cutoff)
		.primaryKeys();
	if (old.length > 0) await viewportDb.viewports.bulkDelete(old);
	return old.length;
}

/** Rows hold whole boards, so prune through the index instead of decoding each one. */
async function pruneOldBoardLastSeen(cutoff: number): Promise<number> {
	const old = await boardLastSeenDb.seen
		.where("updatedAt")
		.below(cutoff)
		.primaryKeys();
	if (old.length > 0) await boardLastSeenDb.seen.bulkDelete(old);
	return old.length;
}

const holdsCommands = (c: ICommandSync) =>
	(c.commands?.length ?? 0) > 0 || (c.chunks?.length ?? 0) > 0;

const awaitsReceipt = (c: ICommandSync) =>
	!!c.pendingReceiptAck || (c.deferredReceiptAcks?.length ?? 0) > 0;

/** Rows still carrying work, or a FlowPilot edit's identity, are never aged out. */
const isSettledSyncRow = (c: ICommandSync) =>
	!holdsCommands(c) &&
	!awaitsReceipt(c) &&
	!c.blockedReason &&
	!c.idempotencyKey?.startsWith("flowpilot-board-edit:");

async function pruneOldOfflineSync(cutoff: number): Promise<number> {
	const isStale = (c: ICommandSync) =>
		timeOf(c.createdAt) < cutoff && isSettledSyncRow(c);
	const old = await offlineSyncDB.commands.filter(isStale).primaryKeys();
	if (old.length > 0) await offlineSyncDB.commands.bulkDelete(old);
	return old.length;
}

/** Discarded batches keep their full payload, so the archive needs its own ceiling. */
async function pruneOldOfflineSyncArchive(cutoff: number): Promise<number> {
	const old = await offlineSyncDB.discarded
		.filter((entry) => timeOf(entry.archivedAt) < cutoff)
		.primaryKeys();
	if (old.length > 0) await offlineSyncDB.discarded.bulkDelete(old);
	return old.length;
}

const PRUNERS: Readonly<
	Record<RetentionKind, (cutoff: number) => Promise<number>>
> = {
	undoHistory: pruneBoardHistories,
	boardSnapshots: pruneOldBoardLastSeen,
	chats: pruneOldChatMessages,
	globalChat: pruneOldGlobalChat,
	flowpilot: pruneOldFlowpilotHistory,
	discardedSync: pruneOldOfflineSyncArchive,
	temporaryFiles: pruneOldTempFiles,
	viewports: pruneOldViewports,
};

/** Rows removed per kind; a kind that failed or is kept forever is absent. */
export type IDBCleanupResult = Partial<Record<RetentionKind, number>>;

/**
 * Prune every local store by the retention policy (the one saved in Storage settings unless
 * given). Safe to call on every app startup — idempotent and fast when there is nothing to prune.
 */
export async function runIDBCleanup(
	policy: StorageRetentionPolicy = readRetentionPolicy(),
): Promise<IDBCleanupResult> {
	const now = Date.now();
	const kinds = RETENTION_KINDS.filter((kind) => policy[kind] !== null);
	const results = await Promise.allSettled([
		pruneOldOfflineSync(retentionCutoff(OFFLINE_SYNC_MAX_AGE_DAYS, now)),
		...kinds.map((kind) =>
			PRUNERS[kind](retentionCutoff(policy[kind] as number, now)),
		),
	]);

	const removed: IDBCleanupResult = {};
	results.forEach((result, index) => {
		if (result.status === "rejected") {
			console.warn("[IDB Cleanup] partial failure:", result.reason);
			return;
		}
		if (index > 0) removed[kinds[index - 1]] = result.value;
	});
	writeLastRetentionRun(now);
	return removed;
}

/** Prune one kind right away, e.g. after its retention was shortened. */
export function pruneRetentionKind(
	kind: RetentionKind,
	days: number,
): Promise<number> {
	return PRUNERS[kind](retentionCutoff(days));
}

/** Delete every conversation of the kind except pinned ones. */
export function clearConversationHistory(
	kind: Extract<RetentionKind, "chats" | "globalChat" | "flowpilot">,
): Promise<number> {
	return PRUNERS[kind](ALL);
}
