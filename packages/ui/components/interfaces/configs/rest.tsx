"use client";

import {
	AlertCircle,
	Check,
	Cloud,
	Copy,
	Globe,
	Info,
	Loader2,
	RefreshCw,
	Trash2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
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
} from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

type RestSink = {
	sink_type?: "rest";
};

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

const METHOD_VARIANT: Record<
	string,
	"default" | "secondary" | "outline" | "destructive"
> = {
	GET: "secondary",
	POST: "default",
	PUT: "default",
	PATCH: "default",
	DELETE: "destructive",
};

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

function normalizeAuthType(value: unknown): string {
	if (typeof value !== "string") return "";
	if (value === "o_auth_bearer") return "oauth_bearer";
	return value;
}

function formatConfigValue(value: unknown, key?: string): string {
	if (key?.endsWith("_configured") && value === true) return "configured";
	if (typeof value === "boolean") return value ? "yes" : "no";
	if (typeof value === "string") return value;
	if (typeof value === "number") return String(value);
	if (value === null || value === undefined) return "none";
	return JSON.stringify(value);
}

function routeKindLabel(kind: string): string {
	if (kind === "rest_fn") return "Function";
	if (kind === "rest_file") return "File";
	if (kind === "rest_openapi") return "OpenAPI spec";
	if (kind === "rest_openapi_ui") return "OpenAPI UI";
	return kind.replace(/^rest_/, "").replaceAll("_", " ");
}

function routeGroupLabel(kind: string): string {
	if (kind === "rest_fn") return "Function Routes";
	if (kind === "rest_file") return "File Routes";
	if (kind === "rest_openapi") return "OpenAPI Spec";
	if (kind === "rest_openapi_ui") return "OpenAPI UI";
	return routeKindLabel(kind);
}

function routeExtras(registration: IEventRegistration): Record<string, any> {
	return registration.extras ?? {};
}

