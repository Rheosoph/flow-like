export type OfflineTablePurpose = "storage" | "user";

export interface IOfflineTableSelection {
	purpose: OfflineTablePurpose;
	table: string;
	primaryKey: string;
	prefetch: boolean;
}

export type OfflineTableStatus = "preparing" | "settling" | "ready" | "error";

export interface IOfflineTableState extends IOfflineTableSelection {
	status: OfflineTableStatus;
	error?: string | null;
	pendingCount: number;
	snapshotVersion?: number | null;
	/** Unix seconds. */
	refreshedAt?: number | null;
	/** Cloud files of the current version kept on this device. */
	cachedBytes: number;
	/** All cloud files of the current version; null while a size is unknown. */
	totalBytes?: number | null;
	/** This device's version metadata and change files. */
	localBytes: number;
	/** Every cloud file of the current version is on this device. */
	offlineComplete: boolean;
	downloading: boolean;
	keyIndexed?: boolean | null;
	/** This table's reason for using an older copy, or for a pending activation. */
	mirrorError?: string | null;
	waitingForRuns: number;
	remoteMissing: boolean;
}

export type OfflineSyncState =
	| "idle"
	| "syncing"
	| "waitingForConnection"
	| "waitingForSignIn"
	| "hubError"
	| "blocked";

export type OfflineOperationKind =
	| "tableInsert"
	| "tableUpsert"
	| "tableUpdate"
	| "tableDelete"
	| "fileWrite"
	| "fileDelete";

export type OfflineErrorCode =
	| "hub_limit"
	| "forbidden"
	| "endpoint_missing"
	| "subject_mismatch"
	| "invalid"
	| "digest_reused";

export type IOfflineResourceLabel =
	| { type: "table"; purpose: OfflineTablePurpose; table: string }
	| { type: "file"; purpose: "files" | "storage" | "user"; path: string };

export interface IOfflineOperation {
	sequence: number;
	operationId: string;
	kind: OfflineOperationKind;
	resource: IOfflineResourceLabel;
	state: "pending" | "attempting" | "blocked" | "conflict" | "outcome_unknown";
	attempts: number;
	/** Unix seconds. */
	createdAt: number;
	bytes: number;
	error?: string | null;
	errorCode?: OfflineErrorCode | null;
}

export interface IOfflineQueueStatus {
	pendingCount: number;
	pendingBytes: number;
	/** Unix seconds. */
	oldestAt?: number | null;
	syncState: OfflineSyncState;
	blockedHeads: IOfflineOperation[];
}

export interface IOfflineLimits {
	maxQueueBytes: number;
	maxOperations: number;
	maxAgeSeconds: number;
	maxMirrorBytes: number;
}

export interface IOfflineHubLimits {
	maxOperationBytes: number;
	maxFileBytes: number;
	maxRequestBytes?: number | null;
}

export interface IOfflineUsage {
	queueBytes: number;
	mirrorBytes: number;
	cacheBytes: number;
	pinnedBytes: number;
	requiredBytes: number;
	downloadedBytesToday: number;
	maxDownloadBytesPerDay?: number | null;
}

export interface IOfflineAppOverview {
	appId: string;
	available: boolean;
	unavailableReason?: string | null;
	subject?: string | null;
	provider?: "s3" | "az" | "gs" | null;
	hubSupport?: boolean | null;
	hubLimits?: IOfflineHubLimits | null;
	tables: IOfflineTableState[];
	queue: IOfflineQueueStatus;
	limits: IOfflineLimits;
	usage: IOfflineUsage;
	otherAccountsPending: number;
}

export type OfflineOperationLookupState =
	| "pending"
	| "attempting"
	| "blocked"
	| "conflict"
	| "outcomeUnknown"
	| "applied"
	| "skipped"
	| "superseded"
	| "unknown";

export interface IOfflineOperationLookup {
	state: OfflineOperationLookupState;
	supersededBy?: string | null;
	errorCode?: OfflineErrorCode | null;
}

export type OfflineTableRoute = "device" | "hub";

export interface IOfflineKeepBothResult {
	newPath: string;
}

export interface IOfflineStatusEvent {
	appId: string;
	pendingCount: number;
	syncState: OfflineSyncState;
	headState?: string | null;
	blockedCount: number;
}

export interface IOfflineTablesChangedEvent {
	appId: string;
}

export interface IOfflineMirrorEvent {
	appId: string;
}

export interface IOfflineWritesState {
	getOverview(appId: string): Promise<IOfflineAppOverview>;
	setTable(
		appId: string,
		selection: IOfflineTableSelection,
	): Promise<IOfflineTableState>;
	removeTable(
		appId: string,
		purpose: OfflineTablePurpose,
		table: string,
	): Promise<void>;
	/** Download everything on or off. Rejects with the E31 text when it does not fit. */
	setPrefetch(
		appId: string,
		purpose: OfflineTablePurpose,
		table: string,
		prefetch: boolean,
	): Promise<IOfflineTableState>;
	setLimits(appId: string, limits: IOfflineLimits): Promise<IOfflineLimits>;
	listOperations(
		appId: string,
		afterSequence?: number,
		limit?: number,
	): Promise<IOfflineOperation[]>;
	getOperationState(
		appId: string,
		operationId: string,
	): Promise<IOfflineOperationLookup>;
	retryOperation(appId: string, operationId: string): Promise<void>;
	skipOperation(
		appId: string,
		operationId: string,
		reason: string,
		acknowledgeUncertain: boolean,
	): Promise<void>;
	keepBoth(appId: string, operationId: string): Promise<IOfflineKeepBothResult>;
	syncNow(appId: string): Promise<void>;
	/** Asked on every Data Studio operation of an online app. Never cached. */
	getTableRoute(
		appId: string,
		table: string,
		userScoped?: boolean,
	): Promise<OfflineTableRoute>;
	forgetApp(appId: string, allAccounts: boolean): Promise<void>;
	subscribe(listener: (event: IOfflineStatusEvent) => void): () => void;
	subscribeTables(
		listener: (event: IOfflineTablesChangedEvent) => void,
	): () => void;
	subscribeMirror(listener: (event: IOfflineMirrorEvent) => void): () => void;
}
