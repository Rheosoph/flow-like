"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { ExternalLink } from "lucide-react";
import type React from "react";
import { useMemo, useState } from "react";
import { useInvoke } from "../../../hooks";
import { getApiOrigin } from "../../../lib/api-url";
import { IEventExecutionMode } from "../../../lib/schema/flow/event";
import { useBackend } from "../../../state/backend-state";
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
} from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

export type SinkExecutionTarget = "REMOTE" | "LOCAL" | "HYBRID";

export type HttpSink = {
	path: string;
	method: string;
	auth_token?: string | null;
	sink_execution?: SinkExecutionTarget;
};

function getPlatform(): "mac" | "windows" | "linux" {
	if (typeof window === "undefined") return "mac";
	const ua = window.navigator.userAgent.toLowerCase();
	if (ua.includes("win")) return "windows";
	if (ua.includes("linux")) return "linux";
	return "mac";
}

function getCloudflareInstallCommand(
	platform: ReturnType<typeof getPlatform>,
): string {
	switch (platform) {
		case "windows":
			return `winget install --id Cloudflare.cloudflared`;
		case "linux":
			return "curl -L --output cloudflared.deb https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb\nsudo dpkg -i cloudflared.deb";
		default:
			return `brew install cloudflare/cloudflare/cloudflared`;
	}
}

function getNgrokInstallCommand(
	platform: ReturnType<typeof getPlatform>,
): string {
	switch (platform) {
		case "windows":
			return `choco install ngrok`;
		case "linux":
			return 'curl -s https://ngrok-agent.s3.amazonaws.com/ngrok.asc | sudo tee /etc/apt/trusted.gpg.d/ngrok.asc >/dev/null\necho "deb https://ngrok-agent.s3.amazonaws.com buster main" | sudo tee /etc/apt/sources.list.d/ngrok.list\nsudo apt update && sudo apt install ngrok';
		default:
			return `brew install ngrok`;
	}
}

const HTTP_METHODS = [
	{ value: "GET", label: "GET", description: "Retrieve data" },
	{ value: "POST", label: "POST", description: "Create or submit data" },
	{ value: "PUT", label: "PUT", description: "Update/replace data" },
	{ value: "PATCH", label: "PATCH", description: "Partially update data" },
	{ value: "DELETE", label: "DELETE", description: "Remove data" },
	{ value: "HEAD", label: "HEAD", description: "Get headers only" },
	{ value: "OPTIONS", label: "OPTIONS", description: "Get supported methods" },
];

