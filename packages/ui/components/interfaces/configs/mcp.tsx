"use client";

import {
	AlertCircle,
	Braces,
	Check,
	Cloud,
	Code2,
	Copy,
	Database,
	Download,
	FileText,
	ImageIcon,
	Info,
	Link2,
	Loader2,
	MessageSquare,
	Music,
	Play,
	RefreshCw,
	Server,
	Terminal,
	Trash2,
	Video,
	Wrench,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { getApiOrigin } from "../../../lib/api-url";
import { useBackend } from "../../../state/backend-state";
import type {
	IEventAlias,
	IEventRegistration,
	IEventRemoteAuth,
	IListRegistrationsResponse,
} from "../../../state/backend-state/event-state";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	Badge,
	Button,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Switch,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
} from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

type McpSink = {
	sink_type?: "mcp";
};

const MCP_INSPECTOR_PROTOCOL_VERSION = "2025-06-18";

const EMPTY_ALIASES: IEventAlias[] = [];

async function listNoEventAliases(): Promise<IEventAlias[]> {
	return EMPTY_ALIASES;
}

async function listNoEventRegistrations(
	_appId: string,
	eventId: string,
): Promise<IListRegistrationsResponse> {
	return {
		event_id: eventId,
		event_version: null,
		registrations: [],
		auths: [],
	};
}

type McpJsonRpcRequest = {
	jsonrpc: "2.0";
	id?: number;
	method: string;
	params?: Record<string, unknown>;
};

type McpInspectorCall = {
	method: string;
	status: number;
	body: unknown;
	durationMs: number;
};

type McpInspectorResult = {
	endpoint: string;
	sessionId: string;
	protocolVersion: string;
	serverInfo: Record<string, unknown> | null;
	capabilities: Record<string, unknown> | null;
	tools: Array<Record<string, unknown>>;
	resources: Array<Record<string, unknown>>;
	prompts: Array<Record<string, unknown>>;
	calls: McpInspectorCall[];
	totalDurationMs: number;
};

type McpInspectorExecution = {
	method: string;
	result: Record<string, unknown>;
	calls: McpInspectorCall[];
	sessionId: string;
	protocolVersion: string;
	totalDurationMs: number;
};

function normalizeAuthType(value: unknown): string {
	if (typeof value !== "string") return "";
	if (value === "o_auth_bearer") return "oauth_bearer";
	return value;
}

function authLabel(config: Record<string, any> | undefined) {
	const type = normalizeAuthType(config?.type);
	if (type === "api_key") return "API key";
	if (type === "bearer_token") return "Bearer token";
	if (type === "basic_auth") return "Basic auth";
	if (type === "hmac_sha256") return "HMAC SHA-256";
	if (type === "oauth_bearer") return "OAuth bearer";
	if (type === "none") return "none";
	if (type && type !== "none") return type.replaceAll("_", " ");
	return "configured";
}

function authConfigEntries(config: Record<string, any> | undefined) {
	if (!config) return [];
	return Object.entries(config).filter(([key, value]) => {
		if (key === "type") return false;
		if (value === null || value === undefined) return false;
		if (typeof value === "string" && value.trim() === "") return false;
		if (Array.isArray(value) && value.length === 0) return false;
		return true;
	});
}

function formatConfigValue(value: unknown, key?: string): string {
	if (key?.endsWith("_configured") && value === true) return "configured";
	if (typeof value === "boolean") return value ? "yes" : "no";
	if (typeof value === "string") return value;
	if (typeof value === "number") return String(value);
	if (value === null || value === undefined) return "none";
	return JSON.stringify(value);
}

function prettyJson(value: unknown): string {
	try {
		const json = JSON.stringify(value, null, 2);
		return json === undefined ? String(value) : json;
	} catch {
		return String(value);
	}
}

function registrationExtras(
	registration: IEventRegistration | undefined,
): Record<string, any> {
	const extras = registration?.extras;
	return extras && typeof extras === "object" && !Array.isArray(extras)
		? extras
		: {};
}

function flowPathLabel(value: unknown): string | null {
	if (!value) return null;
	if (typeof value === "string") return value;
	if (typeof value === "object" && !Array.isArray(value)) {
		const path = (value as Record<string, any>).path;
		if (typeof path === "string" && path.length > 0) return path;
	}
	return null;
}

function promptArguments(template: unknown): string[] {
	if (typeof template !== "string") return [];
	const found = new Set<string>();
	for (const match of template.matchAll(/\{\{\s*([^}]+?)\s*\}\}/g)) {
		const name = match[1]?.trim();
		if (name) found.add(name);
	}
	return Array.from(found);
}

function schemaPropertyNames(schema: unknown): string[] {
	if (!schema || typeof schema !== "object" || Array.isArray(schema)) return [];
	const properties = (schema as Record<string, any>).properties;
	if (
		!properties ||
		typeof properties !== "object" ||
		Array.isArray(properties)
	) {
		return [];
	}
	return Object.keys(properties);
}

function mcpKindLabel(kind: string): string {
	if (kind === "mcp_tool") return "Tools";
	if (kind === "mcp_resource") return "Resources";
	if (kind === "mcp_prompt") return "Prompts";
	if (kind === "mcp_raw") return "Server";
	return kind.replace(/^mcp_/, "").replaceAll("_", " ");
}

function mcpEntryLabel(kind: string): string {
	if (kind === "mcp_tool") return "Tool";
	if (kind === "mcp_resource") return "Resource";
	if (kind === "mcp_prompt") return "Prompt";
	if (kind === "mcp_raw") return "Server";
	return mcpKindLabel(kind);
}

function mcpDetailEntries(registration: IEventRegistration) {
	const extras = registrationExtras(registration);
	const details: Array<[string, string]> = [];

	if (registration.kind === "mcp_tool") {
		const description = extras.description;
		if (registration.node_id) details.push(["handler", registration.node_id]);
		if (typeof description === "string" && description.trim()) {
			details.push(["description", description]);
		}
		const args = schemaPropertyNames(registration.schema);
		if (args.length > 0) details.push(["arguments", args.join(", ")]);
		return details;
	}

	if (registration.kind === "mcp_resource") {
		if (typeof extras.name === "string") details.push(["name", extras.name]);
		const file = flowPathLabel(extras.flow_path);
		if (file) details.push(["file", file]);
		const mime = extras.mime_type ?? extras.mimeType;
		if (typeof mime === "string") details.push(["mime type", mime]);
		return details;
	}

	if (registration.kind === "mcp_prompt") {
		if (typeof extras.description === "string" && extras.description.trim()) {
			details.push(["description", extras.description]);
		}
		const args = promptArguments(extras.template);
		if (args.length > 0) details.push(["arguments", args.join(", ")]);
		return details;
	}

	if (registration.kind === "mcp_raw") {
		for (const key of ["host", "port", "path", "max_connections"]) {
			if (extras[key] !== undefined) {
				details.push([key, formatConfigValue(extras[key])]);
			}
		}
		return details;
	}

	return details;
}

function suggestedInspectorHeader(auths: IEventRemoteAuth[]): string {
	const config = auths[0]?.config;
	const type = normalizeAuthType(config?.type);
	if (type === "api_key") {
		const header = config?.header;
		return typeof header === "string" && header.trim() ? header : "x-api-key";
	}
	if (
		type === "bearer_token" ||
		type === "basic_auth" ||
		type === "oauth_bearer"
	) {
		return "Authorization";
	}
	return "";
}

function inspectorAuthLabel(auths: IEventRemoteAuth[]): string {
	if (auths.length === 0) return "none";
	return auths.map((auth) => authLabel(auth.config)).join(", ");
}

function jsonRpcErrorMessage(body: unknown): string | null {
	if (!body || typeof body !== "object" || Array.isArray(body)) return null;
	const error = (body as Record<string, unknown>).error;
	if (!error || typeof error !== "object" || Array.isArray(error)) return null;
	const message = (error as Record<string, unknown>).message;
	return typeof message === "string" ? message : prettyJson(error);
}

function jsonRpcResult(body: unknown, method: string): Record<string, unknown> {
	const error = jsonRpcErrorMessage(body);
	if (error) throw new Error(`${method}: ${error}`);
	if (!body || typeof body !== "object" || Array.isArray(body)) return {};
	const result = (body as Record<string, unknown>).result;
	return result && typeof result === "object" && !Array.isArray(result)
		? (result as Record<string, unknown>)
		: {};
}

function resultArray<T extends Record<string, unknown>>(
	result: Record<string, unknown>,
	key: string,
): T[] {
	const value = result[key];
	return Array.isArray(value) ? (value as T[]) : [];
}

async function readMcpResponse(response: Response): Promise<unknown> {
	const text = await response.text();
	if (!text.trim()) return null;
	try {
		return JSON.parse(text);
	} catch {
		return text;
	}
}

