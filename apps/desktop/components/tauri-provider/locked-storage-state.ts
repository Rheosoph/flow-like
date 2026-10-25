import {
	LockedStorageClient,
	lockedStoragePath,
} from "@flow-like/flow-like-ui/lib/locked-storage";
import {
	type ILockedStorageStatus,
	LOCKED_STORAGE_DISABLED,
} from "@flow-like/flow-like-ui/state/backend-state/locked-storage-state";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export const LOCKED_STORAGE_OFFLINE_ERROR =
	"Locked files are only available for online apps";

/** Locked files live in the hub's locked bucket only, so offline apps never have any. */
export class LockedStorageState extends LockedStorageClient {
	constructor(private readonly backend: TauriBackend) {
		super(async (appId, action, { method, body, signal }) => {
			if (await backend.isOffline(appId)) {
				throw new Error(LOCKED_STORAGE_OFFLINE_ERROR);
			}
			const { profile, auth } = backend;
			if (!profile || !auth) {
				throw new Error(
					`Backend is not properly initialized for locked files (${action}).`,
				);
			}
			return fetcher(
				profile,
				lockedStoragePath(appId, action),
				{
					method,
					body: body === undefined ? undefined : JSON.stringify(body),
					signal,
				},
				auth,
			);
		});
	}

	override async getLockedStorageStatus(
		appId: string,
	): Promise<ILockedStorageStatus> {
		if (await this.backend.isOffline(appId)) {
			return { ...LOCKED_STORAGE_DISABLED };
		}
		return super.getLockedStorageStatus(appId);
	}
}
