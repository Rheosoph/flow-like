"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Check, Copy, ExternalLink, Play } from "lucide-react";
import { useState } from "react";
import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
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
	TextEditor,
	Textarea,
} from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

export function ApiConfig({
	isEditing,
	appId,
	boardId,
	config,
	nodeId,
	node,
	onConfigUpdate,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const [copiedCode, setCopiedCode] = useState<string | null>(null);
	const [testPayload, setTestPayload] = useState("{}");
	const [isTesting, setIsTesting] = useState(false);
	const [testResult, setTestResult] = useState<string | null>(null);

	// Detect user's platform
	const getPlatform = (): "mac" | "windows" | "linux" => {
		if (typeof window === "undefined") return "mac";
		const userAgent = window.navigator.userAgent.toLowerCase();
		if (userAgent.includes("win")) return "windows";
		if (userAgent.includes("linux")) return "linux";
		return "mac";
	};

	const platform = getPlatform();

	const getCloudflareInstallCommand = () => {
		switch (platform) {
			case "windows":
				return "winget install --id Cloudflare.cloudflared";
			case "linux":
				return "# Debian/Ubuntu\ncurl -L --output cloudflared.deb https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb\nsudo dpkg -i cloudflared.deb";
			case "mac":
			default:
				return "brew install cloudflare/cloudflare/cloudflared";
		}
	};

	const getNgrokInstallCommand = () => {
		switch (platform) {
			case "windows":
				return "choco install ngrok";
			case "linux":
				return '# Download and extract\ncurl -s https://ngrok-agent.s3.amazonaws.com/ngrok.asc | sudo tee /etc/apt/trusted.gpg.d/ngrok.asc >/dev/null\necho "deb https://ngrok-agent.s3.amazonaws.com buster main" | sudo tee /etc/apt/sources.list.d/ngrok.list\nsudo apt update && sudo apt install ngrok';
			case "mac":
			default:
				return "brew install ngrok";
		}
	};

	const getPlatformLabel = () => {
		switch (platform) {
			case "windows":
				return "Windows";
			case "linux":
				return "Linux";
			case "mac":
			default:
				return "macOS";
		}
	};

	const setValue = (key: string, value: any) => {
		if (onConfigUpdate) {
			onConfigUpdate({
				...config,
				[key]: value,
			});
		}
	};

	const method = config?.method ?? "GET";
	const path = config?.path ?? "/webhook";
	const publicEndpoint = config?.public_endpoint ?? false;
	const authToken = config?.auth_token ?? "";
	const endpoint = `http://localhost:9657/${appId}${path}`;

	const copyToClipboard = (text: string, id: string) => {
		navigator.clipboard.writeText(text);
		setCopiedCode(id);
		setTimeout(() => setCopiedCode(null), 2000);
	};

	const handleTestRequest = async () => {
		setIsTesting(true);
		setTestResult(null);

		try {
			const headers: Record<string, string> = {
				"Content-Type": "application/json",
			};

			if (!publicEndpoint && authToken) {
				headers.Authorization = `Bearer ${authToken}`;
			}

			const options: RequestInit = {
				method,
				headers,
			};

			if (method !== "GET" && testPayload) {
				options.body = testPayload;
			}

			const response = await fetch(endpoint, options);
			const text = await response.text();

			setTestResult(
				`Status: ${response.status}\n\n${text || t('noResponseBody', 'No response body')}`,
			);
		} catch (error) {
			setTestResult(
				`Error: ${error instanceof Error ? error.message : String(error)}`,
			);
		} finally {
			setIsTesting(false);
		}
	};

	const generateCodeExample = (lang: string) => {
		const authHeader =
			!publicEndpoint && authToken
				? `Authorization: Bearer ${authToken}`
				: null;

		switch (lang) {
			case "curl":
				return `\`\`\`bash
curl -X ${method} "${endpoint}"${authHeader ? ` \\\n+  -H "${authHeader}"` : ""}${method !== "GET" ? ' \\\n+  -H "Content-Type: application/json" \\\n+  -d \'{"key": "value"}\'' : ""}
\`\`\``;

			case "python":
				return `\`\`\`python
import requests

${
	authHeader
		? `headers = {
    "Authorization": "${authToken}",
    "Content-Type": "application/json"
}

`
		: ""}${method !== "GET"
		? `payload = {"key": "value"}

`
		: ""}response = requests.${method.toLowerCase()}(
    "${endpoint}"${authHeader ? ",\n    headers=headers" : ""}${method !== "GET" ? ",\n    json=payload" : ""}
)
print(response.status_code, response.json())
\`\`\``;

			case "typescript":
				return `\`\`\`typescript
const response = await fetch("${endpoint}", {
  method: "${method}",${authHeader ? `\n  headers: {\n    "Authorization": "Bearer ${authToken}",\n    "Content-Type": "application/json"\n  },` : ""}${method !== "GET" ? '\n  body: JSON.stringify({"key": "value"})' : ""}
});

const data = await response.json();
console.log(data);
\`\`\``;

			case "rust":
				return `\`\`\`rust
use reqwest;

let client = reqwest::Client::new();
let response = client.${method.toLowerCase()}("${endpoint}")${authHeader ? `\n    .header("Authorization", "Bearer ${authToken}")` : ""}${method !== "GET" ? '\n    .json(&serde_json::json!({"key": "value"}))' : ""}
    .send()
    .await?;

println!("Status: {}", response.status());
let body = response.text().await?;
println!("Body: {}", body);
\`\`\``;

			default:
				return "";
		}
	};

	return (
		<div className="w-full space-y-6">
			<div className="space-y-3">
				<Label htmlFor="path">{t('path', 'Path')}</Label>
				{isEditing ? (
					<input
						type="text"
						value={path}
						onChange={(e) => setValue("path", e.target.value)}
						id="path"
						placeholder="/webhook"
						className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
					/>
				) : (
					<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
						{path}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					{t('apiEndpointPathMustStartWith', 'API endpoint path (must start with /)')}
				</p>
			</div>

			<div className="space-y-3">
				<Label htmlFor="method">{t('httpMethod', 'HTTP Method')}</Label>
				{isEditing ? (
					<Select
						value={method}
						onValueChange={(value) => setValue("method", value)}
					>
						<SelectTrigger id="method">
							<SelectValue placeholder={t('selectMethod', 'Select method')} />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="GET">GET</SelectItem>
							<SelectItem value="POST">{`POST`}</SelectItem>
							<SelectItem value="PUT">PUT</SelectItem>
							<SelectItem value="PATCH">{`PATCH`}</SelectItem>
							<SelectItem value="DELETE">{`DELETE`}</SelectItem>
						</SelectContent>
					</Select>
				) : (
					<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
						{method}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					{t('httpMethodForTheApiEndpoint', 'HTTP method for the API endpoint')}
				</p>
			</div>

			<div className="space-y-4">
				<div className="flex items-center space-x-2">
					{isEditing ? (
						<Switch
							id="public_endpoint"
							checked={publicEndpoint}
							onCheckedChange={(checked) =>
								setValue("public_endpoint", checked)
							}
						/>
					) : (
						<div
							className={`h-5 w-9 rounded-full ${publicEndpoint ? "bg-primary" : "bg-muted"} flex items-center ${publicEndpoint ? "justify-end" : "justify-start"} px-0.5`}
						>
							<div className="h-4 w-4 rounded-full bg-white" />
						</div>
					)}
					<Label htmlFor="public_endpoint">{t('publicEndpoint', 'Public Endpoint')}</Label>
					{!isEditing && (
						<span className="text-sm text-muted-foreground">
							{publicEndpoint ? "Enabled" : "Disabled"}
						</span>
					)}
				</div>
				<p className="text-sm text-muted-foreground">
					{t('allowAccessWithoutAuthenticationUseWithCaution', 'Allow access without authentication (use with caution)')}
				</p>
			</div>

			{!publicEndpoint && (
				<div className="space-y-3">
					<Label htmlFor="auth_token">{t('authenticationToken', 'Authentication Token')}</Label>
					{isEditing ? (
						<input
							type="password"
							value={authToken}
							onChange={(e) => setValue("auth_token", e.target.value)}
							id="auth_token"
							placeholder={`Bearer token for authentication`}
							className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
						/>
					) : (
						<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
							{authToken ? "••••••••••••" : t('noTokenSet', 'No token set')}
						</div>
					)}
					<p className="text-sm text-muted-foreground">
						{t('requiredBearerTokenForApiRequests', 'Required Bearer token for API requests')}
					</p>
				</div>
			)}

			<div className="space-y-4 pt-4 border-t">
				<div className="flex items-center justify-between">
					<Label>{t('apiEndpoint', 'API Endpoint')}</Label>
					<Button
						variant="outline"
						size="sm"
						onClick={() => copyToClipboard(endpoint, "endpoint")}
					>
						{copiedCode === "endpoint" ? (
							<Check className="h-4 w-4" />
						) : (
							<Copy className="h-4 w-4" />
						)}
					</Button>
				</div>
				<div className="p-3 bg-muted rounded-md font-mono text-sm break-all">
					{endpoint}
				</div>
				<div className="rounded-lg border border-border bg-card p-4">
					<p className="text-sm text-muted-foreground"><Trans i18nKey="strongClassnametextforegroundlocalOnlystrongThisEndpointIsOnlyAccessibleOnYourMachineUseOneOfTheTunnelingSolutionsBelowToExposeItSecurelyOverHttps"><strong className="text-foreground">Local Only:</strong> This
						endpoint is only accessible on your machine. Use one of the
						tunneling solutions below to expose it securely over HTTPS.</Trans></p>
				</div>

				<div className="space-y-3">
					<Label className="text-base">{t('exposeYourApiSecurely', 'Expose Your API Securely')}</Label>
					<Tabs defaultValue="cloudflare" className="w-full">
						<TabsList className="grid w-full grid-cols-2">
							<TabsTrigger value="cloudflare" className="gap-2"><Trans i18nKey="cloudflareTunnelSpanClassnameroundedfullBgprimaryPx2Py05TextxsFontmediumTextprimaryforegroundRecommendedSpan">Cloudflare Tunnel
								<span className="rounded-full bg-primary px-2 py-0.5 text-xs font-medium text-primary-foreground">
									Recommended
								</span></Trans></TabsTrigger>
							<TabsTrigger value="ngrok">ngrok</TabsTrigger>
						</TabsList>

						<TabsContent value="cloudflare" className="space-y-4 mt-4">
							<div className="rounded-lg border border-green-200 dark:border-green-900 bg-green-50 dark:bg-green-950/30 p-4">
								<h4 className="font-semibold text-sm text-green-900 dark:text-green-100 mb-2">
									{t('whyCloudflareTunnel', 'Why Cloudflare Tunnel?')}
								</h4>
								<ul className="text-sm text-green-800 dark:text-green-200 space-y-1 list-disc list-inside">
									<li><Trans i18nKey="strong100FreestrongNoCreditCardNoPaidPlansRequired"><strong>100% Free</strong> - No credit card, no paid plans
										required</Trans></li>
									<li><Trans i18nKey="strongnoFirewallChangesstrongUsesOutboundConnectionsOnly"><strong>No firewall changes</strong> - Uses outbound
										connections only</Trans></li>
									<li><Trans i18nKey="strongautomaticHttpsstrongTlsCertificatesManagedForYou"><strong>Automatic HTTPS</strong> - TLS certificates managed
										for you</Trans></li>
									<li><Trans i18nKey="strongfastReliablestrongCloudflaresGlobalNetwork"><strong>Fast & reliable</strong> - Cloudflare's global
										network</Trans></li>
								</ul>
							</div>

							<div className="space-y-4">
								<div className="space-y-2">
									<h4 className="font-semibold text-sm flex items-center gap-2"><Trans i18nKey="spanClassnameflexH6W6ItemscenterJustifycenterRoundedfullBgprimaryTextxsFontboldTextprimaryforeground1SpanInstallCloudflared"><span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-bold text-primary-foreground">
											1
										</span>
										Install cloudflared</Trans></h4>
									<p className="text-sm text-muted-foreground pl-8">
										{t('downloadAndInstallTheCloudflareTunnelClientFor', 'Download and install the Cloudflare Tunnel client for')}{" "}
										<strong>{getPlatformLabel()}</strong>:
									</p>
									<div className="pl-8">
										<pre className="p-3 bg-muted rounded-md font-mono text-xs overflow-x-auto whitespace-pre-wrap">
											{getCloudflareInstallCommand()}
										</pre>
										<a
											href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/"
											target="_blank"
											rel="noopener noreferrer"
											className="text-xs text-primary hover:underline inline-flex items-center gap-1 mt-2"
										>
											{t('otherPlatformsOrInstallationMethods', 'Other platforms or installation methods')}
											<ExternalLink className="h-3 w-3" />
										</a>
									</div>
								</div>

								<div className="space-y-2">
									<h4 className="font-semibold text-sm flex items-center gap-2"><Trans i18nKey="spanClassnameflexH6W6ItemscenterJustifycenterRoundedfullBgprimaryTextxsFontboldTextprimaryforeground2SpanStartTheTunnel"><span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-bold text-primary-foreground">
											2
										</span>
										Start the tunnel</Trans></h4>
									<p className="text-sm text-muted-foreground pl-8">
										{t('runThisCommandToCreateAFreeQuickTunnel', 'Run this command to create a free Quick Tunnel:')}
									</p>
									<div className="pl-8 space-y-2">
										<pre className="p-3 bg-muted rounded-md font-mono text-xs overflow-x-auto">
											{t('cloudflaredTunnelUrlHttplocalhost9657', 'cloudflared tunnel --url http://localhost:9657')}
										</pre>
										<p className="text-xs text-muted-foreground">
											{t('thisGeneratesARandom', 'This generates a random')}{" "}
											<code>https://*****.trycloudflare.com</code> URL
										</p>
									</div>
								</div>

								<div className="space-y-2">
									<h4 className="font-semibold text-sm flex items-center gap-2"><Trans i18nKey="spanClassnameflexH6W6ItemscenterJustifycenterRoundedfullBgprimaryTextxsFontboldTextprimaryforeground3SpanUseYourEndpoint"><span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-bold text-primary-foreground">
											3
										</span>
										Use your endpoint</Trans></h4>
									<p className="text-sm text-muted-foreground pl-8">
										{t('copyTheGeneratedUrlAndAppendYourPath', 'Copy the generated URL and append your path:')}{" "}
										<code className="text-foreground">{path}</code>
									</p>
									<div className="pl-8">
										<div className="p-3 bg-muted rounded-md text-xs font-mono">
											https://random-subdomain.trycloudflare.com{path}
										</div>
									</div>
								</div>

								<div className="rounded-lg border border-blue-200 dark:border-blue-900 bg-blue-50 dark:bg-blue-950/30 p-4">
									<h4 className="font-semibold text-sm text-blue-900 dark:text-blue-100 mb-2">
										{t('proTips', '💡 Pro Tips')}
									</h4>
									<ul className="text-sm text-blue-800 dark:text-blue-200 space-y-1.5">
										<li>
											{`• Quick Tunnels generate a new random URL each time you restart`}
										</li>
										<li>
											{t('forAPermanentUrlCreateA', '• For a permanent URL, create a')}{" "}
											<a
												href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/get-started/create-remote-tunnel/"
												target="_blank"
												rel="noopener noreferrer"
												className="underline hover:no-underline"
											>
												{t('namedTunnel', 'named tunnel')}
											</a>{" "}
											{t('freeRequiresCloudflareAccount', '(free, requires Cloudflare account)')}
										</li>
										<li>
											{t('theTunnelStaysActiveAsLongAsTheCommandRuns', '• The tunnel stays active as long as the command runs')}
										</li>
									</ul>
								</div>
							</div>
						</TabsContent>

						<TabsContent value="ngrok" className="space-y-4 mt-4">
							<div className="rounded-lg border border-amber-200 dark:border-amber-900 bg-amber-50 dark:bg-amber-950/30 p-4">
								<h4 className="font-semibold text-sm text-amber-900 dark:text-amber-100 mb-2">
									{t('noteAboutNgrok', '⚠️ Note About ngrok')}
								</h4>
								<ul className="text-sm text-amber-800 dark:text-amber-200 space-y-1">
									<li><Trans i18nKey="strongfreeTierRequiresAccountstrongSignUpAtNgrokcom">• <strong>Free tier requires account</strong> - Sign up at
										ngrok.com</Trans></li>
									<li>{t('urlChangesOnEveryRestartUnlessYouUpgrade', '• URL changes on every restart (unless you upgrade)')}</li>
									<li>{t('goodForQuickTestingAndDemos', '• Good for quick testing and demos')}</li>
								</ul>
							</div>

							<div className="space-y-4">
								<div className="space-y-2">
									<h4 className="font-semibold text-sm flex items-center gap-2"><Trans i18nKey="spanClassnameflexH6W6ItemscenterJustifycenterRoundedfullBgprimaryTextxsFontboldTextprimaryforeground1SpanInstallNgrok"><span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-bold text-primary-foreground">
											1
										</span>
										Install ngrok</Trans></h4>
									<p className="text-sm text-muted-foreground pl-8">
										{t('installNgrokFor', 'Install ngrok for')} <strong>{getPlatformLabel()}</strong>:
									</p>
									<div className="pl-8 space-y-2">
										<pre className="p-3 bg-muted rounded-md font-mono text-xs overflow-x-auto whitespace-pre-wrap">
											{getNgrokInstallCommand()}
										</pre>
										<a
											href="https://dashboard.ngrok.com/get-started/setup"
											target="_blank"
											rel="noopener noreferrer"
											className="text-xs text-primary hover:underline inline-flex items-center gap-1"
										>
											{t('otherInstallationMethods', 'Other installation methods')}
											<ExternalLink className="h-3 w-3" />
										</a>
									</div>
								</div>

								<div className="space-y-2">
									<h4 className="font-semibold text-sm flex items-center gap-2"><Trans i18nKey="spanClassnameflexH6W6ItemscenterJustifycenterRoundedfullBgprimaryTextxsFontboldTextprimaryforeground2SpanAuthenticateWithYourToken"><span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-bold text-primary-foreground">
											2
										</span>
										Authenticate with your token</Trans></h4>
									<p className="text-sm text-muted-foreground pl-8">
										{t('getYourAuthTokenFrom', 'Get your auth token from')}{" "}
										<a
											href="https://dashboard.ngrok.com/get-started/your-authtoken"
											target="_blank"
											rel="noopener noreferrer"
											className="text-primary hover:underline"
										>
											{t('ngrokDashboard', 'ngrok dashboard')}
										</a>{" "}
										{t('andRun', 'and run:')}
									</p>
									<div className="pl-8">
										<pre className="p-3 bg-muted rounded-md font-mono text-xs overflow-x-auto">
											{t('ngrokConfigAddauthtokenYour_token_here', 'ngrok config add-authtoken YOUR_TOKEN_HERE')}
										</pre>
									</div>
								</div>

								<div className="space-y-2">
									<h4 className="font-semibold text-sm flex items-center gap-2"><Trans i18nKey="spanClassnameflexH6W6ItemscenterJustifycenterRoundedfullBgprimaryTextxsFontboldTextprimaryforeground3SpanStartTheTunnel"><span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-bold text-primary-foreground">
											3
										</span>
										Start the tunnel</Trans></h4>
									<div className="pl-8 space-y-2">
										<pre className="p-3 bg-muted rounded-md font-mono text-xs overflow-x-auto">
											{t('ngrokHttp9657', 'ngrok http 9657')}
										</pre>
										<p className="text-xs text-muted-foreground">
											{t('copyTheForwardingUrlAndAppendYourPath', 'Copy the forwarding URL and append your path:')}{" "}
											<code className="text-foreground">{path}</code>
										</p>
									</div>
								</div>
							</div>
						</TabsContent>
					</Tabs>
				</div>
			</div>

			<div className="space-y-3">
				<Label>{t('codeExamples', 'Code Examples')}</Label>
				<Tabs defaultValue="curl" className="w-full">
					<TabsList className="grid w-full grid-cols-4">
						<TabsTrigger value="curl">{`cURL`}</TabsTrigger>
						<TabsTrigger value="python">{t('python', 'Python')}</TabsTrigger>
						<TabsTrigger value="typescript">{t('typescript', 'TypeScript')}</TabsTrigger>
						<TabsTrigger value="rust">{t('rust', 'Rust')}</TabsTrigger>
					</TabsList>
					{["curl", "python", "typescript", "rust"].map((lang) => (
						<TabsContent key={lang} value={lang} className="space-y-2">
							<div className="relative">
								<div className="prose prose-sm max-w-none dark:prose-invert">
									<TextEditor
										initialContent={generateCodeExample(lang)}
										isMarkdown={true}
									/>
								</div>
							</div>
						</TabsContent>
					))}
				</Tabs>
			</div>

			<div className="space-y-3 pt-4 border-t">
				<Label>{t('testEndpoint', 'Test Endpoint')}</Label>
				<Dialog>
					<DialogTrigger asChild>
						<Button variant="outline" className="w-full">
							<Play className="h-4 w-4 mr-2" />
							{t('testApiRequest', 'Test API Request')}
						</Button>
					</DialogTrigger>
					<DialogContent className="max-w-2xl">
						<DialogHeader>
							<DialogTitle>{t('testApiRequest', 'Test API Request')}</DialogTitle>
							<DialogDescription>{t('sendATestRequestToEndpoint', 'Send a test request to {{endpoint}}', { endpoint })}</DialogDescription>
						</DialogHeader>
						<div className="space-y-4">
							{method !== "GET" && (
								<div className="space-y-2">
									<Label>{t('requestBodyJson', 'Request Body (JSON)')}</Label>
									<Textarea
										value={testPayload}
										onChange={(e) => setTestPayload(e.target.value)}
										placeholder={`{"key": "value"}`}
										className="font-mono text-sm"
										rows={6}
									/>
								</div>
							)}
							<Button
								onClick={handleTestRequest}
								disabled={isTesting}
								className="w-full"
							>
								{isTesting ? "Sending..." : t('sendRequest', 'Send Request')}
							</Button>
							{testResult && (
								<div className="space-y-2">
									<Label>{t('response', 'Response')}</Label>
									<pre className="p-4 bg-muted rounded-md text-sm overflow-x-auto whitespace-pre-wrap">
										{testResult}
									</pre>
								</div>
							)}
						</div>
					</DialogContent>
				</Dialog>
			</div>
		</div>
	);
}