async function postMcpInspectorRequest(
	endpoint: string,
	payload: McpJsonRpcRequest,
	options: {
		sessionId?: string;
		protocolVersion?: string;
		headerName?: string;
		headerValue?: string;
	},
): Promise<{ sessionId: string | null; call: McpInspectorCall }> {
	const headers: Record<string, string> = {
		Accept: "application/json",
		"Content-Type": "application/json",
		"MCP-Protocol-Version":
			options.protocolVersion ?? MCP_INSPECTOR_PROTOCOL_VERSION,
	};
	if (options.sessionId) headers["Mcp-Session-Id"] = options.sessionId;
	const headerName = options.headerName?.trim();
	const headerValue = options.headerValue?.trim();
	if (headerName && headerValue) headers[headerName] = headerValue;

	const startedAt = performance.now();
	const response = await fetch(endpoint, {
		method: "POST",
		headers,
		body: JSON.stringify(payload),
	});
	const body = await readMcpResponse(response);
	const durationMs = Math.round(performance.now() - startedAt);
	const call = {
		method: payload.method,
		status: response.status,
		body,
		durationMs,
	};
	if (!response.ok) {
		const message =
			typeof body === "string"
				? body
				: (jsonRpcErrorMessage(body) ?? prettyJson(body));
		throw new Error(`${payload.method}: HTTP ${response.status} ${message}`);
	}
	return {
		sessionId: response.headers.get("mcp-session-id"),
		call,
	};
}

async function openMcpInspectorSession(
	endpoint: string,
	headerName: string,
	headerValue: string,
): Promise<{
	sessionId: string;
	protocolVersion: string;
	serverInfo: Record<string, unknown> | null;
	capabilities: Record<string, unknown> | null;
	calls: McpInspectorCall[];
}> {
	const calls: McpInspectorCall[] = [];
	const initialize = await postMcpInspectorRequest(
		endpoint,
		{
			jsonrpc: "2.0",
			id: 1,
			method: "initialize",
			params: {
				protocolVersion: MCP_INSPECTOR_PROTOCOL_VERSION,
				capabilities: {},
				clientInfo: {
					name: "Flow Like Config Inspector",
					version: "1.0.0",
				},
			},
		},
		{ headerName, headerValue },
	);
	calls.push(initialize.call);

	const initResult = jsonRpcResult(initialize.call.body, "initialize");
	const protocolVersion =
		typeof initResult.protocolVersion === "string"
			? initResult.protocolVersion
			: MCP_INSPECTOR_PROTOCOL_VERSION;
	const sessionId = initialize.sessionId;
	if (!sessionId) {
		throw new Error("initialize: missing Mcp-Session-Id response header");
	}

	const initialized = await postMcpInspectorRequest(
		endpoint,
		{
			jsonrpc: "2.0",
			method: "notifications/initialized",
			params: {},
		},
		{
			headerName,
			headerValue,
			sessionId,
			protocolVersion,
		},
	);
	calls.push(initialized.call);

	return {
		sessionId,
		protocolVersion,
		serverInfo:
			initResult.serverInfo &&
			typeof initResult.serverInfo === "object" &&
			!Array.isArray(initResult.serverInfo)
				? (initResult.serverInfo as Record<string, unknown>)
				: null,
		capabilities:
			initResult.capabilities &&
			typeof initResult.capabilities === "object" &&
			!Array.isArray(initResult.capabilities)
				? (initResult.capabilities as Record<string, unknown>)
				: null,
		calls,
	};
}

function closeMcpInspectorSession(
	endpoint: string,
	sessionId: string,
	protocolVersion: string,
	headerName: string,
	headerValue: string,
) {
	void fetch(endpoint, {
		method: "DELETE",
		headers: {
			Accept: "application/json",
			"MCP-Protocol-Version": protocolVersion,
			"Mcp-Session-Id": sessionId,
			...(headerName.trim() && headerValue.trim()
				? { [headerName.trim()]: headerValue.trim() }
				: {}),
		},
	}).catch(() => undefined);
}

async function inspectMcpEndpoint(
	endpoint: string,
	headerName: string,
	headerValue: string,
): Promise<McpInspectorResult> {
	const startedAt = performance.now();
	let id = 2;
	const session = await openMcpInspectorSession(
		endpoint,
		headerName,
		headerValue,
	);
	const calls = [...session.calls];

	const sessionOptions = {
		headerName,
		headerValue,
		sessionId: session.sessionId,
		protocolVersion: session.protocolVersion,
	};

	const tools = await postMcpInspectorRequest(
		endpoint,
		{ jsonrpc: "2.0", id: id++, method: "tools/list", params: {} },
		sessionOptions,
	);
	calls.push(tools.call);

	const resources = await postMcpInspectorRequest(
		endpoint,
		{ jsonrpc: "2.0", id: id++, method: "resources/list", params: {} },
		sessionOptions,
	);
	calls.push(resources.call);

	const prompts = await postMcpInspectorRequest(
		endpoint,
		{ jsonrpc: "2.0", id: id++, method: "prompts/list", params: {} },
		sessionOptions,
	);
	calls.push(prompts.call);

	const toolsResult = jsonRpcResult(tools.call.body, "tools/list");
	const resourcesResult = jsonRpcResult(resources.call.body, "resources/list");
	const promptsResult = jsonRpcResult(prompts.call.body, "prompts/list");

	closeMcpInspectorSession(
		endpoint,
		session.sessionId,
		session.protocolVersion,
		headerName,
		headerValue,
	);

	return {
		endpoint,
		sessionId: session.sessionId,
		protocolVersion: session.protocolVersion,
		serverInfo: session.serverInfo,
		capabilities: session.capabilities,
		tools: resultArray(toolsResult, "tools"),
		resources: resultArray(resourcesResult, "resources"),
		prompts: resultArray(promptsResult, "prompts"),
		calls,
		totalDurationMs: Math.round(performance.now() - startedAt),
	};
}

async function executeMcpInspectorMethod(
	endpoint: string,
	headerName: string,
	headerValue: string,
	method: string,
	params: Record<string, unknown>,
): Promise<McpInspectorExecution> {
	const startedAt = performance.now();
	const session = await openMcpInspectorSession(
		endpoint,
		headerName,
		headerValue,
	);
	const calls = [...session.calls];
	try {
		const response = await postMcpInspectorRequest(
			endpoint,
			{
				jsonrpc: "2.0",
				id: 2,
				method,
				params,
			},
			{
				headerName,
				headerValue,
				sessionId: session.sessionId,
				protocolVersion: session.protocolVersion,
			},
		);
		calls.push(response.call);
		const result = jsonRpcResult(response.call.body, method);
		return {
			method,
			result,
			calls,
			sessionId: session.sessionId,
			protocolVersion: session.protocolVersion,
			totalDurationMs: Math.round(performance.now() - startedAt),
		};
	} finally {
		closeMcpInspectorSession(
			endpoint,
			session.sessionId,
			session.protocolVersion,
			headerName,
			headerValue,
		);
	}
}

function DetailRow({ label, value }: { label: string; value: string }) {
	return (
		<div className="grid min-w-0 grid-cols-[7.5rem_minmax(0,1fr)] items-start gap-2 rounded-sm bg-muted/50 px-2 py-1.5">
			<span className="text-muted-foreground">{label}</span>
			<code className="min-w-0 whitespace-normal break-all text-left font-mono">
				{value}
			</code>
		</div>
	);
}

function EndpointField({
	label,
	value,
	copyKey,
	copied,
	onCopy,
}: {
	label: string;
	value: string;
	copyKey: string;
	copied: string | null;
	onCopy: (label: string, value: string) => void;
}) {
	return (
		<div className="space-y-1.5">
			<Label className="text-xs uppercase tracking-wide text-muted-foreground">
				{label}
			</Label>
			<div className="flex items-center gap-2">
				<Input readOnly value={value} className="font-mono text-xs" />
				<Button
					type="button"
					size="icon"
					variant="outline"
					onClick={() => onCopy(copyKey, value)}
					title="Copy"
				>
					{copied === copyKey ? (
						<Check className="h-4 w-4" />
					) : (
						<Copy className="h-4 w-4" />
					)}
				</Button>
			</div>
		</div>
	);
}

function InspectorSurface({
	children,
}: {
	children: ReactNode;
}) {
	return (
		<div className="overflow-hidden rounded-md border bg-card">{children}</div>
	);
}

function InspectorHeader({
	title,
	subtitle,
	icon,
	actions,
}: {
	title: string;
	subtitle: string;
	icon: ReactNode;
	actions?: ReactNode;
}) {
	return (
		<div className="flex flex-col gap-2 border-b px-3 py-3 lg:flex-row lg:items-center lg:justify-between">
			<div className="flex min-w-0 items-start gap-2">
				<div className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center text-muted-foreground">
					{icon}
				</div>
				<div className="min-w-0">
					<div className="text-sm font-semibold">{title}</div>
					<div className="mt-0.5 text-xs text-muted-foreground">{subtitle}</div>
				</div>
			</div>
			{actions && <div className="shrink-0">{actions}</div>}
		</div>
	);
}

function InspectorTinyStat({
	label,
	value,
}: {
	label: string;
	value: string | number;
}) {
	return (
		<div className="min-w-0 space-y-1 rounded-md border bg-background p-2">
			<div className="text-[10px] uppercase text-muted-foreground">{label}</div>
			<div className="min-w-0 truncate font-mono text-xs">{value}</div>
		</div>
	);
}

