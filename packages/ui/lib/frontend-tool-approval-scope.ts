import type { IBotState } from "../state/backend-state/bot-state";
import { SCOPE_ALL, programName } from "./tool-grant-scopes";

export interface FrontendToolApprovalScopeInput {
	requestId: string;
	toolName: string;
	arguments: Record<string, unknown>;
	approvalKind?: string;
	approvalSessionKey?: string;
	contextAppId?: string;
}

export interface FrontendToolApprovalScope {
	sessionKey: string;
	rememberable: boolean;
	/** The app a remembered approval is limited to; unset when it covers the tool in every app. */
	appId?: string;
}

function nonEmptyString(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	return value.trim() ? value : undefined;
}

function argumentString(
	args: Record<string, unknown>,
	snakeCase: string,
	camelCase: string,
): string | undefined {
	return nonEmptyString(args[snakeCase]) ?? nonEmptyString(args[camelCase]);
}

function scopePart(value: string): string {
	return encodeURIComponent(value);
}

/** Tools that act inside one app: a standing approval covers that app alone. */
const APP_SCOPED_TOOLS: ReadonlySet<string> = new Set([
	"call_app_event",
	"call_app_chat",
	"database_tool",
]);

/** The program a `shell_run` call is approved under; every `shell: true` call is `sh`. */
function shellProgramName(args: Record<string, unknown>): string {
	return (
		programName(nonEmptyString(args.command) ?? "", args.shell === true) ?? "?"
	);
}

/**
 * Whose standing decisions a tool call falls under: the asking run's bot id, `null` for Flux (every
 * Flux chat shares one), `undefined` when no run owns the call, which nothing waives.
 */
export type ApprovalOwner = string | null | undefined;

/** Where a bot's standing approvals are kept: the account API or this computer's store. */
export interface GrantStore {
	list(botId: string): Promise<ReadonlyArray<{ tool: string; scope: string }>>;
	put(botId: string, tool: string, scope: string): Promise<unknown>;
}

/** The grant store of a backend, or none where its bots keep no grants. */
export function grantStoreOf(
	state: Pick<IBotState, "listToolGrants" | "putToolGrant"> | undefined,
): GrantStore | undefined {
	const list = state?.listToolGrants?.bind(state);
	const put = state?.putToolGrant?.bind(state);
	return list && put ? { list, put } : undefined;
}

const GRANT_CACHE_MS = 5_000;

function grantId(tool: string, scope: string): string {
	return `${tool}\u0000${scope}`;
}

interface LoadedGrants {
	at: number;
	ids: Set<string>;
}

/**
 * "Don't ask again" answers, kept apart per owner. The session keys are the in-memory cache; a
 * bot's answers also persist as grants in the bot's store (when the host keeps them), so its
 * headless turns and its chats on every device honour them.
 */
export class StandingApprovals {
	private readonly byOwner = new Map<string | null, Set<string>>();
	private readonly loaded = new Map<string, LoadedGrants>();
	private readonly loading = new Map<
		string,
		Promise<LoadedGrants | undefined>
	>();

	constructor(
		private readonly store: () => GrantStore | undefined = () => undefined,
	) {}

	has(owner: ApprovalOwner, sessionKey: string): boolean {
		if (owner === undefined) return false;
		return this.byOwner.get(owner)?.has(sessionKey) ?? false;
	}

	add(owner: ApprovalOwner, sessionKey: string): void {
		if (owner === undefined) return;
		const keys = this.byOwner.get(owner) ?? new Set<string>();
		keys.add(sessionKey);
		this.byOwner.set(owner, keys);
	}

	private read(botId: string): Promise<LoadedGrants | undefined> {
		const fresh = this.loaded.get(botId);
		if (fresh && Date.now() - fresh.at < GRANT_CACHE_MS) {
			return Promise.resolve(fresh);
		}
		const store = this.store();
		if (!store) return Promise.resolve(undefined);
		const pending = this.loading.get(botId);
		if (pending) return pending;
		const request = store
			.list(botId)
			.then((grants) => {
				const entry = {
					at: Date.now(),
					ids: new Set(grants.map((g) => grantId(g.tool, g.scope))),
				};
				this.loaded.set(botId, entry);
				return entry;
			})
			.catch((error) => {
				console.warn(`Reading the tool grants of bot ${botId} failed`, error);
				return undefined;
			})
			.finally(() => this.loading.delete(botId));
		this.loading.set(botId, request);
		return request;
	}

