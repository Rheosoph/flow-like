export const SCOPE_ALL = "*";
export const DOCUMENTS_TOOL = "documents";
export const SCOPE_DOCUMENTS_EXPENSIVE = "documents.expensive";
/**
 * The tool of the auto-approval grant `(*, *)`: everything a grant could cover runs without
 * asking. Deleting data, changing settings, cost checks and web destinations after web content
 * still ask; of those, only a site the owner always allows reading is read without asking.
 */
export const AUTO_APPROVE_TOOL = "*";

export function isAutoApproveGrant(grant: {
	tool: string;
	scope: string;
}): boolean {
	return grant.tool === AUTO_APPROVE_TOOL && grant.scope === SCOPE_ALL;
}

const MAX_SCOPE_CHARS = 200;

const NOT_GRANTABLE: ReadonlySet<string> = new Set([
	"bot_update_configuration",
	"create_bot",
	"update_bot",
	"delete_bot",
	"http_request",
	"forecast_tool",
	"flowpilot_board_review",
	"app_build",
	"apply_home_layout",
]);

type Args = Record<string, unknown>;

function isRecord(value: unknown): value is Args {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A scope part the stores accept: trimmed, short, no commas or control characters. */
export function cleanScopePart(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	if (!trimmed || [...trimmed].length > MAX_SCOPE_CHARS) return undefined;
	// biome-ignore lint/suspicious/noControlCharactersInRegex: control characters are what is refused
	return /[\u0000-\u001f\u007f-\u009f,]/.test(trimmed) ? undefined : trimmed;
}

function argument(args: Args, snakeCase: string, camelCase: string): string {
	const value = args[snakeCase] ?? args[camelCase];
	return typeof value === "string" ? value : "";
}

/** The lowercase program a call is approved under: `git` for `/usr/bin/git`, `sh` for `shell: true`. */
export function programName(command: string, shell: boolean): string | null {
	if (shell) return "sh";
	const base = command.trim().split(/[\\/]/).pop()?.toLowerCase() ?? "";
	const name = base.replace(/\.(exe|cmd|bat|com)$/, "");
	return name || null;
}

function appScope(args: Args, contextAppId?: string): string | undefined {
	const id =
		cleanScopePart(argument(args, "app_id", "appId")) ??
		cleanScopePart(contextAppId);
	return id ? `app:${id}` : undefined;
}

function networkHosts(args: Args): string[] {
	const hosts = Array.isArray(args.network_hosts) ? args.network_hosts : [];
	const cleaned = hosts
		.filter((host): host is string => typeof host === "string")
		.map((host) => cleanScopePart(host.toLowerCase()))
		.filter((host): host is string => host !== undefined);
	return [...new Set(cleaned)].sort();
}

function connectionScope(args: Args): string | undefined {
	const attach = args.attach;
	if (!isRecord(attach) || !isRecord(attach.connection)) return undefined;
	const { connection } = attach;
	const port =
		typeof connection.port === "string"
			? connection.port.trim()
			: connection.port === null || connection.port === undefined
				? ""
				: JSON.stringify(connection.port);
	const kind = cleanScopePart(attach.kind);
	const host = cleanScopePart(connection.host);
	const database = cleanScopePart(connection.database);
	if (!kind || !host || !database) return undefined;
	return `connection:${kind}:${host}:${cleanScopePart(port) ?? ""}:${database}`;
}

/** The site a URL belongs to: its lowercase host without a leading `www.`. */
export function siteOf(url: string): string | undefined {
	let host: string;
	try {
		host = new URL(url.trim()).hostname.toLowerCase();
	} catch {
		return undefined;
	}
	host = host.replace(/\.+$/, "");
	host = host.startsWith("www.") ? host.slice(4) : host;
	return host || undefined;
}

/**
 * The site `web_request` grants are kept for: the lowercase host, `host:port` off the default
 * port, `www.` kept. Mirrors `site_of` in `packages/bot-tools/src/web/mod.rs`.
 */
function webSiteOf(raw: string): string | undefined {
	let url: URL;
	try {
		url = new URL(raw.trim());
	} catch {
		return undefined;
	}
	if (url.protocol !== "http:" && url.protocol !== "https:") return undefined;
	if (url.username || url.password) return undefined;
	const host = url.hostname.toLowerCase().replace(/\.+$/, "");
	if (!host) return undefined;
	return url.port ? `${host}:${url.port}` : host;
}

const SECRET_REFERENCE = /\{\{secret:([\s\S]*?)\}\}/g;
const SECRET_NAME = /^[A-Za-z0-9_.-]{1,128}$/;

/** The `{{secret:NAME}}` names anywhere in a call's arguments, sorted. */
function referencedSecrets(args: unknown): string[] {
	const names = new Set<string>();
	const collect = (value: unknown) => {
		if (typeof value === "string") {
			for (const [, name] of value.matchAll(SECRET_REFERENCE)) {
				if (SECRET_NAME.test(name)) names.add(name);
			}
		} else if (Array.isArray(value)) {
			value.forEach(collect);
		} else if (isRecord(value)) {
			Object.values(value).forEach(collect);
		}
	};
	collect(args);
	return [...names].sort();
}

/**
 * `web:<site>:read` or `:write` (`:private:read`/`:private:write` with `allow_private`), plus
 * `web:<site>:secret:<NAME>` per secret sent. Mirrors `grant_scopes` in
 * `packages/bot-tools/src/web/mod.rs`.
 */
function webScopes(args: Args): string[] | null {
	if (typeof args.url !== "string") return null;
	const site = webSiteOf(args.url);
	if (!site) return null;
	const method =
		typeof args.method === "string" && args.method.trim()
			? args.method.trim().toUpperCase()
			: "GET";
	const kind = method === "GET" || method === "HEAD" ? "read" : "write";
	const privateNet = args.allow_private === true ? "private:" : "";
	const scopes = [
		`web:${site}:${privateNet}${kind}`,
		...referencedSecrets(args).map((name) => `web:${site}:secret:${name}`),
	].map(cleanScopePart);
	return scopes.every(Boolean) ? (scopes as string[]) : null;
}

/** A plain read of a site, `web:<site>:read`: not private-network, a write or a secret. */
export function isSiteReadScope(scope: string): boolean {
	const site = scope.startsWith("web:")
		? scope.slice(4).match(/^(.+):read$/)?.[1]
		: undefined;
	return (
		site !== undefined &&
		!site.endsWith(":private") &&
		!site.includes(":secret")
	);
}

/**
 * Whether every scope is a plain site read, the one web destination whose grant still covers a
 * call after the turn read web pages. Mirrors `site_reads_only` in `tool_grants.rs`.
 */
export function siteReadsOnly(scopes: readonly string[] | null): boolean {
	return !!scopes && scopes.length > 0 && scopes.every(isSiteReadScope);
}

function recipientDomain(address: string): string | undefined {
	const at = address.trim().lastIndexOf("@");
	if (at < 1) return undefined;
	const local = address.trim().slice(0, at);
	const host = address
		.trim()
		.slice(at + 1)
		.trim()
		.replace(/\.+$/, "")
		.toLowerCase();
	return local && host && !/\s/.test(host) ? host : undefined;
}

function emailScopes(args: Args): string[] | null {
	const domains: string[] = [];
	for (const key of ["to", "cc"]) {
		const list = args[key];
		if (!Array.isArray(list)) continue;
		for (const address of list) {
			if (typeof address !== "string") continue;
			const domain = recipientDomain(address);
			if (!domain) return null;
			domains.push(`email:${domain}`);
		}
	}
	const unique = [...new Set(domains)].sort();
	return unique.length > 0 ? unique : null;
}

function single(scope: string | undefined): string[] | null {
	return scope ? [scope] : null;
}

/**
 * The persisted grant scopes that cover one call of `toolName`, or `null` when the call is never
 * remembered. Mirrors `grant_scopes` in `packages/core/editor/src/flow/copilot/tool_grants.rs`.
 */
export function grantScopes(
	toolName: string,
	args: Args,
	contextAppId?: string,
): string[] | null {
	if (NOT_GRANTABLE.has(toolName)) return null;
	switch (toolName) {
		case "call_app_event":
		case "call_app_chat":
			return single(appScope(args, contextAppId));
		case "database_tool": {
			if (argument(args, "operation", "operation") !== "delete_table") {
				return single(appScope(args, contextAppId));
			}
			const table = cleanScopePart(argument(args, "table_name", "tableName"));
			return single(table ? `table:${table}` : undefined);
		}
		case "knowledge_drop": {
			const index = cleanScopePart(argument(args, "index", "index"));
			return single(index ? `table:${index}` : undefined);
		}
		case "interact_app_page": {
			const appId = cleanScopePart(argument(args, "app_id", "appId"));
			const app = appId ?? cleanScopePart(contextAppId);
			if (!app) return null;
			const eventId = cleanScopePart(argument(args, "event_id", "eventId"));
			if (eventId) return [`target:${app}:event:${eventId}`];
			const pageId = cleanScopePart(argument(args, "page_id", "pageId"));
			return pageId ? [`target:${app}:page:${pageId}`] : null;
		}
		case "code_run": {
			const hosts = networkHosts(args);
			return hosts.length > 0 ? hosts.map((host) => `host:${host}`) : null;
		}
		case "data_sources":
			return single(connectionScope(args));
		case "browser_open": {
			const site = siteOf(argument(args, "url", "url"));
			const prefix = args.browser === "owner" ? "owner_browser" : "browser";
			return single(site ? `${prefix}:${site}` : undefined);
		}
		case "computer_screenshot":
		case "computer_window":
		case "computer_act":
			return ["computer:session"];
		case "email_send":
			return emailScopes(args);
		case "shell_run": {
			if (typeof args.command !== "string") return null;
			const program = programName(args.command, args.shell === true);
			return single(program ? cleanScopePart(`command:${program}`) : undefined);
		}
		case "web_request":
			return webScopes(args);
		case "file_save":
			return typeof args.url === "string" && args.url.trim()
				? webScopes({ url: args.url })
				: null;
		default:
			return [SCOPE_ALL];
	}
}

/** The grants an "Always allow" on this call writes, as `[tool, scope]` pairs. */
export function grantKeys(
	toolName: string,
	args: Args,
	contextAppId?: string,
): Array<[string, string]> {
	return (grantScopes(toolName, args, contextAppId) ?? []).map((scope) => [
		scope === SCOPE_DOCUMENTS_EXPENSIVE ? DOCUMENTS_TOOL : toolName,
		scope,
	]);
}
