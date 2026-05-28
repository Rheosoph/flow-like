import type { IGenericCommand } from "@flow-like/flow-like-ui";
import Dexie, { type EntityTable } from "dexie";

export interface ICommandSync {
	commandId: string;
	appId: string;
	boardId: string;
	commands: IGenericCommand[];
	createdAt: Date;
}

const offlineSyncDB = new Dexie("OfflineSync") as Dexie & {
	commands: EntityTable<ICommandSync, "commandId">;
};

offlineSyncDB.version(1).stores({
	commands: "commandId, appId, [appId+boardId]",
});

export { offlineSyncDB };
