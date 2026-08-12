"use client";

import { Cloud, ExternalLink, Info, Laptop } from "lucide-react";
import { useMemo, useState } from "react";
import {
	Accordion,
	AccordionContent,
	AccordionItem,
	AccordionTrigger,
} from "../../ui/accordion";
import { Alert, AlertDescription, AlertTitle } from "../../ui/alert";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import type { IConfigInterfaceProps } from "../interfaces";

const GATEWAY_INTENTS = [
	{
		value: "Guilds",
		label: "Guilds",
		description: "Access to guild information",
	},
	{
		value: "GuildMembers",
		label: "Guild Members",
		description: "Access to member updates (privileged)",
	},
	{
		value: "GuildModeration",
		label: "Guild Moderation",
		description: "Access to ban/unban events",
	},
	{
		value: "GuildEmojisAndStickers",
		label: "Emojis & Stickers",
		description: "Access to emoji/sticker updates",
	},
	{
		value: "GuildIntegrations",
		label: "Integrations",
		description: "Access to integration updates",
	},
	{
		value: "GuildWebhooks",
		label: "Webhooks",
		description: "Access to webhook updates",
	},
	{
		value: "GuildInvites",
		label: "Invites",
		description: "Access to invite events",
	},
	{
		value: "GuildVoiceStates",
		label: "Voice States",
		description: "Access to voice state updates",
	},
	{
		value: "GuildPresences",
		label: "Presences",
		description: "Access to presence updates (privileged)",
	},
	{
		value: "GuildMessages",
		label: "Guild Messages",
		description: "Access to guild message events",
	},
	{
		value: "GuildMessageReactions",
		label: "Message Reactions",
		description: "Access to reaction events",
	},
	{
		value: "GuildMessageTyping",
		label: "Message Typing",
		description: "Access to typing events",
	},
	{
		value: "DirectMessages",
		label: "Direct Messages",
		description: "Access to DM events",
	},
	{
		value: "DirectMessageReactions",
		label: "DM Reactions",
		description: "Access to DM reaction events",
	},
	{
		value: "DirectMessageTyping",
		label: "DM Typing",
		description: "Access to DM typing events",
	},
	{
		value: "MessageContent",
		label: "Message Content",
		description: "Access to message content (privileged)",
	},
	{
		value: "GuildScheduledEvents",
		label: "Scheduled Events",
		description: "Access to scheduled event updates",
	},
	{
		value: "AutoModerationConfiguration",
		label: "AutoMod Config",
		description: "Access to auto-moderation config",
	},
	{
		value: "AutoModerationExecution",
		label: "AutoMod Execution",
		description: "Access to auto-moderation execution",
	},
];

const PRIVILEGED_INTENTS = ["GuildMembers", "GuildPresences", "MessageContent"];

