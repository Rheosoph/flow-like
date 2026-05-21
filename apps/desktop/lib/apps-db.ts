import type { IAppVisibility } from "@flow-like/flow-like-ui";
import Dexie, { type EntityTable } from "dexie";

export interface IVisibilityStatus {
	appId: string;
	visibility: IAppVisibility;
}

export interface IShortcut {
	id: string;
	profileId: string;
	label: string;
	path: string;
	appId?: string;
	icon?: string;
	order: number;
	createdAt: string;
}

const appsDB = new Dexie("Apps") as Dexie & {
	visibility: EntityTable<IVisibilityStatus, "appId">;
	shortcuts: EntityTable<IShortcut, "id">;
};

appsDB.version(1).stores({
	visibility: "appId",
});

appsDB.version(2).stores({
	visibility: "appId",
	shortcuts: "id, profileId, order",
});

appsDB.version(3).stores({
	visibility: "appId",
	shortcuts: "id, profileId, appId, order",
});

export { appsDB };
