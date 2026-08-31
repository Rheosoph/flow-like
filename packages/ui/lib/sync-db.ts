import type { IGenericCommand } from "@flow-like/flow-like-ui";
import Dexie, { type EntityTable } from "dexie";

export interface ICommandSync {
	commandId: string;
	appId: string;
	boardId: string;
	/** Legacy v1 payload; migrated to exact chunks on the first retry. */
	commands?: IGenericCommand[];
	/** Exact serialized batching chosen before the first remote attempt. Never rechunk on retry. */
	chunks?: IGenericCommand[][];
	createdAt: Date;
	/** Stable remote mutation id; replays replace/retain one outbox entry instead of duplicating it. */
	idempotencyKey?: string;
	/** Original chunk index when only an undelivered tail was queued. */
	chunkOffset?: number;
	/** Durable insertion order. Legacy rows fall back to `createdAt`. */
	sequence?: number;
	/** Exact post-commit payload retained when it cannot fit the command transport. */
	blockedReason?: string;
	/** Immutable remote destination/owner captured when this mutation entered the outbox. */
	remoteIdentityVersion?: 1;
	remoteProfileId?: string;
	remotePrincipalId?: string;
	remoteHub?: string;
	/** Server receipt to ACK before this row may advance or disappear. */
	pendingReceiptAck?: string;
	/**
	 * Why the last delivery attempt failed. A queued mutation is never dropped, so this is the
	 * only way a permanently rejected batch can be told apart from an offline one — without it a
	 * poisoned batch wedges the whole board queue behind a generic "sync incomplete" toast.
	 */
	lastFailureStatus?: number;
	lastFailureMessage?: string;
	lastFailureAt?: Date;
	failedAttempts?: number;
	/** FlowPilot receipts remain server-durable while this tombstone suppresses native replay. */
	deferReceiptAckUntilNativeTerminal?: boolean;
	deferredReceiptAcks?: string[];
}

/**
 * A queued mutation removed by an explicit server-authoritative reset.
 *
 * The reset is the only path that drops an undelivered edit, so the exact row is retained here
 * instead of being deleted outright: the user can still export what was discarded, and a support
 * case can reconstruct it. Nothing replays from this store.
 */
export interface ICommandSyncArchive extends ICommandSync {
	archiveId: string;
	archivedAt: Date;
	archiveReason: string;
}

const offlineSyncDB = new Dexie("OfflineSync") as Dexie & {
	commands: EntityTable<ICommandSync, "commandId">;
	discarded: EntityTable<ICommandSyncArchive, "archiveId">;
};

offlineSyncDB.version(1).stores({
	commands: "commandId, appId, [appId+boardId]",
});

offlineSyncDB.version(2).stores({
	commands: "commandId, appId, [appId+boardId]",
});

offlineSyncDB.version(3).stores({
	commands: "commandId, appId, [appId+boardId], sequence",
});

offlineSyncDB.version(4).stores({
	commands: "commandId, appId, [appId+boardId], sequence",
	discarded: "archiveId, appId, [appId+boardId], archivedAt",
});

/**
 * Retire every queued mutation for an app that has left this device.
 *
 * Age-based cleanup deliberately never reclaims a row that still carries
 * commands, so rows for an app that was deleted or quit would otherwise sit in
 * the outbox forever: their board no longer exists locally, nothing reopens it,
 * and the hub would answer 403 if anything tried. They are archived rather than
 * dropped, so an edit that never reached the server is still recoverable.
 */
export async function discardOfflineSyncForApp(
	appId: string,
	archiveReason: string,
): Promise<number> {
	const archivedAt = new Date();
	let archived = 0;
	await offlineSyncDB.transaction(
		"rw",
		offlineSyncDB.commands,
		offlineSyncDB.discarded,
		async () => {
			const queued = await offlineSyncDB.commands
				.where("appId")
				.equals(appId)
				.toArray();
			for (const entry of queued) {
				await offlineSyncDB.discarded.put({
					...entry,
					archiveId: `${entry.commandId}${archivedAt.getTime()}`,
					archivedAt,
					archiveReason,
				});
				await offlineSyncDB.commands.delete(entry.commandId);
				archived += 1;
			}
		},
	);
	return archived;
}

export { offlineSyncDB };