export function DiscordConfig({
	isEditing,
	config,
	onConfigUpdate,
	hub,
	eventId,
	section,
}: IConfigInterfaceProps) {
	const [copiedCode, setCopiedCode] = useState<string | null>(null);
	const [showPublicKey, setShowPublicKey] = useState(false);

	const setValue = (key: string, value: any) => {
		if (onConfigUpdate) {
			onConfigUpdate({
				...config,
				[key]: value,
			});
		}
	};

	const token = config?.token ?? "";
	const botName = config?.bot_name ?? "My Discord Bot";
	const botDescription = config?.bot_description ?? "";
	const publicKey = (config?.webhook_secret as string) ?? ""; // Discord public key stored in webhook_secret
	const selectedIntents: string[] = config?.intents ?? [
		"Guilds",
		"GuildMessages",
		"MessageContent",
	];
	const channelWhitelist: string[] = config?.channel_whitelist ?? [];
	const channelBlacklist: string[] = config?.channel_blacklist ?? [];
	const respondToMentions = config?.respond_to_mentions ?? true;
	const respondToDMs = config?.respond_to_dms ?? true;
	const commandPrefix = config?.command_prefix ?? "!";

	// Compute webhook URLs
	const supportsRemote = hub?.supported_sinks?.discord === true;
	const remoteWebhookUrl = useMemo(() => {
		if (!hub?.domain || !eventId) return null;
		const protocol = hub.environment === "Development" ? "http" : "https";
		return `${protocol}://${hub.domain}/sink/trigger/discord/${eventId}`;
	}, [hub?.domain, hub?.environment, eventId]);

	const copyToClipboard = (text: string, id: string) => {
		navigator.clipboard.writeText(text);
		setCopiedCode(id);
		setTimeout(() => setCopiedCode(null), 2000);
	};

	const toggleIntent = (intent: string) => {
		const updated = selectedIntents.includes(intent)
			? selectedIntents.filter((i) => i !== intent)
			: [...selectedIntents, intent];
		setValue("intents", updated);
	};

	const addToWhitelist = (channelId: string) => {
		if (channelId && !channelWhitelist.includes(channelId)) {
			setValue("channel_whitelist", [...channelWhitelist, channelId]);
		}
	};

	const removeFromWhitelist = (channelId: string) => {
		setValue(
			"channel_whitelist",
			channelWhitelist.filter((id) => id !== channelId),
		);
	};

	const addToBlacklist = (channelId: string) => {
		if (channelId && !channelBlacklist.includes(channelId)) {
			setValue("channel_blacklist", [...channelBlacklist, channelId]);
		}
	};

	const removeFromBlacklist = (channelId: string) => {
		setValue(
			"channel_blacklist",
			channelBlacklist.filter((id) => id !== channelId),
		);
	};

	const hasPrivilegedIntents = selectedIntents.some((intent) =>
		PRIVILEGED_INTENTS.includes(intent),
	);

	// The events surface renders one section at a time; anywhere else (and for
	// any section this component doesn't know) it renders whole.
	const shows = (id: string) => !section || section === id;

	return (
		<div className="w-full space-y-6">
			{shows("connection") && (
				<>
					{/* Bot Token */}
					<div className="space-y-3">
						<Label htmlFor="token">Bot Token</Label>
						{isEditing ? (
							<input
								type="password"
								value={token}
								onChange={(e) => setValue("token", e.target.value)}
								id="token"
								placeholder="Your Discord bot token"
								className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
							/>
						) : (
							<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
								{token ? "••••••••••••" : "No token set"}
							</div>
						)}
						<p className="text-sm text-muted-foreground">
							Your Discord bot token from the{" "}
							<a
								href="https://discord.com/developers/applications"
								target="_blank"
								rel="noopener noreferrer"
								className="text-primary hover:underline inline-flex items-center gap-1"
							>
								Developer Portal
								<ExternalLink className="h-3 w-3" />
							</a>
						</p>
					</div>

					{/* Interactions Webhook Setup - shown when remote is supported */}
					{(supportsRemote || true) && (
						<Accordion type="single" collapsible defaultValue="webhook">
							<AccordionItem value="webhook" className="border rounded-lg px-4">
								<AccordionTrigger className="hover:no-underline">
									<div className="flex items-center gap-2">
										<Cloud className="h-4 w-4" />
										<span>Interactions Webhook Setup</span>
										{supportsRemote && remoteWebhookUrl && (
											<Badge variant="default" className="ml-2">
												Remote Available
											</Badge>
										)}
									</div>
								</AccordionTrigger>
								<AccordionContent className="space-y-4 pt-2">
									{supportsRemote && remoteWebhookUrl ? (
										<Tabs defaultValue="remote" className="w-full">
											<TabsList className="grid w-full grid-cols-2">
												<TabsTrigger value="remote">
													<Cloud className="h-3 w-3 mr-1" />
													Remote (Server)
												</TabsTrigger>
												<TabsTrigger value="local">
													<Laptop className="h-3 w-3 mr-1" />
													Local (Desktop)
												</TabsTrigger>
											</TabsList>
											<TabsContent value="remote" className="space-y-4 pt-2">
												{/* Public Key */}
												<div className="space-y-2">
													<Label>Application Public Key</Label>
													<div className="flex gap-2">
														<Input
															type={showPublicKey ? "text" : "password"}
															value={publicKey}
															onChange={(e) =>
																setValue("webhook_secret", e.target.value)
															}
															placeholder="Discord application public key"
															disabled={!isEditing}
															className="font-mono text-xs"
														/>
														<Button
															type="button"
															variant="ghost"
															size="sm"
															onClick={() => setShowPublicKey(!showPublicKey)}
														>
															{showPublicKey ? "Hide" : "Show"}
														</Button>
													</div>
													<p className="text-xs text-muted-foreground">
														Find this in your Discord Developer Portal under
														General Information → Public Key
													</p>
												</div>

												{/* Webhook URL */}
												<div className="space-y-2">
													<Label>Interactions Endpoint URL</Label>
													<div className="relative">
														<div className="flex h-auto min-h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm font-mono break-all">
															{remoteWebhookUrl}
														</div>
														<Button
															type="button"
															variant="ghost"
															size="sm"
															className="absolute right-1 top-1 h-8"
															onClick={() =>
																navigator.clipboard.writeText(remoteWebhookUrl)
															}
														>
															Copy
														</Button>
													</div>
												</div>

												{/* Setup Instructions */}
												<Alert>
													<AlertTitle>Setup Instructions</AlertTitle>
													<AlertDescription className="space-y-3">
														<ol className="text-xs list-decimal list-inside space-y-2">
															<li>
																Go to the{" "}
																<a
																	href="https://discord.com/developers/applications"
																	target="_blank"
																	rel="noopener noreferrer"
																	className="text-primary hover:underline"
																>
																	Discord Developer Portal
																</a>
															</li>
															<li>Select your application</li>
															<li>
																Copy the <strong>Public Key</strong> from
																General Information and paste it above
															</li>
															<li>
																Go to General Information → Interactions
																Endpoint URL
															</li>
															<li>
																Paste the Interactions Endpoint URL shown above
															</li>
															<li>
																Discord will verify the endpoint - make sure the
																sink is active
															</li>
														</ol>
														<p className="text-xs text-muted-foreground mt-2">
															<strong>Note:</strong> Discord verifies signatures
															using Ed25519. The public key is used to verify
															incoming webhook requests.
														</p>
													</AlertDescription>
												</Alert>
											</TabsContent>
											<TabsContent value="local" className="space-y-4 pt-2">
												<Alert>
													<AlertTitle>Local Setup (Gateway Mode)</AlertTitle>
													<AlertDescription className="space-y-3">
														<p className="text-xs">
															When running locally (desktop app), the bot
															connects via <strong>Discord Gateway</strong>{" "}
															(WebSocket) instead of webhooks. No Interactions
															Endpoint URL setup is required - the bot will
															automatically connect when the event is activated.
														</p>
														<p className="text-xs">
															<strong>Note:</strong> If you previously set an
															Interactions Endpoint URL in the Developer Portal,
															you can leave it - the local bot will use Gateway
															connection regardless.
														</p>
													</AlertDescription>
												</Alert>
											</TabsContent>
										</Tabs>
									) : (
										<Alert>
											<AlertTitle>Local Setup (Gateway Mode)</AlertTitle>
											<AlertDescription className="space-y-3">
												<p className="text-xs">
													The bot uses <strong>Discord Gateway</strong>{" "}
													(WebSocket) mode when running locally. No webhook
													setup is required - the bot will automatically connect
													when the event is activated.
												</p>
												<p className="text-xs text-muted-foreground">
													Remote webhook mode is not available for this hub
													configuration.
												</p>
											</AlertDescription>
										</Alert>
									)}
								</AccordionContent>
							</AccordionItem>
						</Accordion>
					)}
				</>
			)}

			{shows("behaviour") && (
				<>
					{/* Bot Metadata */}
					<div className="space-y-3">
						<Label htmlFor="bot_name">Bot Name</Label>
						{isEditing ? (
							<input
								type="text"
								value={botName}
								onChange={(e) => setValue("bot_name", e.target.value)}
								id="bot_name"
								placeholder="My Discord Bot"
								className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
							/>
						) : (
							<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
								{botName}
							</div>
						)}
					</div>

					<div className="space-y-3">
						<Label htmlFor="bot_description">Bot Description (Optional)</Label>
						{isEditing ? (
							<textarea
								value={botDescription}
								onChange={(e) => setValue("bot_description", e.target.value)}
								id="bot_description"
								placeholder="A helpful bot for my server"
								rows={3}
								className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
							/>
						) : (
							<div className="flex min-h-20 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
								{botDescription || "No description"}
							</div>
						)}
					</div>
				</>
			)}

			{/* Gateway Intents */}
			{shows("permissions") && (
				<div className="space-y-3 pt-4 border-t">
					<div className="flex items-center justify-between">
						<div>
							<Label>Gateway Intents</Label>
							<p className="text-sm text-muted-foreground mt-1">
								Select which events your bot should receive
							</p>
						</div>
						{hasPrivilegedIntents && (
							<Badge variant="destructive" className="flex items-center gap-1">
								<Info className="h-3 w-3" />
								Privileged
							</Badge>
						)}
					</div>

					{hasPrivilegedIntents && (
						<div className="rounded-md bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-900 p-3">
							<p className="text-sm text-yellow-800 dark:text-yellow-200">
								<strong>Note:</strong> You've selected privileged intents. These
								must be enabled in your{" "}
								<a
									href="https://discord.com/developers/applications"
									target="_blank"
									rel="noopener noreferrer"
									className="underline"
								>
									Discord Developer Portal
								</a>{" "}
								under Bot → Privileged Gateway Intents.
							</p>
						</div>
					)}

					<Accordion type="single" collapsible className="w-full">
						<AccordionItem value="intents">
							<AccordionTrigger>
								<div className="flex items-center gap-2">
									<span>Configure Intents</span>
									<Badge variant="secondary">
										{selectedIntents.length} selected
									</Badge>
								</div>
							</AccordionTrigger>
							<AccordionContent>
								<div className="space-y-2 max-h-96 overflow-y-auto">
									{GATEWAY_INTENTS.map((intent) => {
										const isPrivileged = PRIVILEGED_INTENTS.includes(
											intent.value,
										);
										const isSelected = selectedIntents.includes(intent.value);

										return (
											<div
												key={intent.value}
												className="flex items-start space-x-3 p-3 rounded-md hover:bg-muted/50"
											>
												{isEditing ? (
													<Switch
														checked={isSelected}
														onCheckedChange={() => toggleIntent(intent.value)}
														id={`intent-${intent.value}`}
													/>
												) : (
													<div
														className={`h-5 w-9 rounded-full ${isSelected ? "bg-primary" : "bg-muted"} flex items-center ${isSelected ? "justify-end" : "justify-start"} px-0.5`}
													>
														<div className="h-4 w-4 rounded-full bg-white" />
													</div>
												)}
												<div className="flex-1">
													<div className="flex items-center gap-2">
														<Label
															htmlFor={`intent-${intent.value}`}
															className="cursor-pointer"
														>
															{intent.label}
														</Label>
														{isPrivileged && (
															<Badge variant="outline" className="text-xs">
																Privileged
															</Badge>
														)}
													</div>
													<p className="text-xs text-muted-foreground mt-1">
														{intent.description}
													</p>
												</div>
											</div>
										);
									})}
								</div>
							</AccordionContent>
						</AccordionItem>
					</Accordion>
				</div>
			)}

			{/* Bot Behavior */}
			{shows("behaviour") && (
				<div className="space-y-4 pt-4 border-t">
					<Label>Bot Behavior</Label>

					<div className="flex items-center space-x-2">
						{isEditing ? (
							<Switch
								id="respond_to_mentions"
								checked={respondToMentions}
								onCheckedChange={(checked) =>
									setValue("respond_to_mentions", checked)
								}
							/>
						) : (
							<div
								className={`h-5 w-9 rounded-full ${respondToMentions ? "bg-primary" : "bg-muted"} flex items-center ${respondToMentions ? "justify-end" : "justify-start"} px-0.5`}
							>
								<div className="h-4 w-4 rounded-full bg-white" />
							</div>
						)}
						<Label htmlFor="respond_to_mentions">
							Respond only to Mentions
						</Label>
					</div>

					<div className="flex items-center space-x-2">
						{isEditing ? (
							<Switch
								id="respond_to_dms"
								checked={respondToDMs}
								onCheckedChange={(checked) =>
									setValue("respond_to_dms", checked)
								}
							/>
						) : (
							<div
								className={`h-5 w-9 rounded-full ${respondToDMs ? "bg-primary" : "bg-muted"} flex items-center ${respondToDMs ? "justify-end" : "justify-start"} px-0.5`}
							>
								<div className="h-4 w-4 rounded-full bg-white" />
							</div>
						)}
						<Label htmlFor="respond_to_dms">Respond to Direct Messages</Label>
					</div>

					<div className="space-y-3">
						<Label htmlFor="command_prefix">Command Prefix</Label>
						{isEditing ? (
							<input
								type="text"
								value={commandPrefix}
								onChange={(e) => setValue("command_prefix", e.target.value)}
								id="command_prefix"
								placeholder="!"
								maxLength={5}
								className="flex h-10 w-32 rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
							/>
						) : (
							<div className="flex h-10 w-32 rounded-md border border-input bg-muted px-3 py-2 text-sm">
								{commandPrefix}
							</div>
						)}
						<p className="text-sm text-muted-foreground">
							Prefix for bot commands (e.g., !help)
						</p>
					</div>
				</div>
			)}

			{/* Channel Filters */}
			{shows("channels") && (
				<div className="space-y-4 pt-4 border-t">
					<Label>Channel Filters</Label>
					<p className="text-sm text-muted-foreground">
						Control which channels the bot monitors. If whitelist is set, only
						those channels are monitored.
					</p>

					<Accordion type="single" collapsible className="w-full">
						<AccordionItem value="whitelist">
							<AccordionTrigger>
								<div className="flex items-center gap-2">
									<span>Channel Whitelist</span>
									<Badge variant="secondary">
										{channelWhitelist.length} channels
									</Badge>
								</div>
							</AccordionTrigger>
							<AccordionContent>
								<div className="space-y-3">
									{isEditing && (
										<div className="flex gap-2">
											<input
												type="text"
												placeholder="Channel ID"
												id="whitelist-input"
												className="flex h-10 flex-1 rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
												onKeyDown={(e) => {
													if (e.key === "Enter") {
														const input = e.currentTarget;
														addToWhitelist(input.value);
														input.value = "";
													}
												}}
											/>
											<Button
												type="button"
												onClick={() => {
													const input = document.getElementById(
														"whitelist-input",
													) as HTMLInputElement;
													if (input) {
														addToWhitelist(input.value);
														input.value = "";
													}
												}}
											>
												Add
											</Button>
										</div>
									)}
									<div className="space-y-2">
										{channelWhitelist.length === 0 ? (
											<p className="text-sm text-muted-foreground">
												No channels in whitelist (all channels allowed)
											</p>
										) : (
											channelWhitelist.map((channelId) => (
												<div
													key={channelId}
													className="flex items-center justify-between p-2 rounded-md bg-muted"
												>
													<span className="text-sm font-mono">{channelId}</span>
													{isEditing && (
														<Button
															variant="ghost"
															size="sm"
															onClick={() => removeFromWhitelist(channelId)}
														>
															Remove
														</Button>
													)}
												</div>
											))
										)}
									</div>
								</div>
							</AccordionContent>
						</AccordionItem>

						<AccordionItem value="blacklist">
							<AccordionTrigger>
								<div className="flex items-center gap-2">
									<span>Channel Blacklist</span>
									<Badge variant="secondary">
										{channelBlacklist.length} channels
									</Badge>
								</div>
							</AccordionTrigger>
							<AccordionContent>
								<div className="space-y-3">
									{isEditing && (
										<div className="flex gap-2">
											<input
												type="text"
												placeholder="Channel ID"
												id="blacklist-input"
												className="flex h-10 flex-1 rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
												onKeyDown={(e) => {
													if (e.key === "Enter") {
														const input = e.currentTarget;
														addToBlacklist(input.value);
														input.value = "";
													}
												}}
											/>
											<Button
												type="button"
												onClick={() => {
													const input = document.getElementById(
														"blacklist-input",
													) as HTMLInputElement;
													if (input) {
														addToBlacklist(input.value);
														input.value = "";
													}
												}}
											>
												Add
											</Button>
										</div>
									)}
									<div className="space-y-2">
										{channelBlacklist.length === 0 ? (
											<p className="text-sm text-muted-foreground">
												No channels in blacklist
											</p>
										) : (
											channelBlacklist.map((channelId) => (
												<div
													key={channelId}
													className="flex items-center justify-between p-2 rounded-md bg-muted"
												>
													<span className="text-sm font-mono">{channelId}</span>
													{isEditing && (
														<Button
															variant="ghost"
															size="sm"
															onClick={() => removeFromBlacklist(channelId)}
														>
															Remove
														</Button>
													)}
												</div>
											))
										)}
									</div>
								</div>
							</AccordionContent>
						</AccordionItem>
					</Accordion>
				</div>
			)}

			{/* Setup Instructions — belongs with the credentials they produce */}
			{shows("connection") && (
				<div className="space-y-3 pt-4 border-t">
					<Label>Setup Instructions</Label>
					<Accordion type="single" collapsible className="w-full">
						<AccordionItem value="setup">
							<AccordionTrigger>How to create a Discord bot</AccordionTrigger>
							<AccordionContent className="space-y-3 text-sm">
								<ol className="list-decimal list-inside space-y-2">
									<li>
										Go to the{" "}
										<a
											href="https://discord.com/developers/applications"
											target="_blank"
											rel="noopener noreferrer"
											className="text-primary hover:underline"
										>
											Discord Developer Portal
										</a>
									</li>
									<li>Click "New Application" and give it a name</li>
									<li>Go to the "Bot" section and click "Add Bot"</li>
									<li>
										Under "Token", click "Reset Token" to get your bot token
									</li>
									<li>
										Enable the required Privileged Gateway Intents if needed
									</li>
									<li>
										Go to "OAuth2" → "URL Generator"
										<ul className="list-disc list-inside ml-4 mt-1">
											<li>
												Select scope:{" "}
												<code className="bg-muted px-1 rounded">bot</code>
											</li>
											<li>
												Select permissions:{" "}
												<code className="bg-muted px-1 rounded">
													Send Messages
												</code>
												,{" "}
												<code className="bg-muted px-1 rounded">
													Read Messages
												</code>
											</li>
										</ul>
									</li>
									<li>
										Copy the generated URL and invite the bot to your server
									</li>
								</ol>
							</AccordionContent>
						</AccordionItem>
					</Accordion>
				</div>
			)}
		</div>
	);
}
