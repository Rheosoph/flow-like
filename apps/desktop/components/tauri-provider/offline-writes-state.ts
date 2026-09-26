import type {
	IOfflineAppOverview,
	IOfflineKeepBothResult,
	IOfflineLimits,
	IOfflineMirrorEvent,
	IOfflineOperation,
	IOfflineOperationLookup,
	IOfflineStatusEvent,
	IOfflineTableSelection,
	IOfflineTableState,
	IOfflineTablesChangedEvent,
	IOfflineWritesState,
	OfflineTablePurpose,
	OfflineTableRoute,
} from "@flow-like/flow-like-ui/state/backend-state/offline-writes-state";
import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";
import type { TauriBackend } from "../tauri-provider";

export const OFFLINE_WRITES_EVENTS = {
	status: "offline-writes:status",
	tables: "offline-writes:tables-changed",
	mirror: "offline-writes:mirror",
} as const;

const TABLE_ROUTE_COMMAND = "offline_writes_table_route";
let missingTableRouteCommandReported = false;

/** A build whose Rust side lacks the routing command can only reach tables through the hub. */
function isMissingTableRouteCommand(error: unknown): boolean {
	const message =
		typeof error === "string"
			? error
			: error instanceof Error
				? error.message
				: "";
	return message.includes(TABLE_ROUTE_COMMAND) && /not found/i.test(message);
}

function subscribeEvent<T>(
	name: string,
	listener: (payload: T) => void,
): () => void {
	let unlisten: UnlistenFn | undefined;
	let closed = false;
	listen<T>(name, (event) => listener(event.payload))
		.then((stop) => {
			if (closed) stop();
			else unlisten = stop;
		})
		.catch((error) =>
			console.warn(`[OfflineWrites] Failed to listen to ${name}:`, error),
		);
	return () => {
		closed = true;
		unlisten?.();
		unlisten = undefined;
	};
}

export class OfflineWritesState implements IOfflineWritesState {
	constructor(private readonly backend: TauriBackend) {}

	private get token(): string | undefined {
		return this.backend.auth?.user?.access_token;
	}

	private requireToken(): string {
		const token = this.token;
		if (!token) {
			throw new Error("Sign in to manage offline access for this project.");
		}
		return token;
	}

	getOverview(appId: string): Promise<IOfflineAppOverview> {
		return invoke("offline_writes_overview", {
			appId,
			token: this.token ?? null,
		});
	}

	async setTable(
		appId: string,
		selection: IOfflineTableSelection,
	): Promise<IOfflineTableState> {
		await this.backend.prepareExecutionAuth();
		return invoke("offline_writes_set_table", {
			appId,
			selection,
			token: this.requireToken(),
			sessionId: this.backend.executionSessionId,
		});
	}

	async removeTable(
		appId: string,
		purpose: OfflineTablePurpose,
		table: string,
	): Promise<void> {
		await invoke("offline_writes_remove_table", {
			appId,
			token: this.requireToken(),
			purpose,
			table,
		});
	}

	async setPrefetch(
		appId: string,
		purpose: OfflineTablePurpose,
		table: string,
		prefetch: boolean,
	): Promise<IOfflineTableState> {
		return invoke("offline_writes_set_prefetch", {
			appId,
			token: this.requireToken(),
			purpose,
			table,
			prefetch,
		});
	}

	async setLimits(
		appId: string,
		limits: IOfflineLimits,
	): Promise<IOfflineLimits> {
		return invoke("offline_writes_set_limits", {
			appId,
			token: this.requireToken(),
			limits,
		});
	}

	async listOperations(
		appId: string,
		afterSequence?: number,
		limit?: number,
	): Promise<IOfflineOperation[]> {
		return invoke("offline_writes_operations", {
			appId,
			token: this.requireToken(),
			afterSequence: afterSequence ?? null,
			limit: limit ?? null,
		});
	}

	async getOperationState(
		appId: string,
		operationId: string,
	): Promise<IOfflineOperationLookup> {
		return invoke("offline_writes_operation_state", {
			appId,
			token: this.requireToken(),
			operationId,
		});
	}

	async retryOperation(appId: string, operationId: string): Promise<void> {
		await invoke("offline_writes_retry", {
			appId,
			token: this.requireToken(),
			operationId,
		});
	}

	async skipOperation(
		appId: string,
		operationId: string,
		reason: string,
		acknowledgeUncertain: boolean,
	): Promise<void> {
		await invoke("offline_writes_skip", {
			appId,
			token: this.requireToken(),
			operationId,
			reason,
			acknowledgeUncertain,
		});
	}

	async keepBoth(
		appId: string,
		operationId: string,
	): Promise<IOfflineKeepBothResult> {
		return invoke("offline_writes_keep_both", {
			appId,
			token: this.requireToken(),
			operationId,
		});
	}

	async syncNow(appId: string): Promise<void> {
		await invoke("offline_writes_sync_now", {
			appId,
			token: this.requireToken(),
		});
	}

	async getTableRoute(
		appId: string,
		table: string,
		userScoped?: boolean,
	): Promise<OfflineTableRoute> {
		const token = this.token;
		if (!token) return "hub";
		try {
			return await invoke<OfflineTableRoute>(TABLE_ROUTE_COMMAND, {
				appId,
				token,
				table,
				userScoped: userScoped ?? false,
			});
		} catch (error) {
			if (!isMissingTableRouteCommand(error)) throw error;
			if (!missingTableRouteCommandReported) {
				missingTableRouteCommandReported = true;
				console.warn(
					`[OfflineWrites] Tauri command ${TABLE_ROUTE_COMMAND} is not registered; routing tables through the hub.`,
					error,
				);
			}
			return "hub";
		}
	}

	async forgetApp(appId: string, allAccounts: boolean): Promise<void> {
		await invoke("offline_writes_forget_app", {
			appId,
			token: this.token ?? null,
			allAccounts,
		});
	}

	subscribe(listener: (event: IOfflineStatusEvent) => void): () => void {
		return subscribeEvent(OFFLINE_WRITES_EVENTS.status, listener);
	}

	subscribeTables(
		listener: (event: IOfflineTablesChangedEvent) => void,
	): () => void {
		return subscribeEvent(OFFLINE_WRITES_EVENTS.tables, listener);
	}

	subscribeMirror(listener: (event: IOfflineMirrorEvent) => void): () => void {
		return subscribeEvent(OFFLINE_WRITES_EVENTS.mirror, listener);
	}
}
