"use client";

import {
	BookOpenIcon,
	CheckIcon,
	ChevronDownIcon,
	ChevronRightIcon,
	CopyIcon,
	ExternalLinkIcon,
	KeyIcon,
	PackageIcon,
	ServerIcon,
	ShieldIcon,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { codeToHtml } from "shiki";
import { toast } from "sonner";

import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../ui/collapsible";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";

interface OpenApiPath {
	method: string;
	path: string;
	summary?: string;
	description?: string;
	tag?: string;
	parameters?: Array<{
		name: string;
		in: string;
		required?: boolean;
		schema?: { type?: string };
		description?: string;
	}>;
	requestBody?: {
		content?: Record<string, { schema?: Record<string, unknown> }>;
	};
	responses?: Record<
		string,
		{ description?: string; content?: Record<string, unknown> }
	>;
	security?: Array<Record<string, string[]>>;
}

interface OpenApiSpec {
	openapi?: string;
	info?: { title?: string; version?: string; description?: string };
	paths?: Record<string, Record<string, Record<string, unknown>>>;
	components?: { schemas?: Record<string, unknown> };
	servers?: Array<{ url?: string; description?: string }>;
	tags?: Array<{ name?: string; description?: string }>;
}

const HTTP_METHOD_COLORS: Record<string, string> = {
	get: "bg-blue-500/10 text-blue-600 dark:text-blue-400 border-blue-500/20",
	post: "bg-green-500/10 text-green-600 dark:text-green-400 border-green-500/20",
	put: "bg-amber-500/10 text-amber-600 dark:text-amber-400 border-amber-500/20",
	patch:
		"bg-orange-500/10 text-orange-600 dark:text-orange-400 border-orange-500/20",
	delete: "bg-red-500/10 text-red-600 dark:text-red-400 border-red-500/20",
};

function CopyButton({ text }: { text: string }) {
	const [copied, setCopied] = useState(false);

	const handleCopy = useCallback(async () => {
		await navigator.clipboard.writeText(text);
		setCopied(true);
		setTimeout(() => setCopied(false), 2000);
	}, [text]);

	return (
		<Button
			variant="ghost"
			size="icon"
			className="h-6 w-6 shrink-0"
			onClick={handleCopy}
		>
			{copied ? (
				<CheckIcon className="h-3 w-3" />
			) : (
				<CopyIcon className="h-3 w-3" />
			)}
		</Button>
	);
}

function CodeBlock({
	code,
	language,
}: {
	code: string;
	language?: string;
}) {
	const [html, setHtml] = useState<string>("");
	const containerRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		let cancelled = false;
		const lang = language === "http" ? "text" : (language ?? "text");
		codeToHtml(code, {
			lang,
			themes: { light: "github-light", dark: "github-dark" },
			defaultColor: false,
		})
			.then((result) => {
				if (!cancelled) setHtml(result);
			})
			.catch(() => {
				if (!cancelled) setHtml("");
			});
		return () => {
			cancelled = true;
		};
	}, [code, language]);

	return (
		<div className="relative group rounded-md border bg-muted/50 overflow-hidden">
			<div className="flex items-center justify-between px-3 py-1.5 border-b bg-muted/30">
				{language && (
					<span className="text-xs text-muted-foreground font-mono">
						{language}
					</span>
				)}
				<CopyButton text={code} />
			</div>
			{html ? (
				<div
					ref={containerRef}
					className="shiki-wrapper [&_pre]:bg-transparent! [&_pre]:p-3 [&_pre]:overflow-x-auto [&_pre]:text-sm [&_pre]:leading-relaxed [&_code]:font-mono"
					// biome-ignore lint: shiki output is trusted static HTML from our own code strings
					dangerouslySetInnerHTML={{ __html: html }}
				/>
			) : (
				<pre className="p-3 overflow-x-auto text-sm leading-relaxed">
					<code className="font-mono text-foreground/90">{code}</code>
				</pre>
			)}
		</div>
	);
}