function McpInspectorPanel({
	endpointUrl,
	aliasUrl,
	auths,
	copied,
	onCopy,
}: {
	endpointUrl: string;
	aliasUrl: string | null;
	auths: IEventRemoteAuth[];
	copied: string | null;
	onCopy: (label: string, value: string) => void;
}) {
	const [target, setTarget] = useState<"event" | "alias">("event");
	const suggestedHeader = useMemo(
		() => suggestedInspectorHeader(auths),
		[auths],
	);
	const [headerName, setHeaderName] = useState("");
	const [headerValue, setHeaderValue] = useState("");
	const [inspecting, setInspecting] = useState(false);
	const [result, setResult] = useState<McpInspectorResult | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!aliasUrl && target === "alias") setTarget("event");
	}, [aliasUrl, target]);

	useEffect(() => {
		setHeaderName((current) => (current.trim() ? current : suggestedHeader));
	}, [suggestedHeader]);

	const targetUrl = target === "alias" && aliasUrl ? aliasUrl : endpointUrl;

	const inspect = async () => {
		setInspecting(true);
		setError(null);
		try {
			const nextResult = await inspectMcpEndpoint(
				targetUrl,
				headerName,
				headerValue,
			);
			setResult(nextResult);
			toast.success(
				`Inspection complete (${nextResult.tools.length} tools, ${nextResult.resources.length} resources, ${nextResult.prompts.length} prompts)`,
			);
		} catch (err) {
			const message =
				err instanceof Error ? err.message : "MCP inspection failed";
			setError(message);
			toast.error(message);
		} finally {
			setInspecting(false);
		}
	};

	const executeMethod = (method: string, params: Record<string, unknown>) =>
		executeMcpInspectorMethod(
			result?.endpoint ?? targetUrl,
			headerName,
			headerValue,
			method,
			params,
		);

	return (
		<InspectorSurface>
			<InspectorHeader
				title="Live MCP Inspector"
				subtitle="Inspect server shape, call tools, read resources, and render prompt output."
				icon={<Terminal className="h-4 w-4" />}
				actions={
					<div className="flex flex-wrap items-center gap-2">
						<Badge variant="outline" className="font-normal">
							{inspectorAuthLabel(auths)}
						</Badge>
						<Button
							type="button"
							size="sm"
							variant="outline"
							onClick={inspect}
							disabled={inspecting}
						>
							{inspecting ? (
								<Loader2 className="h-3 w-3 animate-spin" />
							) : (
								<Play className="h-3 w-3" />
							)}
							<span className="ml-1 text-xs">Inspect</span>
						</Button>
					</div>
				}
			/>

			<div className="space-y-3 p-3">
				<div className="grid gap-2 lg:grid-cols-[minmax(0,1fr)_12rem_12rem]">
					<div className="space-y-1.5">
						<Label className="flex items-center gap-1.5 text-xs uppercase tracking-wide text-muted-foreground">
							<Link2 className="h-3 w-3" />
							Endpoint
						</Label>
						<div className="flex items-center gap-2">
							<Input readOnly value={targetUrl} className="font-mono text-xs" />
							<Button
								type="button"
								size="icon"
								variant="outline"
								onClick={() => onCopy("inspector-endpoint", targetUrl)}
								title="Copy"
							>
								{copied === "inspector-endpoint" ? (
									<Check className="h-4 w-4" />
								) : (
									<Copy className="h-4 w-4" />
								)}
							</Button>
						</div>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs uppercase tracking-wide text-muted-foreground">
							Header
						</Label>
						<Input
							value={headerName}
							onChange={(event) => setHeaderName(event.target.value)}
							placeholder={suggestedHeader || "Authorization"}
							className="font-mono text-xs"
						/>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs uppercase tracking-wide text-muted-foreground">
							Value
						</Label>
						<Input
							value={headerValue}
							onChange={(event) => setHeaderValue(event.target.value)}
							placeholder={
								headerName.trim().toLowerCase() === "authorization"
									? "Bearer ..."
									: "header value"
							}
							type="password"
							className="font-mono text-xs"
						/>
					</div>
				</div>

				{aliasUrl && (
					<div className="inline-flex rounded-md border bg-muted/30 p-1">
						<Button
							type="button"
							size="sm"
							variant={target === "event" ? "secondary" : "ghost"}
							onClick={() => setTarget("event")}
							className="h-7 text-xs"
						>
							Event ID
						</Button>
						<Button
							type="button"
							size="sm"
							variant={target === "alias" ? "secondary" : "ghost"}
							onClick={() => setTarget("alias")}
							className="h-7 text-xs"
						>
							Alias
						</Button>
					</div>
				)}

				{error && (
					<div className="flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/10 p-2 text-xs text-destructive">
						<AlertCircle className="h-3 w-3 shrink-0" />
						<span className="min-w-0 break-words">{error}</span>
					</div>
				)}

				{result && (
					<McpInspectorResults result={result} onExecute={executeMethod} />
				)}
			</div>
		</InspectorSurface>
	);
}

function McpInspectorResults({
	result,
	onExecute,
}: {
	result: McpInspectorResult;
	onExecute: (
		method: string,
		params: Record<string, unknown>,
	) => Promise<McpInspectorExecution>;
}) {
	const serverName =
		typeof result.serverInfo?.name === "string"
			? result.serverInfo.name
			: "MCP server";
	const serverVersion =
		typeof result.serverInfo?.version === "string"
			? result.serverInfo.version
			: result.protocolVersion;

	return (
		<div className="space-y-3">
			<div className="grid gap-2 lg:grid-cols-4">
				<InspectorTinyStat
					label="Server"
					value={`${serverName} ${serverVersion}`}
				/>
				<InspectorTinyStat label="Session" value={result.sessionId} />
				<InspectorTinyStat label="Protocol" value={result.protocolVersion} />
				<InspectorTinyStat
					label="Latency"
					value={`${result.totalDurationMs} ms`}
				/>
			</div>

			<Tabs defaultValue="tools" className="space-y-2">
				<TabsList className="grid w-full grid-cols-4">
					<TabsTrigger value="tools" className="gap-2 text-xs">
						<span className="hidden sm:inline">Tools</span>
						<Badge variant="outline" className="ml-1 h-5 px-1.5 text-[10px]">
							{result.tools.length}
						</Badge>
					</TabsTrigger>
					<TabsTrigger value="resources" className="gap-2 text-xs">
						<span className="hidden sm:inline">Resources</span>
						<Badge variant="outline" className="ml-1 h-5 px-1.5 text-[10px]">
							{result.resources.length}
						</Badge>
					</TabsTrigger>
					<TabsTrigger value="prompts" className="gap-2 text-xs">
						<span className="hidden sm:inline">Prompts</span>
						<Badge variant="outline" className="ml-1 h-5 px-1.5 text-[10px]">
							{result.prompts.length}
						</Badge>
					</TabsTrigger>
					<TabsTrigger value="raw" className="gap-2 text-xs">
						<span className="hidden sm:inline">Raw</span>
					</TabsTrigger>
				</TabsList>
				<TabsContent value="tools" className="mt-0">
					<InspectorItemList
						items={result.tools}
						empty="No live tools returned."
						kind="tool"
						onExecute={onExecute}
					/>
				</TabsContent>
				<TabsContent value="resources" className="mt-0">
					<InspectorItemList
						items={result.resources}
						empty="No live resources returned."
						kind="resource"
						onExecute={onExecute}
					/>
				</TabsContent>
				<TabsContent value="prompts" className="mt-0">
					<InspectorItemList
						items={result.prompts}
						empty="No live prompts returned."
						kind="prompt"
						onExecute={onExecute}
					/>
				</TabsContent>
				<TabsContent value="raw" className="mt-0">
					<Textarea
						readOnly
						value={prettyJson({
							endpoint: result.endpoint,
							sessionId: result.sessionId,
							protocolVersion: result.protocolVersion,
							serverInfo: result.serverInfo,
							capabilities: result.capabilities,
							calls: result.calls,
						})}
						className="min-h-56 resize-y font-mono text-xs"
					/>
				</TabsContent>
			</Tabs>
		</div>
	);
}

function InspectorItemList({
	items,
	empty,
	kind,
	onExecute,
}: {
	items: Array<Record<string, unknown>>;
	empty: string;
	kind: "tool" | "resource" | "prompt";
	onExecute: (
		method: string,
		params: Record<string, unknown>,
	) => Promise<McpInspectorExecution>;
}) {
	if (items.length === 0) {
		return (
			<div className="flex items-center gap-2 rounded-md border border-dashed p-2 text-xs text-muted-foreground">
				<Info className="h-3 w-3" />
				<span>{empty}</span>
			</div>
		);
	}

	return (
		<ul className="grid gap-2">
			{items.map((item, index) => (
				<InspectorItemCard
					key={`${kind}-${inspectorItemName(item, kind, index)}-${index}`}
					item={item}
					index={index}
					kind={kind}
					onExecute={onExecute}
				/>
			))}
		</ul>
	);
}