function routeConfig(registration: IEventRegistration): Record<string, any> {
	const extras = routeExtras(registration);
	const route = extras.route;
	if (route && typeof route === "object" && !Array.isArray(route)) {
		return route as Record<string, any>;
	}
	return extras;
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

function routeDetailEntries(registration: IEventRegistration) {
	const extras = routeExtras(registration);
	const route = routeConfig(registration);
	const details: Array<[string, string]> = [];

	if (registration.kind === "rest_fn") {
		if (registration.node_id) details.push(["handler", registration.node_id]);
		if (registration.schema) details.push(["request schema", "configured"]);
		return details;
	}

	if (registration.kind === "rest_file") {
		const flowPath = flowPathLabel(route.flow_path);
		if (flowPath) details.push(["file", flowPath]);
		if (route.flow_path?.store_ref) {
			details.push(["store", String(route.flow_path.store_ref)]);
		}
		if (route.directory !== undefined) {
			details.push(["directory", formatConfigValue(route.directory)]);
		}
		if (route.content_type)
			details.push(["content type", String(route.content_type)]);
		return details;
	}

	if (registration.kind === "rest_openapi") {
		const title = extras.spec?.info?.title;
		const version = extras.spec?.info?.version;
		const uiPath = extras.ui_path ?? route.ui_path;
		const pathCount = Object.keys(extras.spec?.paths ?? {}).length;
		if (title) details.push(["title", String(title)]);
		if (version) details.push(["spec version", String(version)]);
		if (uiPath) details.push(["ui path", String(uiPath)]);
		if (pathCount > 0) details.push(["paths", String(pathCount)]);
		return details;
	}

	if (registration.kind === "rest_openapi_ui") {
		const specPath = extras.spec_path ?? route.path;
		if (specPath) details.push(["spec path", String(specPath)]);
		return details;
	}

	return details;
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

function AuthBadge({
	authText,
	active = true,
}: {
	authText: string;
	active?: boolean;
}) {
	return (
		<Badge
			variant={active ? "secondary" : "outline"}
			className="max-w-full whitespace-normal text-left font-normal leading-tight"
		>
			auth: {authText}
		</Badge>
	);
}

export function RestConfig({
	config,
	onConfigUpdate,
	eventId,
	appId,
}: IConfigInterfaceProps) {
	useEffect(() => {
		if (!(config as RestSink)?.sink_type) {
			onConfigUpdate?.({
				...(config as RestSink),
				sink_type: "rest",
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

	const endpointUrl = eventId ? `${baseUrl}/r/${eventId}` : null;

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

	const registrationData = registrations.data as
		| IListRegistrationsResponse
		| undefined;
	const registrationRows = registrationData?.registrations ?? [];
	const authRows = registrationData?.auths ?? [];
	const openApiRegistration = registrationRows.find(
		(r: any) => r.kind === "rest_openapi",
	);
	const openApiSpecPath = openApiRegistration?.path ?? "/openapi.json";
	const openApiUiPath =
		registrationRows.find((r: any) => r.kind === "rest_openapi_ui")?.path ??
		openApiRegistration?.extras?.ui_path ??
		openApiRegistration?.extras?.route?.ui_path ??
		null;
	const openApiUrl = eventId
		? `${baseUrl}/r/${eventId}${openApiSpecPath}`
		: null;
	const openApiUiUrl =
		eventId && openApiUiPath ? `${baseUrl}/r/${eventId}${openApiUiPath}` : null;

	useEffect(() => {
		if (!appId || !eventId) return;
		const id = window.setInterval(() => {
			registrations.refetch();
			aliases.refetch();
		}, 4000);
		return () => window.clearInterval(id);
	}, [aliases.refetch, appId, eventId, registrations.refetch]);

	const [copied, setCopied] = useState<string | null>(null);
	const [setupBusy, setSetupBusy] = useState(false);
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
				const response = await backend.eventState.setupEvent(appId, eventId, true);
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

	const restRegs = registrationRows.filter(
		(r: any) =>
			r.kind === "rest_fn" ||
			r.kind === "rest_file" ||
			r.kind === "rest_openapi" ||
			r.kind === "rest_openapi_ui",
	);
	const knownAuthIds = new Set(authRows.map((auth) => auth.id));
	const missingAuthIds = Array.from(
		new Set(
			restRegs
				.map((registration) => registration.auth_id)
				.filter((authId): authId is string => Boolean(authId)),
		),
	).filter((authId) => !knownAuthIds.has(authId));
	const currentAlias = aliases.data?.[0]?.slug ?? "";
	const [aliasInput, setAliasInput] = useState("");
	const [aliasBusy, setAliasBusy] = useState(false);
	const [aliasError, setAliasError] = useState<string | null>(null);
	const aliasUrl = currentAlias ? `${baseUrl}/r/${currentAlias}` : null;

	useEffect(() => {
		setAliasInput(currentAlias);
	}, [currentAlias]);

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
				<Globe className="h-4 w-4" />
				<AlertTitle className="flex items-center gap-2">
					REST API Server
					<span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs">
						<Cloud className="h-3 w-3" /> Remote only
					</span>
				</AlertTitle>
				<AlertDescription>
					Spins up a remote REST API server. Endpoints, methods, authentication
					and request schemas are declared inside the workflow board. The server
					is mounted at <code>/r/&#123;event_id&#125;</code>.
				</AlertDescription>
			</Alert>

			{endpointUrl && openApiUrl ? (
				<div className="space-y-3 rounded-md border bg-muted/30 p-3">
					<div className="space-y-1.5">
						<Label className="text-xs uppercase tracking-wide text-muted-foreground">
							Base URL
						</Label>
						<div className="flex items-center gap-2">
							<Input
								readOnly
								value={endpointUrl}
								className="font-mono text-xs"
							/>
							<Button
								type="button"
								size="icon"
								variant="outline"
								onClick={() => copy("base", endpointUrl)}
								title="Copy"
							>
								{copied === "base" ? (
									<Check className="h-4 w-4" />
								) : (
									<Copy className="h-4 w-4" />
								)}
							</Button>
						</div>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs uppercase tracking-wide text-muted-foreground">
							OpenAPI Spec
						</Label>
						<div className="flex items-center gap-2">
							<Input
								readOnly
								value={openApiUrl}
								className="font-mono text-xs"
							/>
							<Button
								type="button"
								size="icon"
								variant="outline"
								onClick={() => copy("openapi", openApiUrl)}
								title="Copy"
							>
								{copied === "openapi" ? (
									<Check className="h-4 w-4" />
								) : (
									<Copy className="h-4 w-4" />
								)}
							</Button>
						</div>
					</div>
					{openApiUiUrl && (
						<div className="space-y-1.5">
							<Label className="text-xs uppercase tracking-wide text-muted-foreground">
								OpenAPI UI
							</Label>
							<div className="flex items-center gap-2">
								<Input
									readOnly
									value={openApiUiUrl}
									className="font-mono text-xs"
								/>
								<Button
									type="button"
									size="icon"
									variant="outline"
									onClick={() => copy("openapi-ui", openApiUiUrl)}
									title="Copy"
								>
									{copied === "openapi-ui" ? (
										<Check className="h-4 w-4" />
									) : (
										<Copy className="h-4 w-4" />
									)}
								</Button>
							</div>
						</div>
					)}
				</div>
			) : (
				<p className="text-xs text-muted-foreground rounded-md border border-dashed p-3">
					Save the event to see its endpoint URL.
				</p>
			)}

			{eventId && appId && backend.eventState.listEventAliases && (
				<div className="space-y-3 rounded-md border bg-card p-3">
					<div className="space-y-1">
						<Label className="text-xs uppercase tracking-wide text-muted-foreground">
							Public Alias
						</Label>
						<div className="flex items-center gap-2">
							<div className="shrink-0 rounded-md border bg-muted px-2 py-2 font-mono text-xs text-muted-foreground">
								{baseUrl}/r/
							</div>
							<Input
								value={aliasInput}
								onChange={(event) => {
									setAliasError(null);
									setAliasInput(event.target.value);
								}}
								placeholder="my-api"
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
					</div>
					{aliasUrl && (
						<div className="flex items-center gap-2">
							<Input readOnly value={aliasUrl} className="font-mono text-xs" />
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
			)}

			{eventId && appId && (
				<SetupConfigPanel
					version={registrationData?.event_version ?? null}
					auths={authRows}
					missingAuthIds={missingAuthIds}
					registrations={restRegs}
					loading={registrations.isLoading}
					fetching={registrations.isFetching || setupBusy}
					onRefresh={refreshSetup}
				/>
			)}

			{eventId && appId && (
				<RegistrationsPanel
					title="Registered Routes"
					emptyHint="Setup runs automatically when you save this event. Routes will appear here once the workflow has declared its endpoints."
					loading={registrations.isLoading}
					fetching={registrations.isFetching || setupBusy}
					error={registrations.error?.message ?? null}
					regs={restRegs.map((r: any) => ({
						id: r.id,
						method: r.method ?? "GET",
						path: r.path,
						node_id: r.node_id ?? null,
						kind: r.kind,
					}))}
					onRefresh={refreshSetup}
				/>
			)}

			<p className="text-xs text-muted-foreground flex items-center gap-1">
				<Info className="h-3 w-3" />
				Save the event to trigger a remote setup that registers all declared
				routes.
			</p>
		</div>
	);
}

interface SetupConfigPanelProps {
	version: string | null;
	auths: IEventRemoteAuth[];
	missingAuthIds: string[];
	registrations: IEventRegistration[];
	loading: boolean;
	fetching: boolean;
	onRefresh: () => Promise<void> | void;
}

function SetupConfigPanel({
	version,
	auths,
	missingAuthIds,
	registrations,
	loading,
	fetching,
	onRefresh,
}: SetupConfigPanelProps) {
	const authById = new Map(auths.map((auth) => [auth.id, auth]));
	const routeGroups = [
		"rest_fn",
		"rest_file",
		"rest_openapi",
		"rest_openapi_ui",
	]
		.map((kind) => ({
			kind,
			label: routeGroupLabel(kind),
			registrations: registrations.filter(
				(registration) => registration.kind === kind,
			),
		}))
		.filter((group) => group.registrations.length > 0);
	const authCount = auths.length + missingAuthIds.length;

	return (
		<div className="space-y-3 rounded-md border bg-card p-3">
			<div className="flex items-center justify-between">
				<Label className="text-xs uppercase tracking-wide text-muted-foreground">
					Current Setup
				</Label>
				<Button
					type="button"
					size="sm"
					variant="ghost"
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
			</div>

			<div className="grid gap-2 lg:grid-cols-3">
				<div className="space-y-1 rounded-md border bg-background p-2">
					<div className="text-[10px] uppercase text-muted-foreground">
						Version
					</div>
					<div className="font-mono text-xs">{version ?? "not registered"}</div>
				</div>
				<div className="space-y-1 rounded-md border bg-background p-2">
					<div className="text-[10px] uppercase text-muted-foreground">
						Authentication
					</div>
					{authCount === 0 ? (
						<div className="text-xs text-muted-foreground">none</div>
					) : (
						<div className="flex flex-wrap gap-1">
							{auths.map((auth) => (
								<Badge
									key={auth.id}
									variant="outline"
									className="max-w-full whitespace-normal font-normal leading-tight"
								>
									{authLabel(auth.config)}
								</Badge>
							))}
							{missingAuthIds.map((authId) => (
								<Badge
									key={authId}
									variant="outline"
									className="max-w-full whitespace-normal font-normal leading-tight"
								>
									configured
								</Badge>
							))}
						</div>
					)}
				</div>
				<div className="space-y-1 rounded-md border bg-background p-2">
					<div className="text-[10px] uppercase text-muted-foreground">
						Routes
					</div>
					<div className="text-xs">
						{loading ? "loading" : `${registrations.length} registered`}
					</div>
				</div>
			</div>

			{authCount > 0 && (
				<div className="space-y-2">
					<div className="text-xs font-medium">Authentication</div>
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
								Auth is linked to routes, but details are not available.
							</span>
							<code className="truncate">{authId}</code>
						</div>
					))}
				</div>
			)}

			<div className="space-y-2">
				<div className="text-xs font-medium">Routes</div>
				{loading ? (
					<div className="flex items-center gap-2 rounded-md border bg-background p-2 text-xs text-muted-foreground">
						<Loader2 className="h-3 w-3 animate-spin" />
						Loading setup…
					</div>
				) : routeGroups.length === 0 ? (
					<div className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
						No REST routes registered yet.
					</div>
				) : (
					<div className="space-y-2">
						{routeGroups.map((group) => (
							<div key={group.kind} className="rounded-md border bg-background">
								<div className="flex items-center justify-between border-b px-2 py-1.5">
									<span className="text-xs font-medium">{group.label}</span>
									<Badge variant="outline" className="font-mono text-[10px]">
										{group.registrations.length}
									</Badge>
								</div>
								<ul className="divide-y">
									{group.registrations.map((registration) => {
										const method = (registration.method ?? "GET").toUpperCase();
										const auth = registration.auth_id
											? authById.get(registration.auth_id)
											: null;
										const authText = registration.auth_id
											? auth
												? authLabel(auth.config)
												: "configured"
											: "none";
										const details = routeDetailEntries(registration);

										return (
											<li
												key={registration.id}
												className="space-y-2 p-2 text-xs"
											>
												<div className="flex min-w-0 flex-wrap items-center gap-2">
													<Badge
														variant={METHOD_VARIANT[method] ?? "outline"}
														className="shrink-0 font-mono text-[10px]"
													>
														{method}
													</Badge>
													<code className="min-w-0 break-all font-mono text-sm">
														{registration.path}
													</code>
													<AuthBadge
														authText={authText}
														active={Boolean(registration.auth_id)}
													/>
												</div>
												{details.length > 0 && (
													<div className="grid gap-1 lg:grid-cols-2">
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
	);
}

interface RegRow {
	id: string;
	method: string;
	path: string;
	node_id: string | null;
	kind: string;
}

interface RegistrationsPanelProps {
	title: string;
	emptyHint: string;
	loading: boolean;
	fetching: boolean;
	error: string | null;
	regs: RegRow[];
	onRefresh: () => Promise<void> | void;
	showMethod?: boolean;
}

export function RegistrationsPanel({
	title,
	emptyHint,
	loading,
	fetching,
	error,
	regs,
	onRefresh,
	showMethod = true,
}: RegistrationsPanelProps) {
	return (
		<div className="space-y-2 rounded-md border bg-card p-3">
			<div className="flex items-center justify-between">
				<Label className="text-xs uppercase tracking-wide text-muted-foreground">
					{title}
					{regs.length > 0 && (
						<span className="ml-1 text-muted-foreground/70">
							({regs.length})
						</span>
					)}
				</Label>
				<Button
					type="button"
					size="sm"
					variant="ghost"
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
			</div>

			{error && (
				<div className="flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/10 p-2 text-xs text-destructive">
					<AlertCircle className="h-3 w-3 shrink-0" />
					<span className="truncate">{error}</span>
				</div>
			)}

			{loading ? (
				<div className="flex items-center gap-2 p-2 text-xs text-muted-foreground">
					<Loader2 className="h-3 w-3 animate-spin" /> Loading…
				</div>
			) : regs.length === 0 ? (
				<div className="flex items-start gap-2 rounded-md border border-dashed p-2 text-xs text-muted-foreground">
					<Loader2 className="h-3 w-3 mt-0.5 shrink-0 animate-spin" />
					<span>{emptyHint}</span>
				</div>
			) : (
				<ul className="divide-y rounded-md border bg-background">
					{regs.map((r) => (
						<li
							key={r.id}
							className="flex items-center gap-2 px-2 py-1.5 text-xs"
						>
							{showMethod && (
								<Badge
									variant={METHOD_VARIANT[r.method.toUpperCase()] ?? "outline"}
									className="font-mono text-[10px]"
								>
									{r.method.toUpperCase()}
								</Badge>
							)}
							<code className="flex-1 truncate font-mono text-xs">
								{r.path}
							</code>
							{r.kind && (
								<span className="text-[10px] uppercase text-muted-foreground/70">
									{r.kind.replace(/^rest_|^mcp_/, "")}
								</span>
							)}
						</li>
					))}
				</ul>
			)}
		</div>
	);
}