function EndpointRow({
	endpoint,
	appId,
	baseUrl,
}: {
	endpoint: OpenApiPath;
	appId: string;
	baseUrl: string;
}) {
	const [open, setOpen] = useState(false);
	const filledPath = endpoint.path.replace("{app_id}", appId);
	const fullUrl = `${baseUrl}${filledPath}`;

	const nonAppParams = useMemo(
		() =>
			(endpoint.parameters ?? []).filter(
				(p) => p.name !== "app_id",
			),
		[endpoint.parameters],
	);

	return (
		<Collapsible open={open} onOpenChange={setOpen}>
			<CollapsibleTrigger asChild>
				<button
					type="button"
					className="flex items-center gap-3 w-full px-4 py-3 text-left hover:bg-muted/50 transition-colors rounded-md"
				>
					{open ? (
						<ChevronDownIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
					) : (
						<ChevronRightIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
					)}
					<Badge
						variant="outline"
						className={`uppercase font-mono text-xs min-w-15 justify-center ${HTTP_METHOD_COLORS[endpoint.method] ?? ""}`}
					>
						{endpoint.method}
					</Badge>
					<span className="font-mono text-sm truncate flex-1">
						{filledPath}
					</span>
					{endpoint.summary && (
						<span className="text-sm text-muted-foreground truncate max-w-75">
							{endpoint.summary}
						</span>
					)}
				</button>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<div className="ml-11 mr-4 mb-4 space-y-3">
					{endpoint.description && (
						<p className="text-sm text-muted-foreground">
							{endpoint.description}
						</p>
					)}

					<div className="flex items-center gap-2">
						<span className="text-xs text-muted-foreground">Full URL:</span>
						<code className="text-xs font-mono bg-muted rounded px-2 py-0.5 flex-1 truncate">
							{fullUrl}
						</code>
						<CopyButton text={fullUrl} />
					</div>

					{nonAppParams.length > 0 && (
						<div className="space-y-1.5">
							<span className="text-xs font-medium">Parameters</span>
							<div className="rounded-md border overflow-hidden">
								<table className="w-full text-sm">
									<thead>
										<tr className="border-b bg-muted/30">
											<th className="text-left px-3 py-1.5 text-xs font-medium text-muted-foreground">
												Name
											</th>
											<th className="text-left px-3 py-1.5 text-xs font-medium text-muted-foreground">
												In
											</th>
											<th className="text-left px-3 py-1.5 text-xs font-medium text-muted-foreground">
												Type
											</th>
											<th className="text-left px-3 py-1.5 text-xs font-medium text-muted-foreground">
												Required
											</th>
											<th className="text-left px-3 py-1.5 text-xs font-medium text-muted-foreground">
												Description
											</th>
										</tr>
									</thead>
									<tbody>
										{nonAppParams.map((p) => (
											<tr key={p.name} className="border-b last:border-b-0">
												<td className="px-3 py-1.5 font-mono text-xs">
													{p.name}
												</td>
												<td className="px-3 py-1.5 text-xs text-muted-foreground">
													{p.in}
												</td>
												<td className="px-3 py-1.5 text-xs text-muted-foreground">
													{p.schema?.type ?? "—"}
												</td>
												<td className="px-3 py-1.5 text-xs">
													{p.required ? (
														<Badge
															variant="outline"
															className="text-[10px] px-1"
														>
															required
														</Badge>
													) : (
														<span className="text-muted-foreground">
															optional
														</span>
													)}
												</td>
												<td className="px-3 py-1.5 text-xs text-muted-foreground">
													{p.description ?? "—"}
												</td>
											</tr>
										))}
									</tbody>
								</table>
							</div>
						</div>
					)}

					<div className="space-y-1.5">
						<span className="text-xs font-medium">Example (cURL)</span>
						<CodeBlock
							language="bash"
							code={buildCurlExample(endpoint, fullUrl)}
						/>
					</div>
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}

function buildCurlExample(endpoint: OpenApiPath, fullUrl: string): string {
	const method = endpoint.method.toUpperCase();
	const parts = [`curl -X ${method} "${fullUrl}"`];
	parts.push('  -H "Authorization: pat_{id}.{secret}"');

	if (["post", "put", "patch"].includes(endpoint.method)) {
		parts.push('  -H "Content-Type: application/json"');
		parts.push("  -d '{}'");
	}

	return parts.join(" \\\n");
}

function extractAppEndpoints(spec: OpenApiSpec): OpenApiPath[] {
	const endpoints: OpenApiPath[] = [];
	if (!spec.paths) return endpoints;

	for (const [path, methods] of Object.entries(spec.paths)) {
		if (!path.includes("{app_id}")) continue;

		for (const [method, details] of Object.entries(methods)) {
			if (
				["get", "post", "put", "patch", "delete"].includes(
					method.toLowerCase(),
				)
			) {
				const d = details as Record<string, unknown>;
				endpoints.push({
					method: method.toLowerCase(),
					path,
					summary: d.summary as string | undefined,
					description: d.description as string | undefined,
					tag: Array.isArray(d.tags) ? (d.tags[0] as string) : undefined,
					parameters: d.parameters as OpenApiPath["parameters"],
					requestBody: d.requestBody as OpenApiPath["requestBody"],
					responses: d.responses as OpenApiPath["responses"],
					security: d.security as OpenApiPath["security"],
				});
			}
		}
	}

	return endpoints;
}

function extractGlobalEndpoints(spec: OpenApiSpec): OpenApiPath[] {
	const endpoints: OpenApiPath[] = [];
	if (!spec.paths) return endpoints;

	for (const [path, methods] of Object.entries(spec.paths)) {
		if (path.includes("{app_id}")) continue;
		for (const [method, details] of Object.entries(methods)) {
			if (
				["get", "post", "put", "patch", "delete"].includes(
					method.toLowerCase(),
				)
			) {
				const d = details as Record<string, unknown>;
				const tags = Array.isArray(d.tags) ? (d.tags as string[]) : [];
				if (tags.includes("tmp") || tags.includes("chat")) {
					endpoints.push({
						method: method.toLowerCase(),
						path,
						summary: d.summary as string | undefined,
						description: d.description as string | undefined,
						tag: tags[0],
						parameters: d.parameters as OpenApiPath["parameters"],
						requestBody: d.requestBody as OpenApiPath["requestBody"],
						responses: d.responses as OpenApiPath["responses"],
						security: d.security as OpenApiPath["security"],
					});
				}
			}
		}
	}

	return endpoints;
}

function groupByTag(
	endpoints: OpenApiPath[],
): Record<string, OpenApiPath[]> {
	const groups: Record<string, OpenApiPath[]> = {};
	for (const ep of endpoints) {
		const tag = ep.tag ?? "other";
		if (!groups[tag]) groups[tag] = [];
		groups[tag].push(ep);
	}
	return groups;
}

export function EndpointsPage() {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [spec, setSpec] = useState<OpenApiSpec | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);

	const hubBase = useMemo(() => {
		const hub = profile.data?.hub;
		if (!hub) return "https://api.flow-like.com";
		let base = hub;
		if (!base.startsWith("http://") && !base.startsWith("https://")) {
			const protocol = profile.data?.secure === false ? "http" : "https";
			base = `${protocol}://${base}`;
		}
		return base.replace(/\/+$/, "");
	}, [profile.data?.hub, profile.data?.secure]);

	const baseUrl = `${hubBase}/api/v1`;
	const swaggerUrl = `${hubBase}/swagger-ui`;
	const openApiUrl = `${hubBase}/api-doc/openapi.json`;

	const loadSpec = useCallback(async () => {
		if (!profile.data) return;
		try {
			setLoading(true);
			setError(null);
			let res: Response;
			try {
				const { fetch: tauriFetch } = await import(
					"@tauri-apps/plugin-http"
				);
				res = await tauriFetch(openApiUrl);
			} catch {
				res = await fetch(openApiUrl);
			}
			if (!res.ok) throw new Error(`HTTP ${res.status}`);
			setSpec((await res.json()) as OpenApiSpec);
		} catch {
			setError("Failed to load API specification");
			toast.error("Could not load API specification");
		} finally {
			setLoading(false);
		}
	}, [profile.data, openApiUrl]);

	useEffect(() => {
		if (profile.data) {
			loadSpec();
		}
	}, [profile.data, loadSpec]);

	const appEndpoints = useMemo(
		() => (spec ? extractAppEndpoints(spec) : []),
		[spec],
	);
	const globalEndpoints = useMemo(
		() => (spec ? extractGlobalEndpoints(spec) : []),
		[spec],
	);
	const groupedApp = useMemo(() => groupByTag(appEndpoints), [appEndpoints]);
	const groupedGlobal = useMemo(
		() => groupByTag(globalEndpoints),
		[globalEndpoints],
	);
	const tagDescriptions = useMemo(() => {
		const map: Record<string, string> = {};
		if (spec?.tags) {
			for (const t of spec.tags) {
				if (t.name && t.description) map[t.name] = t.description;
			}
		}
		return map;
	}, [spec]);

	if (loading || profile.isLoading) {
		return (
			<div className="flex items-center justify-center h-64">
				<div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary" />
			</div>
		);
	}

	return (
		<div className="space-y-6">
				{/* SDK Installation */}
				<Card>
					<CardHeader>
						<div className="flex items-center gap-2">
							<PackageIcon className="h-5 w-5" />
							<CardTitle>SDK Installation</CardTitle>
						</div>
						<CardDescription>
							Use the official Flow-Like SDK to interact with your app
							programmatically.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Tabs defaultValue="npm">
							<TabsList>
								<TabsTrigger value="npm">npm / TypeScript</TabsTrigger>
								<TabsTrigger value="python">Python</TabsTrigger>
							</TabsList>
							<TabsContent value="npm" className="space-y-3 mt-3">
								<CodeBlock
									language="bash"
									code="npm install @flow-like/sdk"
								/>
								<CodeBlock
									language="typescript"
									code={`import { FlowLikeClient } from "@flow-like/sdk";

const client = new FlowLikeClient({
  baseUrl: "${baseUrl}",
  // Authenticate with a Personal Access Token (format: pat_{id}.{secret})
  pat: "<your-pat-token>",
  // Or use a Technical User API Key (format: flk_{app_id}.{key_id}.{secret})
  // apiKey: "<your-api-key>",
});

const appId = "${appId}";

// Trigger an event (returns AsyncIterable<SSEChunk>)
for await (const chunk of client.triggerEvent(appId, "event-id", { key: "value" })) {
  console.log(chunk);
}

// Upload a file
const uploaded = await client.uploadFile(appId, file);

// Query a database table
const rows = await client.queryTable(appId, "table-name", {
  filter: "column = 'value'",
});`}
								/>
							</TabsContent>
							<TabsContent value="python" className="space-y-3 mt-3">
								<CodeBlock language="bash" code="pip install flow-like" />
								<CodeBlock
									language="python"
									code={`from flow_like import FlowLikeClient

client = FlowLikeClient(
    base_url="${baseUrl}",
    # Authenticate with a Personal Access Token (format: pat_{id}.{secret})
    pat="<your-pat-token>",
    # Or use a Technical User API Key (format: flk_{app_id}.{key_id}.{secret})
    # api_key="<your-api-key>",
)

app_id = "${appId}"

# Trigger an event (returns Iterator[SSEEvent])
for event in client.trigger_event(app_id, "event-id", {"key": "value"}):
    print(event)

# Upload a file
uploaded = client.upload_file(app_id, file)

# Query a database table
rows = client.query_table(app_id, "table-name", {"filter": "column = 'value'"})`}
								/>
							</TabsContent>
						</Tabs>
					</CardContent>
				</Card>

				{/* Authentication */}
				<Card>
					<CardHeader>
						<div className="flex items-center gap-2">
							<ShieldIcon className="h-5 w-5" />
							<CardTitle>Authentication</CardTitle>
						</div>
						<CardDescription>
							Choose one of the supported authentication methods for API
							access.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="grid gap-4 md:grid-cols-2">
							<Card>
								<CardHeader className="pb-2">
									<div className="flex items-center gap-2">
										<KeyIcon className="h-4 w-4" />
										<CardTitle className="text-sm">
											Personal Access Token (PAT)
										</CardTitle>
									</div>
								</CardHeader>
								<CardContent className="space-y-2">
									<p className="text-xs text-muted-foreground">
										Create PATs in your user settings. Best for personal
										scripts and development. Token format:{" "}
										<code className="text-xs font-mono">pat_&#123;id&#125;.&#123;secret&#125;</code>
									</p>
									<CodeBlock
										language="http"
										code={"Authorization: pat_{id}.{secret}"}
									/>
								</CardContent>
							</Card>
							<Card>
								<CardHeader className="pb-2">
									<div className="flex items-center gap-2">
										<ServerIcon className="h-4 w-4" />
										<CardTitle className="text-sm">
											Technical User API Key
										</CardTitle>
									</div>
								</CardHeader>
								<CardContent className="space-y-2">
									<p className="text-xs text-muted-foreground">
										Create API keys in the Team settings of this app. Best for
										server-to-server integrations. Key format:{" "}
										<code className="text-xs font-mono">flk_&#123;app_id&#125;.&#123;key_id&#125;.&#123;secret&#125;</code>
									</p>
									<CodeBlock
										language="http"
										code={"X-API-Key: flk_{app_id}.{key_id}.{secret}"}
									/>
								</CardContent>
							</Card>
						</div>
					</CardContent>
				</Card>

				{/* App-scoped Endpoints */}
				<Card>
					<CardHeader>
						<div className="flex items-center gap-2">
							<BookOpenIcon className="h-5 w-5" />
							<CardTitle>App Endpoints</CardTitle>
						</div>
						<CardDescription>
							These endpoints are scoped to your app. The app ID{" "}
							<code className="text-xs rounded bg-muted px-1.5 py-0.5 font-mono">
								{appId}
							</code>{" "}
							is pre-filled in all paths.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						{error && (
							<div className="text-sm text-destructive rounded-md border border-destructive/20 bg-destructive/5 p-3">
								{error}
								<Button
									variant="link"
									size="sm"
									className="ml-2 h-auto p-0"
									onClick={loadSpec}
								>
									Retry
								</Button>
							</div>
						)}
						{!error && appEndpoints.length === 0 && (
							<p className="text-sm text-muted-foreground">
								No app-scoped endpoints found.
							</p>
						)}
						{Object.entries(groupedApp).map(([tag, endpoints]) => (
							<div key={tag} className="space-y-1">
								<div className="flex items-center gap-2 mb-2">
									<h3 className="text-sm font-semibold capitalize">{tag}</h3>
									{tagDescriptions[tag] && (
										<span className="text-xs text-muted-foreground">
											— {tagDescriptions[tag]}
										</span>
									)}
									<Badge variant="secondary" className="text-[10px] ml-auto">
										{endpoints.length}
									</Badge>
								</div>
								<div className="border rounded-lg divide-y">
									{endpoints.map((ep) => (
										<EndpointRow
											key={`${ep.method}-${ep.path}`}
											endpoint={ep}
											appId={appId}
											baseUrl={baseUrl}
										/>
									))}
								</div>
							</div>
						))}
					</CardContent>
				</Card>

				{/* Utility Endpoints (tmp, chat) */}
				{Object.keys(groupedGlobal).length > 0 && (
					<Card>
						<CardHeader>
							<div className="flex items-center gap-2">
								<ServerIcon className="h-5 w-5" />
								<CardTitle>Utility Endpoints</CardTitle>
							</div>
							<CardDescription>
								Global endpoints for temporary file uploads, chat completions,
								and other utilities.
							</CardDescription>
						</CardHeader>
						<CardContent className="space-y-4">
							{Object.entries(groupedGlobal).map(([tag, endpoints]) => (
								<div key={tag} className="space-y-1">
									<div className="flex items-center gap-2 mb-2">
										<h3 className="text-sm font-semibold capitalize">{tag}</h3>
										{tagDescriptions[tag] && (
											<span className="text-xs text-muted-foreground">
												— {tagDescriptions[tag]}
											</span>
										)}
										<Badge
											variant="secondary"
											className="text-[10px] ml-auto"
										>
											{endpoints.length}
										</Badge>
									</div>
									<div className="border rounded-lg divide-y">
										{endpoints.map((ep) => (
											<EndpointRow
												key={`${ep.method}-${ep.path}`}
												endpoint={ep}
												appId={appId}
												baseUrl={baseUrl}
											/>
										))}
									</div>
								</div>
							))}
						</CardContent>
					</Card>
				)}

				{/* Swagger UI Link */}
				<Card>
					<CardHeader>
						<div className="flex items-center gap-2">
							<ExternalLinkIcon className="h-5 w-5" />
							<CardTitle>Full API Documentation</CardTitle>
						</div>
						<CardDescription>
							View the complete interactive API documentation in Swagger UI,
							including request/response schemas and the ability to try
							endpoints.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Button variant="outline" asChild>
							<a
								href={swaggerUrl}
								target="_blank"
								rel="noopener noreferrer"
								className="gap-2"
							>
								<ExternalLinkIcon className="h-4 w-4" />
								Open Swagger UI
							</a>
						</Button>
					</CardContent>
				</Card>
		</div>
	);
}