function inspectorItemName(
	item: Record<string, unknown>,
	kind: "tool" | "resource" | "prompt",
	index: number,
): string {
	if (typeof item.name === "string") return item.name;
	if (typeof item.uri === "string") return item.uri;
	return `${kind}-${index + 1}`;
}

function isJsonObject(value: unknown): value is Record<string, unknown> {
	return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function promptArgumentDefaults(
	item: Record<string, unknown>,
): Record<string, unknown> {
	const args = item.arguments;
	if (!Array.isArray(args)) return {};
	const defaults: Record<string, unknown> = {};
	for (const arg of args) {
		if (!isJsonObject(arg)) continue;
		const name = arg.name;
		if (typeof name === "string" && name.trim()) {
			defaults[name] = "";
		}
	}
	return defaults;
}

function schemaProperties(
	schema: unknown,
): Record<string, Record<string, unknown>> {
	if (!isJsonObject(schema) || !isJsonObject(schema.properties)) return {};
	const properties: Record<string, Record<string, unknown>> = {};
	for (const [key, value] of Object.entries(schema.properties)) {
		properties[key] = isJsonObject(value) ? value : {};
	}
	return properties;
}

function schemaRequired(schema: unknown): Set<string> {
	if (!isJsonObject(schema) || !Array.isArray(schema.required)) {
		return new Set();
	}
	return new Set(
		schema.required.filter(
			(value): value is string => typeof value === "string",
		),
	);
}

function schemaType(schema: Record<string, unknown>): string {
	const type = schema.type;
	if (typeof type === "string") return type;
	if (Array.isArray(type)) {
		const nonNull = type.find((value) => value !== "null");
		if (typeof nonNull === "string") return nonNull;
	}
	if (Array.isArray(schema.enum)) return "string";
	if (schema.properties) return "object";
	if (schema.items) return "array";
	return "string";
}

function schemaLabel(name: string, schema: Record<string, unknown>): string {
	return typeof schema.title === "string" && schema.title.trim()
		? schema.title
		: name;
}

function schemaDescription(schema: Record<string, unknown>): string | null {
	return typeof schema.description === "string" && schema.description.trim()
		? schema.description
		: null;
}

function schemaEnumValues(schema: Record<string, unknown>): string[] {
	if (!Array.isArray(schema.enum)) return [];
	return schema.enum
		.filter((value) => value !== null && value !== undefined)
		.map((value) => String(value));
}

function schemaDefaultValue(schema: Record<string, unknown>): unknown {
	if (schema.default !== undefined) return schema.default;
	const type = schemaType(schema);
	if (type === "number" || type === "integer") return 0;
	if (type === "boolean") return false;
	if (type === "array") return [];
	if (type === "object") return {};
	return "";
}

function toolArgumentDefaults(
	item: Record<string, unknown>,
): Record<string, unknown> {
	const properties = schemaProperties(item.inputSchema);
	const defaults: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(properties)) {
		defaults[key] = schemaDefaultValue(value);
	}
	return defaults;
}

function promptArgumentSpecs(
	item: Record<string, unknown>,
): Array<Record<string, unknown>> {
	return Array.isArray(item.arguments)
		? item.arguments.filter(isJsonObject)
		: [];
}

function valueToDisplay(value: unknown): string {
	if (value === null || value === undefined) return "";
	if (typeof value === "string") return value;
	if (typeof value === "number" || typeof value === "boolean") {
		return String(value);
	}
	return prettyJson(value);
}

function isPrimitiveJson(value: unknown): boolean {
	return (
		value === undefined ||
		value === null ||
		typeof value === "string" ||
		typeof value === "number" ||
		typeof value === "boolean"
	);
}

