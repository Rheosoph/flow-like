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
	/** FlowPilot receipts remain server-durable while this tombstone suppresses native replay. */
	deferReceiptAckUntilNativeTerminal?: boolean;
	deferredReceiptAcks?: string[];
}

const offlineSyncDB = new Dexie("OfflineSync") as Dexie & {
	commands: EntityTable<ICommandSync, "commandId">;
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

export { offlineSyncDB };