	/** Whether the bot's persisted grants cover every scope of this call; false when they can't be read. */
	async coveredByGrant(
		owner: ApprovalOwner,
		tool: string,
		scopes: readonly string[],
	): Promise<boolean> {
		if (typeof owner !== "string" || scopes.length === 0) return false;
		const grants = await this.read(owner);
		if (!grants) return false;
		return (
			grants.ids.has(grantId(tool, SCOPE_ALL)) ||
			scopes.every((scope) => grants.ids.has(grantId(tool, scope)))
		);
	}

	/**
	 * Remembers an answer for this session and, for a bot, writes the grants through to its store.
	 * Rejects when the store refused; the session still remembers.
	 */
	async remember(
		owner: ApprovalOwner,
		sessionKey: string,
		grants: ReadonlyArray<readonly [tool: string, scope: string]>,
	): Promise<void> {
		this.add(owner, sessionKey);
		const store = this.store();
		if (typeof owner !== "string" || !store) return;
		const held = this.loaded.get(owner);
		await Promise.all(
			grants.map(async ([tool, scope]) => {
				await store.put(owner, tool, scope);
				held?.ids.add(grantId(tool, scope));
			}),
		);
	}
}

/**
 * Resolve the frontend session allowlist key for one tool request. Page interaction approvals are
 * reusable only when both the app and exact Event/page target are known. HTTP requests require
 * their own approval because their destinations, credentials, and bodies may differ.
 */
export function resolveFrontendToolApprovalScope({
	requestId,
	toolName,
	arguments: args,
	approvalKind,
	approvalSessionKey,
	contextAppId,
}: FrontendToolApprovalScopeInput): FrontendToolApprovalScope {
	if (toolName === "http_request") {
		return {
			sessionKey: `http_request:request:${scopePart(requestId)}`,
			rememberable: false,
		};
	}
	if (toolName === "forecast_tool" && args.operation === "save") {
		// Approval names one retained result and destination; never waive future saves.
		return {
			sessionKey: `forecast_tool:save:request:${scopePart(requestId)}`,
			rememberable: false,
		};
	}
	if (toolName === "shell_run") {
		return {
			sessionKey: `shell_run:command:${scopePart(shellProgramName(args))}`,
			rememberable: true,
		};
	}
	if (APP_SCOPED_TOOLS.has(toolName)) {
		const appId =
			argumentString(args, "app_id", "appId") ?? nonEmptyString(contextAppId);
		const base = approvalSessionKey || `${toolName}:${approvalKind ?? "none"}`;
		return appId
			? {
					sessionKey: `${base}:app:${scopePart(appId)}`,
					rememberable: true,
					appId,
				}
			: {
					sessionKey: `${toolName}:request:${scopePart(requestId)}`,
					rememberable: false,
				};
	}
	if (toolName !== "interact_app_page") {
		return {
			sessionKey: approvalSessionKey || `${toolName}:${approvalKind ?? "none"}`,
			rememberable: true,
		};
	}

	const appId =
		argumentString(args, "app_id", "appId") ?? nonEmptyString(contextAppId);
	const eventId = argumentString(args, "event_id", "eventId");
	const pageId = argumentString(args, "page_id", "pageId");
	const target = eventId
		? { kind: "event", id: eventId }
		: pageId
			? { kind: "page", id: pageId }
			: undefined;

	if (appId && target) {
		return {
			sessionKey: `interact_app_page:${scopePart(appId)}:${target.kind}:${scopePart(target.id)}`,
			rememberable: true,
			appId,
		};
	}

	return {
		sessionKey: `interact_app_page:request:${scopePart(requestId)}`,
		rememberable: false,
	};
}