function maybeParseJsonText(text: string): unknown | null {
	const trimmed = text.trim();
	if (!trimmed) return null;
	if (!/^[\[{"]|^-?\d|^(true|false|null)\b/.test(trimmed)) return null;
	try {
		return JSON.parse(trimmed);
	} catch {
		return null;
	}
}

function parseJsonObjectInput(
	input: string,
	fallback: Record<string, unknown>,
) {
	if (!input.trim()) return fallback;
	const parsed = JSON.parse(input);
	if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
		throw new Error("Arguments must be a JSON object");
	}
	return parsed as Record<string, unknown>;
}

function resourceContents(result: Record<string, unknown>) {
	const contents = result.contents;
	return Array.isArray(contents)
		? contents.filter(
				(item): item is Record<string, unknown> =>
					item && typeof item === "object" && !Array.isArray(item),
			)
		: [];
}

function resourceMime(content: Record<string, unknown>): string {
	const mime = content.mimeType ?? content.mime_type;
	return typeof mime === "string" && mime.trim()
		? mime
		: "application/octet-stream";
}

function isTextMime(mime: string): boolean {
	const lower = mime.toLowerCase();
	return (
		lower.startsWith("text/") ||
		lower.includes("json") ||
		lower.includes("xml") ||
		lower.includes("javascript") ||
		lower.includes("yaml")
	);
}

function decodeBase64Text(value: string): string {
	try {
		const binary = atob(value);
		const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
		return new TextDecoder().decode(bytes);
	} catch {
		return value;
	}
}

function resourceFilename(
	content: Record<string, unknown>,
	index: number,
): string {
	const uri = typeof content.uri === "string" ? content.uri : "";
	const name = uri.split(/[\\/]/).filter(Boolean).pop();
	return name || `resource-${index + 1}`;
}

function ResourcePreview({ result }: { result: Record<string, unknown> }) {
	const contents = resourceContents(result);
	if (contents.length === 0) {
		return (
			<pre className="max-h-72 overflow-auto rounded-md bg-muted/50 p-2 font-mono text-[11px]">
				{prettyJson(result)}
			</pre>
		);
	}

	return (
		<div className="space-y-2">
			{contents.map((content, index) => {
				const mime = resourceMime(content);
				const blob = typeof content.blob === "string" ? content.blob : null;
				const text = typeof content.text === "string" ? content.text : null;
				const uri = typeof content.uri === "string" ? content.uri : null;
				const dataUrl = blob ? `data:${mime};base64,${blob}` : null;
				const filename = resourceFilename(content, index);

				return (
					<div
						key={`${uri ?? filename}-${index}`}
						className="space-y-2 rounded-md border bg-background p-2"
					>
						<div className="flex min-w-0 flex-wrap items-center gap-2">
							<Badge variant="outline" className="font-mono text-[10px]">
								{mime}
							</Badge>
							{uri && (
								<code className="min-w-0 break-all font-mono text-[11px] text-muted-foreground">
									{uri}
								</code>
							)}
						</div>
						{text !== null ? (
							<SmartTextPreview text={text} />
						) : blob && mime.startsWith("image/") && dataUrl ? (
							<img
								src={dataUrl}
								alt={filename}
								className="max-h-96 max-w-full rounded-md border bg-muted object-contain"
							/>
						) : blob && mime.startsWith("audio/") && dataUrl ? (
							<audio src={dataUrl} controls className="w-full" />
						) : blob && mime.startsWith("video/") && dataUrl ? (
							<video
								src={dataUrl}
								controls
								className="max-h-96 w-full rounded-md border bg-muted"
							/>
						) : blob && mime === "application/pdf" && dataUrl ? (
							<iframe
								src={dataUrl}
								title={filename}
								className="h-96 w-full rounded-md border bg-muted"
							/>
						) : blob && isTextMime(mime) ? (
							<SmartTextPreview text={decodeBase64Text(blob)} />
						) : dataUrl ? (
							<a
								href={dataUrl}
								download={filename}
								className="inline-flex h-8 items-center gap-2 rounded-md border px-3 text-xs hover:bg-muted"
							>
								<Download className="h-3 w-3" />
								Download {filename}
							</a>
						) : (
							<pre className="max-h-72 overflow-auto rounded-md bg-muted/50 p-2 font-mono text-[11px]">
								{prettyJson(content)}
							</pre>
						)}
					</div>
				);
			})}
			<details>
				<summary className="cursor-pointer text-xs text-muted-foreground">
					Raw resource response
				</summary>
				<pre className="max-h-72 overflow-auto rounded-md bg-muted/50 p-2 font-mono text-[11px]">
					{prettyJson(result)}
				</pre>
			</details>
		</div>
	);
}

function JsonValuePreview({
	value,
	depth = 0,
}: {
	value: unknown;
	depth?: number;
}) {
	if (isPrimitiveJson(value)) {
		const tone =
			typeof value === "boolean"
				? "bg-blue-500/10 text-blue-700 dark:text-blue-300"
				: typeof value === "number"
					? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
					: value === null
						? "bg-muted text-muted-foreground"
						: "bg-muted/70 text-foreground";
		return (
			<span
				className={`inline-flex max-w-full rounded-sm px-1.5 py-0.5 font-mono text-[11px] ${tone}`}
			>
				<span className="min-w-0 break-all">
					{value === null
						? "null"
						: value === undefined
							? "undefined"
							: String(value)}
				</span>
			</span>
		);
	}

	if (Array.isArray(value)) {
		if (value.length === 0) {
			return <span className="text-xs text-muted-foreground">Empty array</span>;
		}
		const objectRows = value.filter(isJsonObject);
		const tableKeys = Array.from(
			new Set(
				objectRows.flatMap((row) =>
					Object.entries(row)
						.filter(([, field]) => isPrimitiveJson(field))
						.map(([key]) => key),
				),
			),
		).slice(0, 6);
		if (objectRows.length === value.length && tableKeys.length > 0) {
			return (
				<div className="max-h-80 overflow-auto rounded-md border">
					<table className="w-full border-collapse text-xs">
						<thead className="sticky top-0 bg-muted">
							<tr>
								{tableKeys.map((key) => (
									<th
										key={key}
										className="border-b px-2 py-1 text-left font-medium text-muted-foreground"
									>
										{key}
									</th>
								))}
							</tr>
						</thead>
						<tbody>
							{objectRows.map((row, index) => (
								<tr key={index} className="border-b last:border-b-0">
									{tableKeys.map((key) => (
										<td key={key} className="align-top px-2 py-1">
											<JsonValuePreview value={row[key]} depth={depth + 1} />
										</td>
									))}
								</tr>
							))}
						</tbody>
					</table>
				</div>
			);
		}
		return (
			<div className="space-y-2">
				{value.map((item, index) => (
					<div key={index} className="rounded-md border bg-background p-2">
						<div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
							Item {index + 1}
						</div>
						<JsonValuePreview value={item} depth={depth + 1} />
					</div>
				))}
			</div>
		);
	}

	if (isJsonObject(value)) {
		const entries = Object.entries(value);
		if (entries.length === 0) {
			return (
				<span className="text-xs text-muted-foreground">Empty object</span>
			);
		}
		return (
			<div className="grid gap-1.5">
				{entries.map(([key, field]) => {
					const nested = !isPrimitiveJson(field);
					return (
						<div
							key={key}
							className="grid gap-1 rounded-md border bg-background p-2 sm:grid-cols-[10rem_minmax(0,1fr)]"
						>
							<div className="min-w-0 break-all font-mono text-[11px] text-muted-foreground">
								{key}
							</div>
							<div className="min-w-0">
								{nested && depth > 1 ? (
									<details>
										<summary className="cursor-pointer text-xs text-muted-foreground">
											{Array.isArray(field)
												? `Array (${field.length})`
												: "Object"}
										</summary>
										<div className="mt-2">
											<JsonValuePreview value={field} depth={depth + 1} />
										</div>
									</details>
								) : (
									<JsonValuePreview value={field} depth={depth + 1} />
								)}
							</div>
						</div>
					);
				})}
			</div>
		);
	}

	return (
		<pre className="max-h-72 overflow-auto rounded-md bg-muted/50 p-2 font-mono text-[11px]">
			{prettyJson(value)}
		</pre>
	);
}

function SmartTextPreview({ text }: { text: string }) {
	const parsed = maybeParseJsonText(text);
	if (parsed !== null) {
		return (
			<div className="space-y-2 rounded-md border bg-muted/20 p-2">
				<div className="flex items-center gap-2 text-[10px] uppercase tracking-wide text-muted-foreground">
					<Braces className="h-3 w-3" />
					Parsed JSON
				</div>
				<JsonValuePreview value={parsed} />
				<details>
					<summary className="cursor-pointer text-xs text-muted-foreground">
						Raw text
					</summary>
					<pre className="max-h-72 overflow-auto rounded-md bg-muted/50 p-2 font-mono text-[11px]">
						{text}
					</pre>
				</details>
			</div>
		);
	}
	return (
		<pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-md bg-muted/50 p-2 font-mono text-[11px]">
			{text}
		</pre>
	);
}

function McpContentPreview({ content }: { content: unknown }) {
	if (typeof content === "string") return <SmartTextPreview text={content} />;
	if (!isJsonObject(content)) return <JsonValuePreview value={content} />;

	const type = typeof content.type === "string" ? content.type : "content";
	const text = typeof content.text === "string" ? content.text : null;
	const data =
		typeof content.data === "string"
			? content.data
			: typeof content.blob === "string"
				? content.blob
				: null;
	const mimeType =
		typeof content.mimeType === "string"
			? content.mimeType
			: typeof content.mime_type === "string"
				? content.mime_type
				: type === "image"
					? "image/png"
					: "application/octet-stream";
	const dataUrl = data ? `data:${mimeType};base64,${data}` : null;

	return (
		<div className="space-y-2 rounded-md border bg-background p-2">
			<div className="flex flex-wrap items-center gap-2">
				{mimeType.startsWith("image/") ? (
					<ImageIcon className="h-3 w-3 text-muted-foreground" />
				) : mimeType.startsWith("audio/") ? (
					<Music className="h-3 w-3 text-muted-foreground" />
				) : mimeType.startsWith("video/") ? (
					<Video className="h-3 w-3 text-muted-foreground" />
				) : (
					<FileText className="h-3 w-3 text-muted-foreground" />
				)}
				<Badge variant="outline" className="font-normal">
					{type}
				</Badge>
				{mimeType !== "application/octet-stream" && (
					<code className="font-mono text-[11px] text-muted-foreground">
						{mimeType}
					</code>
				)}
			</div>
			{text !== null ? (
				<SmartTextPreview text={text} />
			) : dataUrl && mimeType.startsWith("image/") ? (
				<img
					src={dataUrl}
					alt={type}
					className="max-h-96 max-w-full rounded-md border bg-muted object-contain"
				/>
			) : dataUrl && mimeType.startsWith("audio/") ? (
				<audio src={dataUrl} controls className="w-full" />
			) : dataUrl && mimeType.startsWith("video/") ? (
				<video
					src={dataUrl}
					controls
					className="max-h-96 w-full rounded-md border bg-muted"
				/>
			) : isJsonObject(content.resource) ? (
				<ResourcePreview result={{ contents: [content.resource] }} />
			) : (
				<JsonValuePreview value={content} />
			)}
		</div>
	);
}

function ToolResultPreview({ result }: { result: Record<string, unknown> }) {
	const content = Array.isArray(result.content) ? result.content : [];
	const structured = result.structuredContent ?? result.structured_content;
	const isError = result.isError === true;
	return (
		<div className="space-y-2">
			{isError && (
				<div className="flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/10 p-2 text-xs text-destructive">
					<AlertCircle className="h-3 w-3" />
					Tool reported an error result.
				</div>
			)}
			{structured !== undefined && (
				<div className="space-y-2 rounded-md border bg-muted/20 p-2">
					<div className="flex items-center gap-2 text-[10px] uppercase tracking-wide text-muted-foreground">
						<Database className="h-3 w-3" />
						Structured content
					</div>
					<JsonValuePreview value={structured} />
				</div>
			)}
			{content.length > 0 ? (
				<div className="space-y-2">
					{content.map((item, index) => (
						<McpContentPreview key={index} content={item} />
					))}
				</div>
			) : structured === undefined ? (
				<JsonValuePreview value={result} />
			) : null}
			<details>
				<summary className="cursor-pointer text-xs text-muted-foreground">
					Raw tool result
				</summary>
				<pre className="max-h-72 overflow-auto rounded-md bg-muted/50 p-2 font-mono text-[11px]">
					{prettyJson(result)}
				</pre>
			</details>
		</div>
	);
}

function PromptResultPreview({ result }: { result: Record<string, unknown> }) {
	const messages = Array.isArray(result.messages) ? result.messages : [];
	if (messages.length === 0) return <JsonValuePreview value={result} />;
	return (
		<div className="space-y-2">
			{messages.map((message, index) => {
				const role =
					isJsonObject(message) && typeof message.role === "string"
						? message.role
						: `message ${index + 1}`;
				const content = isJsonObject(message) ? message.content : message;
				return (
					<div
						key={index}
						className="space-y-2 rounded-md border bg-background p-2"
					>
						<div className="flex items-center gap-2">
							<MessageSquare className="h-3 w-3 text-muted-foreground" />
							<Badge variant="secondary" className="font-normal">
								{role}
							</Badge>
						</div>
						<McpContentPreview content={content} />
					</div>
				);
			})}
			<details>
				<summary className="cursor-pointer text-xs text-muted-foreground">
					Raw prompt result
				</summary>
				<pre className="max-h-72 overflow-auto rounded-md bg-muted/50 p-2 font-mono text-[11px]">
					{prettyJson(result)}
				</pre>
			</details>
		</div>
	);
}

function ExecutionResultPreview({
	execution,
}: {
	execution: McpInspectorExecution;
}) {
	if (execution.method === "resources/read") {
		return <ResourcePreview result={execution.result} />;
	}
	if (execution.method === "tools/call") {
		return <ToolResultPreview result={execution.result} />;
	}
	if (execution.method === "prompts/get") {
		return <PromptResultPreview result={execution.result} />;
	}
	return <JsonValuePreview value={execution.result} />;
}

function JsonFieldEditor({
	value,
	onChange,
}: {
	value: unknown;
	onChange: (value: unknown) => void;
}) {
	const [text, setText] = useState(() => prettyJson(value));
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setText(prettyJson(value));
		setError(null);
	}, [value]);

	return (
		<div className="space-y-1">
			<Textarea
				value={text}
				onChange={(event) => {
					const next = event.target.value;
					setText(next);
					try {
						onChange(JSON.parse(next));
						setError(null);
					} catch {
						setError("Invalid JSON");
					}
				}}
				className="min-h-20 resize-y font-mono text-xs"
			/>
			{error && <div className="text-[11px] text-destructive">{error}</div>}
		</div>
	);
}

function SchemaArgumentForm({
	schema,
	value,
	onChange,
}: {
	schema: unknown;
	value: Record<string, unknown>;
	onChange: (value: Record<string, unknown>) => void;
}) {
	const properties = schemaProperties(schema);
	const required = schemaRequired(schema);
	const entries = Object.entries(properties);
	if (entries.length === 0) {
		return (
			<div className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
				No declared input fields.
			</div>
		);
	}

	const update = (name: string, nextValue: unknown) => {
		onChange({ ...value, [name]: nextValue });
	};

	return (
		<div className="grid gap-3 md:grid-cols-2">
			{entries.map(([name, fieldSchema]) => (
				<SchemaArgumentField
					key={name}
					name={name}
					schema={fieldSchema}
					required={required.has(name)}
					value={value[name]}
					onChange={(nextValue) => update(name, nextValue)}
				/>
			))}
		</div>
	);
}

function SchemaArgumentField({
	name,
	schema,
	required,
	value,
	onChange,
}: {
	name: string;
	schema: Record<string, unknown>;
	required: boolean;
	value: unknown;
	onChange: (value: unknown) => void;
}) {
	const type = schemaType(schema);
	const enumValues = schemaEnumValues(schema);
	const description = schemaDescription(schema);
	const label = schemaLabel(name, schema);

	return (
		<div className="space-y-1.5 rounded-md border bg-background p-2">
			<div className="flex min-w-0 items-center justify-between gap-2">
				<Label className="min-w-0 break-all text-xs font-medium">
					{label}
					{required && <span className="ml-1 text-destructive">*</span>}
				</Label>
				<Badge variant="outline" className="shrink-0 font-mono text-[10px]">
					{type}
				</Badge>
			</div>
			{description && (
				<p className="text-[11px] leading-4 text-muted-foreground">
					{description}
				</p>
			)}
			{enumValues.length > 0 ? (
				<Select
					value={
						value === undefined || value === null ? "__empty__" : String(value)
					}
					onValueChange={(nextValue) =>
						onChange(nextValue === "__empty__" ? "" : nextValue)
					}
				>
					<SelectTrigger className="h-8 text-xs">
						<SelectValue placeholder="Select value" />
					</SelectTrigger>
					<SelectContent>
						{!required && <SelectItem value="__empty__">Empty</SelectItem>}
						{enumValues.map((option) => (
							<SelectItem key={option} value={option}>
								{option}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : type === "boolean" ? (
				<div className="flex h-8 items-center gap-2">
					<Switch checked={Boolean(value)} onCheckedChange={onChange} />
					<span className="text-xs text-muted-foreground">
						{Boolean(value) ? "true" : "false"}
					</span>
				</div>
			) : type === "number" || type === "integer" ? (
				<Input
					type="number"
					step={type === "integer" ? 1 : "any"}
					value={valueToDisplay(value)}
					onChange={(event) => {
						const next = event.target.value;
						onChange(next === "" ? "" : Number(next));
					}}
					className="h-8 font-mono text-xs"
				/>
			) : type === "array" || type === "object" ? (
				<JsonFieldEditor
					value={value ?? schemaDefaultValue(schema)}
					onChange={onChange}
				/>
			) : (
				<Input
					value={valueToDisplay(value)}
					onChange={(event) => onChange(event.target.value)}
					className="h-8 font-mono text-xs"
				/>
			)}
		</div>
	);
}

function PromptArgumentForm({
	item,
	value,
	onChange,
}: {
	item: Record<string, unknown>;
	value: Record<string, unknown>;
	onChange: (value: Record<string, unknown>) => void;
}) {
	const specs = promptArgumentSpecs(item);
	if (specs.length === 0) {
		return (
			<div className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
				No prompt arguments.
			</div>
		);
	}
	return (
		<div className="grid gap-3 md:grid-cols-2">
			{specs.map((spec) => {
				const name = typeof spec.name === "string" ? spec.name : "";
				const description = schemaDescription(spec);
				const required = spec.required === true;
				return (
					<div
						key={name}
						className="space-y-1.5 rounded-md border bg-background p-2"
					>
						<Label className="min-w-0 break-all text-xs font-medium">
							{name}
							{required && <span className="ml-1 text-destructive">*</span>}
						</Label>
						{description && (
							<p className="text-[11px] leading-4 text-muted-foreground">
								{description}
							</p>
						)}
						<Input
							value={valueToDisplay(value[name])}
							onChange={(event) =>
								onChange({ ...value, [name]: event.target.value })
							}
							className="h-8 font-mono text-xs"
						/>
					</div>
				);
			})}
		</div>
	);
}

function InspectorItemCard({
	item,
	index,
	kind,
	onExecute,
}: {
	item: Record<string, unknown>;
	index: number;
	kind: "tool" | "resource" | "prompt";
	onExecute: (
		method: string,
		params: Record<string, unknown>,
	) => Promise<McpInspectorExecution>;
}) {
	const name = inspectorItemName(item, kind, index);
	const description =
		typeof item.description === "string" ? item.description : null;
	const schema = kind === "tool" ? item.inputSchema : null;
	const detail =
		kind === "resource"
			? {
					uri: item.uri,
					mimeType: item.mimeType,
				}
			: kind === "prompt"
				? { arguments: item.arguments }
				: null;
	const defaultArguments =
		kind === "tool"
			? toolArgumentDefaults(item)
			: kind === "prompt"
				? promptArgumentDefaults(item)
				: {};
	const [argumentsValue, setArgumentsValue] =
		useState<Record<string, unknown>>(defaultArguments);
	const [argumentsText, setArgumentsText] = useState(
		prettyJson(defaultArguments),
	);
	const [rawMode, setRawMode] = useState(false);
	const [running, setRunning] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [execution, setExecution] = useState<McpInspectorExecution | null>(
		null,
	);

	const setStructuredArguments = (nextValue: Record<string, unknown>) => {
		setArgumentsValue(nextValue);
		setArgumentsText(prettyJson(nextValue));
	};

	const run = async () => {
		setRunning(true);
		setError(null);
		try {
			let method = "tools/call";
			let params: Record<string, unknown>;
			if (kind === "tool") {
				params = {
					name,
					arguments: rawMode
						? parseJsonObjectInput(argumentsText, defaultArguments)
						: argumentsValue,
				};
			} else if (kind === "resource") {
				method = "resources/read";
				params = { uri: item.uri };
			} else {
				method = "prompts/get";
				params = {
					name,
					arguments: rawMode
						? parseJsonObjectInput(argumentsText, defaultArguments)
						: argumentsValue,
				};
			}
			const nextExecution = await onExecute(method, params);
			setExecution(nextExecution);
		} catch (err) {
			const message = err instanceof Error ? err.message : "MCP request failed";
			setError(message);
			toast.error(message);
		} finally {
			setRunning(false);
		}
	};

	return (
		<li className="space-y-2 rounded-md border bg-background p-2 text-xs">
			<div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
				<div className="flex min-w-0 flex-wrap items-center gap-2">
					<span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted">
						{kind === "tool" ? (
							<Wrench className="h-3.5 w-3.5 text-muted-foreground" />
						) : kind === "resource" ? (
							<FileText className="h-3.5 w-3.5 text-muted-foreground" />
						) : (
							<MessageSquare className="h-3.5 w-3.5 text-muted-foreground" />
						)}
					</span>
					<code className="min-w-0 break-all font-mono text-sm">{name}</code>
					<Badge variant="secondary" className="font-normal">
						{kind}
					</Badge>
				</div>
				<Button
					type="button"
					size="sm"
					variant="outline"
					onClick={run}
					disabled={running}
				>
					{running ? (
						<Loader2 className="h-3 w-3 animate-spin" />
					) : (
						<Play className="h-3 w-3" />
					)}
					<span className="ml-1 text-xs">
						{kind === "tool" ? "Call" : kind === "resource" ? "Read" : "Get"}
					</span>
				</Button>
			</div>
			{description && (
				<p className="text-xs text-muted-foreground">{description}</p>
			)}
			<div className="space-y-2">
				{(kind === "tool" || kind === "prompt") && (
					<div className="space-y-2 rounded-md border bg-muted/20 p-2">
						<div className="flex flex-wrap items-center justify-between gap-2">
							<div className="flex items-center gap-2 text-[10px] uppercase tracking-wide text-muted-foreground">
								{rawMode ? (
									<Code2 className="h-3 w-3" />
								) : (
									<Braces className="h-3 w-3" />
								)}
								Arguments
							</div>
							<Button
								type="button"
								size="sm"
								variant="ghost"
								onClick={() => setRawMode((current) => !current)}
								className="h-7 text-xs"
							>
								{rawMode ? "Use form" : "Edit JSON"}
							</Button>
						</div>
						{rawMode ? (
							<Textarea
								value={argumentsText}
								onChange={(event) => setArgumentsText(event.target.value)}
								className="min-h-24 resize-y font-mono text-xs"
							/>
						) : kind === "tool" ? (
							<SchemaArgumentForm
								schema={schema}
								value={argumentsValue}
								onChange={setStructuredArguments}
							/>
						) : (
							<PromptArgumentForm
								item={item}
								value={argumentsValue}
								onChange={setStructuredArguments}
							/>
						)}
					</div>
				)}
				{kind === "resource" && detail ? (
					<div className="grid gap-2 rounded-md border bg-muted/20 p-2 sm:grid-cols-2">
						{Object.entries(detail)
							.filter(([, value]) => value !== undefined && value !== null)
							.map(([key, value]) => (
								<div key={key} className="min-w-0">
									<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
										{key}
									</div>
									<code className="break-all font-mono text-[11px]">
										{String(value)}
									</code>
								</div>
							))}
					</div>
				) : null}
				{error && (
					<div className="flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/10 p-2 text-xs text-destructive">
						<AlertCircle className="h-3 w-3 shrink-0" />
						<span className="min-w-0 break-words">{error}</span>
					</div>
				)}
				{execution && (
					<div className="space-y-2 rounded-md border bg-card p-2">
						<div className="flex flex-wrap items-center gap-2 border-b pb-2">
							<Badge variant="outline" className="font-normal">
								{execution.method}
							</Badge>
							<span className="text-xs text-muted-foreground">
								{execution.totalDurationMs} ms
							</span>
						</div>
						<ExecutionResultPreview execution={execution} />
					</div>
				)}
			</div>
		</li>
	);
}

export function McpConfig({
	config,
	onConfigUpdate,
	eventId,
	appId,
}: IConfigInterfaceProps) {
	useEffect(() => {
		if (!(config as McpSink)?.sink_type) {
			onConfigUpdate?.({
				...(config as McpSink),
				sink_type: "mcp",
			} as any);
		}
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, []);

	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const baseUrl = useMemo(
		() => getApiOrigin(profile.data ?? null),
		[profile.data?.hub, profile.data?.secure],
	);

	const endpointUrl = eventId ? `${baseUrl}/m/${eventId}` : null;

	const registrations = useInvoke(
		(backend.eventState.listEventRegistrations ??
			listNoEventRegistrations) as any,
		backend.eventState,
		[appId ?? "", eventId ?? ""],
		Boolean(appId && eventId && backend.eventState.listEventRegistrations),
	);
	const aliases = useInvoke<IEventAlias[], [string, string]>(
		(backend.eventState.listEventAliases ?? listNoEventAliases) as any,
		backend.eventState,
		[appId ?? "", eventId ?? ""],
		Boolean(appId && eventId && backend.eventState.listEventAliases),
	);

	useEffect(() => {
		if (!appId || !eventId) return;
		const id = window.setInterval(() => {
			registrations.refetch();
			aliases.refetch();
		}, 4000);
		return () => window.clearInterval(id);
	}, [aliases.refetch, appId, eventId, registrations.refetch]);

	const registrationData = registrations.data as
		| IListRegistrationsResponse
		| undefined;
	const registrationRows = registrationData?.registrations ?? [];
	const authRows = registrationData?.auths ?? [];
	const mcpRegs = registrationRows.filter((r) => r.kind.startsWith("mcp_"));
	const knownAuthIds = new Set(authRows.map((auth) => auth.id));
	const missingAuthIds = Array.from(
		new Set(
			mcpRegs
				.map((registration) => registration.auth_id)
				.filter((authId): authId is string => Boolean(authId)),
		),
	).filter((authId) => !knownAuthIds.has(authId));

	const [copied, setCopied] = useState<string | null>(null);
	const [setupBusy, setSetupBusy] = useState(false);
	const currentAlias = aliases.data?.[0]?.slug ?? "";
	const [aliasInput, setAliasInput] = useState("");
	const [aliasBusy, setAliasBusy] = useState(false);
	const [aliasError, setAliasError] = useState<string | null>(null);
	const aliasUrl = currentAlias ? `${baseUrl}/m/${currentAlias}` : null;

	useEffect(() => {
		setAliasInput(currentAlias);
	}, [currentAlias]);

	const copy = async (label: string, value: string) => {
		try {
			await navigator.clipboard.writeText(value);
			setCopied(label);
			setTimeout(() => setCopied(null), 1500);
		} catch {
			toast.error("Failed to copy");
		}
	};

	const refreshSetup = async () => {
		setSetupBusy(true);
		try {
			if (appId && eventId && backend.eventState.setupEvent) {
				const response = await backend.eventState.setupEvent(
					appId,
					eventId,
					true,
				);
				toast.success(
					`Setup refreshed (${response.registrations_written} registrations)`,
				);
			}
			await Promise.all([registrations.refetch(), aliases.refetch()]);
		} catch (error) {
			const message =
				error instanceof Error ? error.message : "Failed to refresh setup";
			toast.error(message);
			await Promise.allSettled([registrations.refetch(), aliases.refetch()]);
		} finally {
			setSetupBusy(false);
		}
	};

	const saveAlias = async () => {
		if (!appId || !eventId || !backend.eventState.upsertEventAlias) return;
		const slug = aliasInput.trim().toLowerCase();
		if (!slug) return;
		setAliasBusy(true);
		setAliasError(null);
		try {
			await backend.eventState.upsertEventAlias(appId, eventId, slug);
			toast.success("Alias saved");
			await aliases.refetch();
		} catch (error) {
			const message =
				error instanceof Error ? error.message : "Failed to save alias";
			setAliasError(message);
			toast.error(message);
		} finally {
			setAliasBusy(false);
		}
	};

	const deleteAlias = async () => {
		if (
			!appId ||
			!eventId ||
			!currentAlias ||
			!backend.eventState.deleteEventAlias
		)
			return;
		setAliasBusy(true);
		setAliasError(null);
		try {
			await backend.eventState.deleteEventAlias(appId, eventId, currentAlias);
			toast.success("Alias removed");
			await aliases.refetch();
		} catch (error) {
			const message =
				error instanceof Error ? error.message : "Failed to remove alias";
			setAliasError(message);
			toast.error(message);
		} finally {
			setAliasBusy(false);
		}
	};

	return (
		<div className="w-full space-y-4">
			<Alert>
				<Server className="h-4 w-4" />
				<AlertTitle className="flex items-center gap-2">
					MCP Server
					<span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs">
						<Cloud className="h-3 w-3" /> Remote only
					</span>
				</AlertTitle>
				<AlertDescription>
					Exposes the workflow as a remote Model Context Protocol server. Tools,
					resources, prompts and authentication are declared inside the board
					and mounted at <code>/m/&#123;event_id&#125;</code>.
				</AlertDescription>
			</Alert>

			{endpointUrl ? (
				<div className="space-y-3 rounded-md border bg-muted/30 p-3">
					<EndpointField
						label="Streamable HTTP"
						value={endpointUrl}
						copyKey="streamable"
						copied={copied}
						onCopy={copy}
					/>
					<EndpointField
						label="Legacy SSE"
						value={endpointUrl}
						copyKey="sse"
						copied={copied}
						onCopy={copy}
					/>
				</div>
			) : (
				<p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
					Save the event to see its endpoint URL.
				</p>
			)}

			{eventId && appId && backend.eventState.listEventAliases && (
				<InspectorSurface>
					<InspectorHeader
						title="Public Alias"
						subtitle="Give this MCP server a stable, readable mount path."
						icon={<Link2 className="h-4 w-4" />}
					/>
					<div className="space-y-3 p-3">
						<div className="flex flex-col gap-2 lg:flex-row lg:items-center">
							<div className="shrink-0 rounded-md border bg-muted px-2 py-2 font-mono text-xs text-muted-foreground">
								{baseUrl}/m/
							</div>
							<Input
								value={aliasInput}
								onChange={(event) => {
									setAliasError(null);
									setAliasInput(event.target.value);
								}}
								placeholder="my-mcp"
								className="font-mono text-xs"
								disabled={aliasBusy}
							/>
							<Button
								type="button"
								variant="outline"
								onClick={saveAlias}
								disabled={
									aliasBusy ||
									!backend.eventState.upsertEventAlias ||
									!aliasInput.trim() ||
									aliasInput.trim().toLowerCase() === currentAlias
								}
							>
								{aliasBusy ? (
									<Loader2 className="h-4 w-4 animate-spin" />
								) : (
									<Check className="h-4 w-4" />
								)}
								Save
							</Button>
							<Button
								type="button"
								size="icon"
								variant="ghost"
								onClick={deleteAlias}
								disabled={
									aliasBusy ||
									!currentAlias ||
									!backend.eventState.deleteEventAlias
								}
								title="Remove alias"
							>
								<Trash2 className="h-4 w-4" />
							</Button>
						</div>
						{aliasUrl && (
							<div className="flex items-center gap-2">
								<Input
									readOnly
									value={aliasUrl}
									className="font-mono text-xs"
								/>
								<Button
									type="button"
									size="icon"
									variant="outline"
									onClick={() => copy("alias", aliasUrl)}
									title="Copy"
								>
									{copied === "alias" ? (
										<Check className="h-4 w-4" />
									) : (
										<Copy className="h-4 w-4" />
									)}
								</Button>
							</div>
						)}
						{aliasError && (
							<div className="flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/10 p-2 text-xs text-destructive">
								<AlertCircle className="h-3 w-3 shrink-0" />
								<span className="truncate">{aliasError}</span>
							</div>
						)}
					</div>
				</InspectorSurface>
			)}

			{eventId && appId && (
				<McpSetupPanel
					version={registrationData?.event_version ?? null}
					auths={authRows}
					missingAuthIds={missingAuthIds}
					registrations={mcpRegs}
					loading={registrations.isLoading}
					fetching={registrations.isFetching || setupBusy}
					error={registrations.error?.message ?? null}
					onRefresh={refreshSetup}
				/>
			)}

			{endpointUrl && (
				<McpInspectorPanel
					endpointUrl={endpointUrl}
					aliasUrl={aliasUrl}
					auths={authRows}
					copied={copied}
					onCopy={copy}
				/>
			)}

			<p className="flex items-center gap-1 text-xs text-muted-foreground">
				<Info className="h-3 w-3" />
				Save the event to trigger remote setup and refresh the registered MCP
				server shape.
			</p>
		</div>
	);
}

interface McpSetupPanelProps {
	version: string | null;
	auths: IEventRemoteAuth[];
	missingAuthIds: string[];
	registrations: IEventRegistration[];
	loading: boolean;
	fetching: boolean;
	error: string | null;
	onRefresh: () => Promise<void> | void;
}

function McpSetupPanel({
	version,
	auths,
	missingAuthIds,
	registrations,
	loading,
	fetching,
	error,
	onRefresh,
}: McpSetupPanelProps) {
	const authById = new Map(auths.map((auth) => [auth.id, auth]));
	const rawConfig = registrationExtras(
		registrations.find((registration) => registration.kind === "mcp_raw"),
	);
	const fallbackFunctionRefs =
		registrations.some((registration) => registration.kind === "mcp_tool") ||
		!Array.isArray(rawConfig.function_refs)
			? []
			: rawConfig.function_refs.filter(
					(value: unknown): value is string => typeof value === "string",
				);
	const groups = ["mcp_tool", "mcp_resource", "mcp_prompt", "mcp_raw"]
		.map((kind) => ({
			kind,
			label: mcpKindLabel(kind),
			registrations: registrations.filter(
				(registration) => registration.kind === kind,
			),
		}))
		.filter((group) => group.registrations.length > 0);
	const authCount = auths.length + missingAuthIds.length;
	const toolCount =
		registrations.filter((registration) => registration.kind === "mcp_tool")
			.length || fallbackFunctionRefs.length;
	const resourceCount = registrations.filter(
		(registration) => registration.kind === "mcp_resource",
	).length;
	const promptCount = registrations.filter(
		(registration) => registration.kind === "mcp_prompt",
	).length;

	return (
		<InspectorSurface>
			<InspectorHeader
				title="Current MCP Setup"
				subtitle="The registered server shape generated from the board."
				icon={<Server className="h-4 w-4" />}
				actions={
					<Button
						type="button"
						size="sm"
						variant="outline"
						onClick={onRefresh}
						disabled={fetching}
					>
						{fetching ? (
							<Loader2 className="h-3 w-3 animate-spin" />
						) : (
							<RefreshCw className="h-3 w-3" />
						)}
						<span className="ml-1 text-xs">Refresh</span>
					</Button>
				}
			/>
			<div className="space-y-3 p-3">
				{error && (
					<div className="flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/10 p-2 text-xs text-destructive">
						<AlertCircle className="h-3 w-3 shrink-0" />
						<span className="truncate">{error}</span>
					</div>
				)}

				<div className="grid gap-2 lg:grid-cols-4">
					<SetupMetric
						label="Version"
						value={version ?? "not registered"}
						mono
					/>
					<SetupMetric
						label="Authentication"
						value={authCount === 0 ? "none" : `${authCount} configured`}
					/>
					<SetupMetric
						label="Tools"
						value={loading ? "loading" : `${toolCount} registered`}
					/>
					<SetupMetric
						label="Resources / Prompts"
						value={loading ? "loading" : `${resourceCount} / ${promptCount}`}
					/>
				</div>

				{authCount > 0 && (
					<div className="space-y-2">
						<div className="text-xs font-semibold">Authentication</div>
						{auths.map((auth) => (
							<div
								key={auth.id}
								className="space-y-2 rounded-md border bg-background p-2 text-xs"
							>
								<div className="flex min-w-0 flex-wrap items-center gap-2">
									<Badge variant="secondary" className="font-normal">
										{authLabel(auth.config)}
									</Badge>
									<code className="min-w-0 break-all font-mono text-muted-foreground">
										{auth.node_id}
									</code>
								</div>
								{authConfigEntries(auth.config).length > 0 && (
									<div className="grid gap-1">
										{authConfigEntries(auth.config).map(([key, value]) => (
											<DetailRow
												key={key}
												label={key}
												value={formatConfigValue(value, key)}
											/>
										))}
									</div>
								)}
							</div>
						))}
						{missingAuthIds.map((authId) => (
							<div
								key={authId}
								className="flex items-center justify-between gap-2 rounded-md border bg-background p-2 text-xs"
							>
								<span className="text-muted-foreground">
									Auth is linked to MCP entries, but details are not available.
								</span>
								<code className="truncate">{authId}</code>
							</div>
						))}
					</div>
				)}

				<div className="space-y-2">
					<div className="text-xs font-semibold">MCP Entries</div>
					{loading ? (
						<div className="flex items-center gap-2 rounded-md border bg-background p-2 text-xs text-muted-foreground">
							<Loader2 className="h-3 w-3 animate-spin" />
							Loading setup...
						</div>
					) : groups.length === 0 && fallbackFunctionRefs.length === 0 ? (
						<div className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
							No MCP setup registered yet.
						</div>
					) : (
						<div className="space-y-2">
							{fallbackFunctionRefs.length > 0 && (
								<div className="rounded-md border bg-background">
									<div className="flex items-center justify-between border-b px-2 py-1.5">
										<span className="text-xs font-medium">Tool References</span>
										<Badge variant="outline" className="font-mono text-[10px]">
											{fallbackFunctionRefs.length}
										</Badge>
									</div>
									<ul className="divide-y">
										{fallbackFunctionRefs.map((nodeId) => (
											<li key={nodeId} className="p-2 text-xs">
												<DetailRow label="handler" value={nodeId} />
											</li>
										))}
									</ul>
								</div>
							)}
							{groups.map((group) => (
								<div
									key={group.kind}
									className="rounded-md border bg-background"
								>
									<div className="flex items-center justify-between border-b px-2 py-1.5">
										<span className="text-xs font-medium">{group.label}</span>
										<Badge variant="outline" className="font-mono text-[10px]">
											{group.registrations.length}
										</Badge>
									</div>
									<ul className="divide-y">
										{group.registrations.map((registration) => {
											const auth = registration.auth_id
												? authById.get(registration.auth_id)
												: null;
											const authText = registration.auth_id
												? auth
													? authLabel(auth.config)
													: "configured"
												: "none";
											const details = mcpDetailEntries(registration);

											return (
												<li
													key={registration.id}
													className="space-y-2 p-2 text-xs"
												>
													<div className="flex min-w-0 flex-wrap items-center gap-2">
														<Badge variant="secondary" className="font-normal">
															{mcpEntryLabel(registration.kind)}
														</Badge>
														<code className="min-w-0 break-all font-mono text-sm">
															{registration.path}
														</code>
														<Badge
															variant={
																registration.auth_id ? "secondary" : "outline"
															}
															className="max-w-full whitespace-normal text-left font-normal leading-tight"
														>
															auth: {authText}
														</Badge>
													</div>
													{details.length > 0 && (
														<div className="grid gap-1.5 lg:grid-cols-2">
															{details.map(([key, value]) => (
																<DetailRow
																	key={`${registration.id}-${key}`}
																	label={key}
																	value={value}
																/>
															))}
														</div>
													)}
												</li>
											);
										})}
									</ul>
								</div>
							))}
						</div>
					)}
				</div>
			</div>
		</InspectorSurface>
	);
}

function SetupMetric({
	label,
	value,
	mono = false,
}: {
	label: string;
	value: string;
	mono?: boolean;
}) {
	return (
		<div className="space-y-1 rounded-md border bg-background p-2">
			<div className="text-[10px] uppercase text-muted-foreground">{label}</div>
			<div className={mono ? "break-all font-mono text-xs" : "text-xs"}>
				{value}
			</div>
		</div>
	);
}