export function HttpConfig({
	isEditing,
	appId,
	config,
	onConfigUpdate,
	hub,
	eventExecutionMode,
	section,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const [showToken, setShowToken] = useState(false);
	const platform = getPlatform();

	const path = (config?.path as string) || "/webhook";
	const method = (config?.method as string) || "POST";
	const authToken = (config?.auth_token as string | null) || null;

	const setValue = (key: string, value: any) => {
		onConfigUpdate?.({
			...config,
			[key]: value,
		});
	};

	const isRemote = eventExecutionMode === IEventExecutionMode.Remote;

	const localUrl = `http://localhost:9657/${appId}${path}`;

	const remoteUrl = useMemo(() => {
		// Prefer the hub's declared domain (authoritative), then the same API
		// origin the rest of the app uses for API calls (profile.hub /
		// NEXT_PUBLIC_API_URL). Never fall back to `window.location.origin`:
		// in dev the web app is served from a different host than the API.
		// The sink trigger lives under /api/v1/ just like every other route
		// the backend exposes — easy to miss because it's a "public" webhook
		// endpoint, but it's still nested under /api/v1.
		if (hub?.domain) {
			const protocol = hub.environment === "Development" ? "http" : "https";
			return `${protocol}://${hub.domain}/api/v1/sink/trigger/http/${appId}${path}`;
		}
		const origin = getApiOrigin(profile.data);
		if (origin) {
			return `${origin}/api/v1/sink/trigger/http/${appId}${path}`;
		}
		return null;
	}, [hub?.domain, hub?.environment, profile.data, appId, path]);

	const pathError =
		path && !path.startsWith("/")
			? t("pathMustStartWith", "Path must start with '/'")
			: null;

	const CurlExample = ({
		url,
		withAuth,
	}: { url: string; withAuth: boolean }) => (
		<pre className="mt-2 overflow-x-auto text-xs bg-muted p-3 rounded-md">
			{withAuth
				? `curl -X ${method} "${url}" \\\n  -H "Authorization: Bearer ${authToken}"`
				: `curl -X ${method} "${url}"`}
		</pre>
	);

	// The events surface renders one section at a time; anywhere else (and for
	// any section this component doesn't know) it renders whole.
	const shows = (id: string) => !section || section === id;

	return (
		<div className="w-full space-y-6">
			{!section && (
				<div className="space-y-1">
					<h3 className="text-lg font-semibold">
						{t("httpEventSink", "HTTP Event Sink")}
					</h3>
					<p className="text-sm text-muted-foreground">
						{t(
							"triggerThisEventViaHttpRequests",
							"Trigger this event via HTTP requests.",
						)}
					</p>
				</div>
			)}

			{shows("endpoint") && (
				<>
					{/* Method Selection */}
					<div className="space-y-2">
						<Label htmlFor="http_method">
							{t("httpMethod", "HTTP Method")}
						</Label>
						<Select
							value={method}
							onValueChange={(value) => setValue("method", value)}
							disabled={!isEditing}
						>
							<SelectTrigger id="http_method" className="w-full">
								<SelectValue
									placeholder={t("selectHttpMethod", "Select HTTP method")}
								/>
							</SelectTrigger>
							<SelectContent>
								{HTTP_METHODS.map((m) => (
									<SelectItem key={m.value} value={m.value}>
										<div className="flex items-center gap-2">
											<Badge variant="outline" className="font-mono">
												{m.label}
											</Badge>
											<span className="text-muted-foreground text-xs">
												{m.description}
											</span>
										</div>
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<p className="text-sm text-muted-foreground">
							{t(
								"theHttpMethodThatWillTriggerThisEvent",
								"The HTTP method that will trigger this event.",
							)}
						</p>
					</div>

					{/* Path */}
					<div className="space-y-2">
						<Label htmlFor="http_path">{t("path", "Path")}</Label>
						<div className="flex items-center gap-2">
							<div className="shrink-0 text-sm text-muted-foreground">
								/{appId}
							</div>
							<Input
								id="http_path"
								value={path}
								onChange={(e) => setValue("path", e.target.value)}
								placeholder="/webhook"
								disabled={!isEditing}
								className={pathError ? "border-destructive" : ""}
							/>
						</div>
						{pathError && (
							<p className="text-sm text-destructive">{pathError}</p>
						)}
						<p className="text-sm text-muted-foreground">
							<Trans i18nKey="thePathForThisEndpointMustStartWithCodecode">
								The path for this endpoint. Must start with <code>/</code>.
							</Trans>
						</p>
					</div>

					{/* URL Preview — tied to the event's execution mode, not to platform
			    capabilities. Remote events show the server endpoint; Local events
			    show the desktop localhost URL plus tunnel instructions. */}
					<div className="space-y-2">
						<Label>{t("endpointUrl", "Endpoint URL")}</Label>
						{isRemote ? (
							remoteUrl ? (
								<div className="space-y-3">
									<UrlPreview
										url={remoteUrl}
										method={method}
										variant="default"
										authToken={authToken}
										CurlExample={CurlExample}
									/>
									<p className="text-xs text-muted-foreground">
										{t(
											"thisIsAPublicServerhostedEndpointItsAlwaysAvailableNoTunnelingRequiredPointExternalServicesAtThisUrlToTriggerTheEvent",
											"This is a public, server-hosted endpoint. It's always available — no tunneling required. Point external services at this URL to trigger the event.",
										)}
									</p>
								</div>
							) : (
								<Alert variant="destructive">
									<AlertTitle>
										{t(
											"serverEndpointUnavailable",
											"Server endpoint unavailable",
										)}
									</AlertTitle>
									<AlertDescription>
										{t(
											"thisEventIsConfiguredToRunRemotelyButNoHubDomainIsAvailableSignInToAHubThatSupportsHttpSinksOrSwitchTheEventToRunLocally",
											"This event is configured to run remotely, but no hub domain is available. Sign in to a hub that supports HTTP sinks, or switch the event to run locally.",
										)}
									</AlertDescription>
								</Alert>
							)
						) : (
							<div className="space-y-4">
								<UrlPreview
									url={localUrl}
									method={method}
									variant="secondary"
									authToken={authToken}
									CurlExample={CurlExample}
								/>
								<Alert>
									<AlertTitle>
										{t("localEndpoint", "Local endpoint")}
									</AlertTitle>
									<AlertDescription>
										{t(
											"thisUrlIsOnlyReachableWhileTheDesktopAppIsRunningOnThisMachineToExposeItToThePublicInternetUseATunnelInstructionsBelow",
											"This URL is only reachable while the desktop app is running on this machine. To expose it to the public internet, use a tunnel (instructions below).",
										)}
									</AlertDescription>
								</Alert>
								<LocalTunnelGuide path={path} platform={platform} />
							</div>
						)}
					</div>
				</>
			)}

			{shows("access") && (
				<div className="space-y-4">
					<div className="flex items-center justify-between">
						<div className="space-y-0.5">
							<Label>{t("authentication", "Authentication")}</Label>
							<p className="text-sm text-muted-foreground">
								{t(
									"optionalBearerTokenToSecureThisEndpoint",
									"Optional Bearer token to secure this endpoint",
								)}
							</p>
						</div>
						<Switch
							checked={authToken !== null && authToken !== ""}
							onCheckedChange={(checked) => {
								if (checked) {
									setValue("auth_token", generateToken());
								} else {
									setValue("auth_token", null);
								}
							}}
							disabled={!isEditing}
						/>
					</div>

					{authToken && (
						<div className="space-y-2">
							<div className="flex items-center justify-between">
								<Label htmlFor="http_auth_token">{`Bearer Token`}</Label>
								<Button
									type="button"
									variant="ghost"
									size="sm"
									onClick={() => setShowToken(!showToken)}
								>
									{showToken ? "Hide" : "Show"}
								</Button>
							</div>
							<div className="flex gap-2">
								<Input
									id="http_auth_token"
									type={showToken ? "text" : "password"}
									value={authToken}
									onChange={(e) => setValue("auth_token", e.target.value)}
									placeholder={t(
										"enterTokenOrGenerateOne",
										"Enter token or generate one",
									)}
									disabled={!isEditing}
									className="font-mono text-xs"
								/>
								<Button
									type="button"
									variant="secondary"
									onClick={() => setValue("auth_token", generateToken())}
									disabled={!isEditing}
								>
									{t("generate", "Generate")}
								</Button>
							</div>
							<p className="text-sm text-muted-foreground">
								{t("includeThisTokenAs", "Include this token as")}{" "}
								<code>
									{`Authorization: Bearer`} {"{token}"}
								</code>{" "}
								{t("inYourRequests", "in your requests.")}
							</p>
						</div>
					)}
				</div>
			)}

			{/* Conflict Warning */}
			{shows("endpoint") && !pathError && (
				<Alert>
					<AlertTitle>{t("routeConflicts", "Route Conflicts")}</AlertTitle>
					<AlertDescription>
						{t(
							"ifMultipleEventsUseTheSameAppIdPathAndMethodOnlyTheMostRecentlyRegisteredEventWillBeTriggeredTheSystemWillLogWarningsIfConflictsOccur",
							"If multiple events use the same app ID, path, and method, only the most recently registered event will be triggered. The system will log warnings if conflicts occur.",
						)}
					</AlertDescription>
				</Alert>
			)}
		</div>
	);
}

function UrlPreview({
	url,
	method,
	variant,
	authToken,
	CurlExample,
}: {
	url: string;
	method: string;
	variant: "default" | "secondary";
	authToken: string | null;
	CurlExample: (props: { url: string; withAuth: boolean }) => React.ReactNode;
}) {
	const { t } = useTranslation("interfaces");
	return (
		<>
			<div className="relative">
				<div className="flex h-auto min-h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm items-center font-mono break-all">
					<Badge variant={variant} className="mr-2 font-mono shrink-0">
						{method}
					</Badge>
					{url}
				</div>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="absolute right-1 top-1 h-8"
					onClick={() => navigator.clipboard.writeText(url)}
				>
					Copy
				</Button>
			</div>
			<Alert>
				<AlertTitle>{t("exampleRequest", "Example Request")}</AlertTitle>
				<AlertDescription>
					<CurlExample url={url} withAuth={!!authToken} />
				</AlertDescription>
			</Alert>
		</>
	);
}

function generateToken(): string {
	const array = new Uint8Array(32);
	crypto.getRandomValues(array);
	return Array.from(array, (byte) => byte.toString(16).padStart(2, "0")).join(
		"",
	);
}

function LocalTunnelGuide({
	path,
	platform,
}: {
	path: string;
	platform: ReturnType<typeof getPlatform>;
}) {
	const { t } = useTranslation("interfaces");
	const platformLabel =
		platform === "windows"
			? "Windows"
			: platform === "linux"
				? "Linux"
				: "macOS";
	return (
		<div className="space-y-3">
			<Label className="text-base">
				{t("exposeThisEndpointPublicly", "Expose this endpoint publicly")}
			</Label>
			<p className="text-sm text-muted-foreground">
				<Trans i18nKey="localEventsAreOnlyReachableOnThisMachineIfYouNeedAnExternalServiceWebhookProviderPartnerSystemEtcToCallThisEndpointRunATunnelInFrontOfPortCode9657code">
					Local events are only reachable on this machine. If you need an
					external service (webhook provider, partner system, etc.) to call this
					endpoint, run a tunnel in front of port <code>9657</code>.
				</Trans>
			</p>
			<Tabs defaultValue="cloudflare" className="w-full">
				<TabsList className="grid w-full grid-cols-2">
					<TabsTrigger value="cloudflare" className="gap-2">
						{t("cloudflareTunnel", "Cloudflare Tunnel")}
						<Badge className="px-2 py-0.5 text-xs">
							{t("recommended", "Recommended")}
						</Badge>
					</TabsTrigger>
					<TabsTrigger value="ngrok">ngrok</TabsTrigger>
				</TabsList>
				<TabsContent value="cloudflare" className="space-y-4 mt-4">
					<ol className="space-y-3 text-sm list-decimal list-inside">
						<li>
							<Trans i18nKey="installCodecloudflaredcodeOn">
								Install <code>cloudflared</code> on
							</Trans>{" "}
							<strong>{platformLabel}</strong>:
							<pre className="mt-2 p-3 bg-muted rounded-md text-xs whitespace-pre-wrap overflow-x-auto">
								{getCloudflareInstallCommand(platform)}
							</pre>
						</li>
						<li>
							<Trans i18nKey="startAFreeQuickTunnelPreClassnamemt2P3BgmutedRoundedmdTextxsOverflowxautoCloudflaredTunnelUrlHttplocalhost9657Pre">
								Start a free Quick Tunnel:
								<pre className="mt-2 p-3 bg-muted rounded-md text-xs overflow-x-auto">
									cloudflared tunnel --url http://localhost:9657
								</pre>
							</Trans>
						</li>
						<li>
							{t("useTheGenerated", "Use the generated")}{" "}
							<code>https://*****.trycloudflare.com{path}</code>{" "}
							{t("urlInYourExternalSystem", "URL in your external system.")}
						</li>
					</ol>
					<a
						href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/"
						target="_blank"
						rel="noopener noreferrer"
						className="text-xs text-primary hover:underline inline-flex items-center gap-1"
					>
						{t("installationDocs", "Installation docs")}{" "}
						<ExternalLink className="h-3 w-3" />
					</a>
				</TabsContent>
				<TabsContent value="ngrok" className="space-y-4 mt-4">
					<ol className="space-y-3 text-sm list-decimal list-inside">
						<li>
							{t("installNgrokOn", "Install ngrok on")}{" "}
							<strong>{platformLabel}</strong>:
							<pre className="mt-2 p-3 bg-muted rounded-md text-xs whitespace-pre-wrap overflow-x-auto">
								{getNgrokInstallCommand(platform)}
							</pre>
						</li>
						<li>
							<Trans i18nKey="authenticateRequiresAFreeNgrokAccountPreClassnamemt2P3BgmutedRoundedmdTextxsOverflowxautoNgrokConfigAddauthtokenYour_token_herePre">
								Authenticate (requires a free ngrok account):
								<pre className="mt-2 p-3 bg-muted rounded-md text-xs overflow-x-auto">
									ngrok config add-authtoken YOUR_TOKEN_HERE
								</pre>
							</Trans>
						</li>
						<li>
							<Trans i18nKey="startTheTunnelPreClassnamemt2P3BgmutedRoundedmdTextxsOverflowxautoNgrokHttp9657Pre">
								Start the tunnel:
								<pre className="mt-2 p-3 bg-muted rounded-md text-xs overflow-x-auto">
									ngrok http 9657
								</pre>
							</Trans>
						</li>
					</ol>
					<a
						href="https://dashboard.ngrok.com/get-started/setup"
						target="_blank"
						rel="noopener noreferrer"
						className="text-xs text-primary hover:underline inline-flex items-center gap-1"
					>
						{t("ngrokDocs", "ngrok docs")} <ExternalLink className="h-3 w-3" />
					</a>
				</TabsContent>
			</Tabs>
		</div>
	);
}
