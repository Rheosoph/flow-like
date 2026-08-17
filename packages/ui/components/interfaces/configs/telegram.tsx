"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Cloud,
	ExternalLink,
	Info,
	Laptop,
	Loader2,
	Plus,
	RefreshCw,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
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

interface WebhookInfo {
	url: string;
	has_custom_certificate: boolean;
	pending_update_count: number;
	last_error_date?: number;
	last_error_message?: string;
	max_connections?: number;
}

interface TelegramApiResponse<T> {
	ok: boolean;
	result?: T;
	description?: string;
}

export function TelegramConfig({
	isEditing,
	config,
	onConfigUpdate,
	hub,
	eventId,
	section,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const [newWhitelistId, setNewWhitelistId] = useState("");
	const [newBlacklistId, setNewBlacklistId] = useState("");
	const [showSecret, setShowSecret] = useState(false);

	// Webhook management state
	const [webhookStatus, setWebhookStatus] = useState<WebhookInfo | null>(null);
	const [webhookLoading, setWebhookLoading] = useState(false);
	const [webhookError, setWebhookError] = useState<string | null>(null);
	const [webhookSuccess, setWebhookSuccess] = useState<string | null>(null);

	const setValue = (key: string, value: unknown) => {
		if (onConfigUpdate) {
			onConfigUpdate({
				...config,
				[key]: value,
			});
		}
	};

	const botToken = (config?.bot_token as string) ?? "";
	const botName = (config?.bot_name as string) ?? t('myTelegramBot', 'My Telegram Bot');
	const botDescription = (config?.bot_description as string) ?? "";
	const chatWhitelist: string[] = (config?.chat_whitelist as string[]) ?? [];
	const chatBlacklist: string[] = (config?.chat_blacklist as string[]) ?? [];
	const respondToMentions = (config?.respond_to_mentions as boolean) ?? true;
	const respondToPrivate = (config?.respond_to_private as boolean) ?? true;
	const commandPrefix = (config?.command_prefix as string) ?? "/";
	const webhookSecret = (config?.webhook_secret as string) ?? "";

	// Compute webhook URLs
	const supportsRemote = hub?.supported_sinks?.telegram === true;
	const remoteWebhookUrl = useMemo(() => {
		if (!hub?.domain || !eventId) return null;
		const protocol = hub.environment === "Development" ? "http" : "https";
		return `${protocol}://${hub.domain}/sink/trigger/telegram/${eventId}`;
	}, [hub?.domain, hub?.environment, eventId]);

	// Generate webhook secret
	const generateWebhookSecret = () => {
		const array = new Uint8Array(32);
		crypto.getRandomValues(array);
		const secret = Array.from(array, (byte) =>
			byte.toString(16).padStart(2, "0"),
		).join("");
		setValue("webhook_secret", secret);
	};

	// Deliberately not auto-generated on mount: writing config the user never
	// touched marks the event as having unsaved changes the moment they open it.
	// The secret is generated when they set up the webhook, or from the button.

	// Telegram API helper - uses proxy endpoint to avoid CORS
	const telegramApi = useCallback(
		async <T,>(
			method: string,
			params?: Record<string, unknown>,
		): Promise<TelegramApiResponse<T>> => {
			if (!botToken) {
				return { ok: false, description: t('botTokenIsRequired', 'Bot token is required') };
			}

			try {
				// Try direct API first (works in desktop app / Tauri)
				const response = await fetch(
					`https://api.telegram.org/bot${botToken}/${method}`,
					{
						method: "POST",
						headers: { "Content-Type": "application/json" },
						body: params ? JSON.stringify(params) : undefined,
					},
				);
				return await response.json();
			} catch {
				// If CORS fails, we need to show manual instructions
				return {
					ok: false,
					description:
						`CORS blocked - use the manual commands below or run from desktop app`,
				};
			}
		},
		[botToken],
	);

	// Check webhook status
	const checkWebhookStatus = useCallback(async () => {
		if (!botToken) return;

		setWebhookLoading(true);
		setWebhookError(null);

		const result = await telegramApi<WebhookInfo>("getWebhookInfo");

		if (result.ok && result.result) {
			setWebhookStatus(result.result);
		} else {
			setWebhookError(result.description || "Failed to get webhook info");
		}

		setWebhookLoading(false);
	}, [botToken, telegramApi]);

	// Set webhook for remote mode
	const setWebhook = useCallback(async () => {
		if (!botToken || !remoteWebhookUrl) return;

		// Ensure we have a secret
		let secret = webhookSecret;
		if (!secret) {
			const array = new Uint8Array(32);
			crypto.getRandomValues(array);
			secret = Array.from(array, (byte) =>
				byte.toString(16).padStart(2, "0"),
			).join("");
			setValue("webhook_secret", secret);
		}

		setWebhookLoading(true);
		setWebhookError(null);
		setWebhookSuccess(null);

		const result = await telegramApi<boolean>("setWebhook", {
			url: remoteWebhookUrl,
			secret_token: secret,
		});

		if (result.ok) {
			setWebhookSuccess("Webhook configured successfully!");
			await checkWebhookStatus();
		} else {
			setWebhookError(result.description || "Failed to set webhook");
		}

		setWebhookLoading(false);
	}, [
		botToken,
		remoteWebhookUrl,
		webhookSecret,
		telegramApi,
		checkWebhookStatus,
		setValue,
	]);

	// Delete webhook for local mode
	const deleteWebhook = useCallback(async () => {
		if (!botToken) return;

		setWebhookLoading(true);
		setWebhookError(null);
		setWebhookSuccess(null);

		const result = await telegramApi<boolean>("deleteWebhook");

		if (result.ok) {
			setWebhookSuccess("Webhook deleted - bot will use polling mode");
			await checkWebhookStatus();
		} else {
			setWebhookError(result.description || "Failed to delete webhook");
		}

		setWebhookLoading(false);
	}, [botToken, telegramApi, checkWebhookStatus]);

	// Check webhook status when token changes
	useEffect(() => {
		if (botToken && botToken.includes(":")) {
			checkWebhookStatus();
		} else {
			setWebhookStatus(null);
		}
	}, [botToken, checkWebhookStatus]);

	const addToWhitelist = () => {
		if (newWhitelistId && !chatWhitelist.includes(newWhitelistId)) {
			setValue("chat_whitelist", [...chatWhitelist, newWhitelistId]);
			setNewWhitelistId("");
		}
	};

	const removeFromWhitelist = (chatId: string) => {
		setValue(
			"chat_whitelist",
			chatWhitelist.filter((id) => id !== chatId),
		);
	};

	const addToBlacklist = () => {
		if (newBlacklistId && !chatBlacklist.includes(newBlacklistId)) {
			setValue("chat_blacklist", [...chatBlacklist, newBlacklistId]);
			setNewBlacklistId("");
		}
	};

	const removeFromBlacklist = (chatId: string) => {
		setValue(
			"chat_blacklist",
			chatBlacklist.filter((id) => id !== chatId),
		);
	};

	// The events surface renders one section at a time; anywhere else (and for
	// any section this component doesn't know) it renders whole.
	const shows = (id: string) => !section || section === id;

	return (
		<div className="w-full space-y-6">
			{shows("connection") && (
				<>
					{/* Bot Token */}
					<div className="space-y-3">
						<Label htmlFor="bot_token">{t('botToken', 'Bot Token')}</Label>
						{isEditing ? (
							<Input
								type="password"
								value={botToken}
								onChange={(e) => setValue("bot_token", e.target.value)}
								id="bot_token"
								placeholder={t('enterYourTelegramBotToken', 'Enter your Telegram bot token')}
								className="font-mono"
							/>
						) : (
							<div className="text-sm text-muted-foreground font-mono">
								{botToken ? "••••••••••••••••" : t('notSet', 'Not set')}
							</div>
						)}
						<p className="text-xs text-muted-foreground">
							{t('getYourBotTokenFrom', 'Get your bot token from')}{" "}
							<a
								href="https://t.me/BotFather"
								target="_blank"
								rel="noopener noreferrer"
								className="text-primary hover:underline inline-flex items-center gap-1"
							>
								{t('botfather', '@BotFather')} <ExternalLink className="h-3 w-3" />
							</a>
						</p>
					</div>

					{/* Webhook Setup - shown when remote is supported */}
					{(supportsRemote || true) && (
						<Accordion type="single" collapsible defaultValue="webhook">
							<AccordionItem value="webhook" className="border rounded-lg px-4">
								<AccordionTrigger className="hover:no-underline">
									<div className="flex items-center gap-2">
										<Cloud className="h-4 w-4" />
										<span>{t('webhookSetup', 'Webhook Setup')}</span>
										{webhookStatus && (
											<Badge
												variant={webhookStatus.url ? "default" : "secondary"}
												className="ml-2"
											>
												{webhookStatus.url ? t('webhookActive', 'Webhook Active') : "Polling Mode"}
											</Badge>
										)}
									</div>
								</AccordionTrigger>
								<AccordionContent className="space-y-4 pt-2">
									{/* Current Status */}
									{botToken && (
										<div className="space-y-3">
											<div className="flex items-center justify-between">
												<Label>{t('currentMode', 'Current Mode')}</Label>
												<Button
													type="button"
													variant="ghost"
													size="sm"
													onClick={checkWebhookStatus}
													disabled={webhookLoading}
												>
													{webhookLoading ? (
														<Loader2 className="h-4 w-4 animate-spin" />
													) : (
														<RefreshCw className="h-4 w-4" />
													)}
												</Button>
											</div>
											{webhookStatus ? (
												<div className="text-sm p-3 bg-muted rounded-md space-y-1">
													<div className="flex items-center gap-2">
														{webhookStatus.url ? (
															<>
																<Cloud className="h-4 w-4 text-green-500" />
																<span className="text-green-600 dark:text-green-400 font-medium">
																	{t('remoteWebhook', 'Remote (Webhook)')}
																</span>
															</>
														) : (
															<>
																<Laptop className="h-4 w-4 text-blue-500" />
																<span className="text-blue-600 dark:text-blue-400 font-medium">
																	{t('localPolling', 'Local (Polling)')}
																</span>
															</>
														)}
													</div>
													{webhookStatus.url && (
														<p className="text-xs text-muted-foreground font-mono break-all">
															{webhookStatus.url}
														</p>
													)}
													{webhookStatus.pending_update_count > 0 && (
														<p className="text-xs text-amber-600">{t('pending_update_countPendingUpdates', "{{pending_update_count}} pending updates", { pending_update_count: webhookStatus.pending_update_count })}</p>
													)}
													{webhookStatus.last_error_message && (
														<p className="text-xs text-red-600">{t('lastErrorLast_error_message', 'Last error: {{last_error_message}}', { last_error_message: webhookStatus.last_error_message })}</p>
													)}
												</div>
											) : webhookLoading ? (
												<div className="text-sm text-muted-foreground">
													{t('checkingStatus', 'Checking status...')}
												</div>
											) : null}

											{/* Success/Error Messages */}
											{webhookSuccess && (
												<Alert>
													<AlertDescription className="text-green-600">
														{webhookSuccess}
													</AlertDescription>
												</Alert>
											)}
											{webhookError && (
												<Alert variant="destructive">
													<AlertDescription>{webhookError}</AlertDescription>
												</Alert>
											)}
										</div>
									)}

									{supportsRemote && remoteWebhookUrl ? (
										<Tabs defaultValue="remote" className="w-full">
											<TabsList className="grid w-full grid-cols-2">
												<TabsTrigger value="remote">
													<Cloud className="h-3 w-3 mr-1" />
													{t('remoteServer', 'Remote (Server)')}
												</TabsTrigger>
												<TabsTrigger value="local">
													<Laptop className="h-3 w-3 mr-1" />
													{t('localDesktop', 'Local (Desktop)')}
												</TabsTrigger>
											</TabsList>
											<TabsContent value="remote" className="space-y-4 pt-2">
												{/* Webhook Secret */}
												<div className="space-y-2">
													<Label>{t('webhookSecret', 'Webhook Secret')}</Label>
													<div className="flex gap-2">
														<Input
															type={showSecret ? "text" : "password"}
															value={webhookSecret}
															onChange={(e) =>
																setValue("webhook_secret", e.target.value)
															}
															placeholder={t('webhookVerificationSecret', 'Webhook verification secret')}
															disabled={!isEditing}
															className="font-mono text-xs"
														/>
														<Button
															type="button"
															variant="ghost"
															size="sm"
															onClick={() => setShowSecret(!showSecret)}
														>
															{showSecret ? "Hide" : "Show"}
														</Button>
														{isEditing && (
															<Button
																type="button"
																variant="secondary"
																onClick={generateWebhookSecret}
															>
																{t('generate', 'Generate')}
															</Button>
														)}
													</div>
													<p className="text-xs text-muted-foreground">
														{t('thisSecretIsSentByTelegramInThe', 'This secret is sent by Telegram in the')}{" "}<Trans i18nKey="codextelegrambotapisecrettokencodeHeader"><code>X-Telegram-Bot-Api-Secret-Token</code> header.</Trans></p>
												</div>

												{/* Webhook URL */}
												<div className="space-y-2">
													<Label>{t('webhookUrl', 'Webhook URL')}</Label>
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
															{t('copy', 'Copy')}
														</Button>
													</div>
												</div>

												{/* Quick Setup Button */}
												<div className="flex gap-2">
													<Button
														type="button"
														onClick={setWebhook}
														disabled={webhookLoading || !botToken}
														className="flex-1"
													>
														{webhookLoading ? (
															<Loader2 className="h-4 w-4 mr-2 animate-spin" />
														) : (
															<Cloud className="h-4 w-4 mr-2" />
														)}
														{t('enableRemoteMode', 'Enable Remote Mode')}
													</Button>
												</div>

												{/* Manual Instructions (collapsed) */}
												<Accordion type="single" collapsible className="w-full">
													<AccordionItem value="manual" className="border-0">
														<AccordionTrigger className="text-xs text-muted-foreground py-2">
															{t('manualSetupIfAutomaticFails', 'Manual Setup (if automatic fails)')}
														</AccordionTrigger>
														<AccordionContent>
															<Alert>
																<AlertDescription className="space-y-3">
																	<p className="text-xs">
																		{t('configureYourTelegramBotWebhookUsingThisCommand', "Configure your Telegram bot webhook using this command:")}
																	</p>
																	<pre className="text-xs bg-muted p-3 rounded-md overflow-x-auto">
																		{`curl -X POST "https://api.telegram.org/bot${botToken || "<YOUR_BOT_TOKEN>"}/setWebhook" \\
  -H "Content-Type: application/json" \\
  -d '{
    "url": "${remoteWebhookUrl}",
    "secret_token": "${webhookSecret || "<YOUR_SECRET>"}"
  }'`}
																	</pre>
																	<Button
																		type="button"
																		variant="outline"
																		size="sm"
																		onClick={() => {
																			const cmd = `curl -X POST "https://api.telegram.org/bot${botToken || "<YOUR_BOT_TOKEN>"}/setWebhook" -H "Content-Type: application/json" -d '{"url": "${remoteWebhookUrl}", "secret_token": "${webhookSecret || "<YOUR_SECRET>"}"}'`;
																			navigator.clipboard.writeText(cmd);
																		}}
																	>
																		{t('copyCommand', 'Copy Command')}
																	</Button>
																</AlertDescription>
															</Alert>
														</AccordionContent>
													</AccordionItem>
												</Accordion>
											</TabsContent>
											<TabsContent value="local" className="space-y-4 pt-2">
												<p className="text-sm text-muted-foreground">
													{t('whenRunningLocallyDesktopAppTheBotUses', 'When running locally (desktop app), the bot uses')}{" "}<Trans i18nKey="stronglongPollingstrongInsteadOfWebhooksNoAdditionalSetupIsRequired"><strong>long polling</strong> instead of webhooks. No
													additional setup is required.</Trans></p>

												{/* Quick Setup Button */}
												<Button
													type="button"
													onClick={deleteWebhook}
													disabled={webhookLoading || !botToken}
													variant="outline"
													className="w-full"
												>
													{webhookLoading ? (
														<Loader2 className="h-4 w-4 mr-2 animate-spin" />
													) : (
														<Laptop className="h-4 w-4 mr-2" />
													)}
													{t('enableLocalModeDeleteWebhook', 'Enable Local Mode (Delete Webhook)')}
												</Button>

												{/* Manual Instructions (collapsed) */}
												<Accordion type="single" collapsible className="w-full">
													<AccordionItem value="manual" className="border-0">
														<AccordionTrigger className="text-xs text-muted-foreground py-2">
															{t('manualSetupIfAutomaticFails', 'Manual Setup (if automatic fails)')}
														</AccordionTrigger>
														<AccordionContent>
															<Alert>
																<AlertDescription className="space-y-3">
																	<p className="text-xs">
																		{t('deleteTheWebhookToEnablePollingMode', 'Delete the webhook to enable polling mode:')}
																	</p>
																	<pre className="text-xs bg-muted p-3 rounded-md overflow-x-auto">
																		{`curl -X POST "https://api.telegram.org/bot${botToken || "<YOUR_BOT_TOKEN>"}/deleteWebhook"`}
																	</pre>
																	<Button
																		type="button"
																		variant="outline"
																		size="sm"
																		onClick={() => {
																			const cmd = `curl -X POST "https://api.telegram.org/bot${botToken || "<YOUR_BOT_TOKEN>"}/deleteWebhook"`;
																			navigator.clipboard.writeText(cmd);
																		}}
																	>
																		{t('copyCommand', 'Copy Command')}
																	</Button>
																</AlertDescription>
															</Alert>
														</AccordionContent>
													</AccordionItem>
												</Accordion>
											</TabsContent>
										</Tabs>
									) : (
										<Alert>
											<AlertTitle>{t('localSetupPollingMode', 'Local Setup (Polling Mode)')}</AlertTitle>
											<AlertDescription className="space-y-3">
												<p className="text-xs"><Trans i18nKey="theBotUsesStronglongPollingstrongModeWhenRunningLocallyNoWebhookSetupIsRequiredTheBotWillAutomaticallyStartPollingWhenTheEventIsActivated">The bot uses <strong>long polling</strong> mode when
													running locally. No webhook setup is required - the
													bot will automatically start polling when the event is
													activated.</Trans></p>
												<p className="text-xs text-muted-foreground">
													{t('remoteWebhookModeIsNotAvailableForThisHubConfiguration', "Remote webhook mode is not available for this hub configuration.")}
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

			{/* Bot Identity */}
			{shows("behaviour") && (
				<Accordion type="single" collapsible defaultValue="identity">
					<AccordionItem value="identity" className="border rounded-lg px-4">
						<AccordionTrigger className="hover:no-underline">
							<div className="flex items-center gap-2">
								<Info className="h-4 w-4" />
								<span>{t('botIdentity', 'Bot Identity')}</span>
							</div>
						</AccordionTrigger>
						<AccordionContent className="space-y-4 pt-2">
							<div className="space-y-2">
								<Label htmlFor="bot_name">{t('botName', 'Bot Name')}</Label>
								{isEditing ? (
									<Input
										value={botName}
										onChange={(e) => setValue("bot_name", e.target.value)}
										id="bot_name"
										placeholder={t('myTelegramBot', 'My Telegram Bot')}
									/>
								) : (
									<div className="text-sm text-muted-foreground">{botName}</div>
								)}
							</div>

							<div className="space-y-2">
								<Label htmlFor="bot_description">{t('botDescription', 'Bot Description')}</Label>
								{isEditing ? (
									<Input
										value={botDescription}
										onChange={(e) =>
											setValue("bot_description", e.target.value)
										}
										id="bot_description"
										placeholder={t('aHelpfulBotPoweredByFlowlike', 'A helpful bot powered by Flow-Like')}
									/>
								) : (
									<div className="text-sm text-muted-foreground">
										{botDescription || t('notSet', 'Not set')}
									</div>
								)}
							</div>
						</AccordionContent>
					</AccordionItem>
				</Accordion>
			)}

			{/* Response Settings */}
			{shows("behaviour") && (
				<Accordion type="single" collapsible defaultValue="responses">
					<AccordionItem value="responses" className="border rounded-lg px-4">
						<AccordionTrigger className="hover:no-underline">
							<div className="flex items-center gap-2">
								<Info className="h-4 w-4" />
								<span>{t('responseSettings', 'Response Settings')}</span>
							</div>
						</AccordionTrigger>
						<AccordionContent className="space-y-4 pt-2">
							<div className="flex items-center justify-between">
								<div className="space-y-0.5">
									<Label>{t('respondToMentions', 'Respond to Mentions')}</Label>
									<p className="text-xs text-muted-foreground">
										{t('botWillRespondWhenMentionedInGroupChats', 'Bot will respond when mentioned in group chats')}
									</p>
								</div>
								<Switch
									checked={respondToMentions}
									onCheckedChange={(checked) =>
										setValue("respond_to_mentions", checked)
									}
									disabled={!isEditing}
								/>
							</div>

							<div className="flex items-center justify-between">
								<div className="space-y-0.5">
									<Label>{t('respondToPrivateMessages', 'Respond to Private Messages')}</Label>
									<p className="text-xs text-muted-foreground">
										{t('botWillRespondToDirectMessages', 'Bot will respond to direct messages')}
									</p>
								</div>
								<Switch
									checked={respondToPrivate}
									onCheckedChange={(checked) =>
										setValue("respond_to_private", checked)
									}
									disabled={!isEditing}
								/>
							</div>

							<div className="space-y-2">
								<Label htmlFor="command_prefix">{t('commandPrefix', 'Command Prefix')}</Label>
								{isEditing ? (
									<Input
										value={commandPrefix}
										onChange={(e) => setValue("command_prefix", e.target.value)}
										id="command_prefix"
										placeholder="/"
										className="w-20"
									/>
								) : (
									<div className="text-sm text-muted-foreground font-mono">
										{commandPrefix}
									</div>
								)}
								<p className="text-xs text-muted-foreground">
									{t('prefixForBotCommandsDefault', 'Prefix for bot commands (default: /)')}
								</p>
							</div>
						</AccordionContent>
					</AccordionItem>
				</Accordion>
			)}

			{/* Chat Filters */}
			{shows("chats") && (
				<Accordion type="single" collapsible>
					<AccordionItem value="filters" className="border rounded-lg px-4">
						<AccordionTrigger className="hover:no-underline">
							<div className="flex items-center gap-2">
								<Info className="h-4 w-4" />
								<span>{t('chatFilters', 'Chat Filters')}</span>
								{(chatWhitelist.length > 0 || chatBlacklist.length > 0) && (
									<Badge variant="secondary" className="ml-2">
										{chatWhitelist.length + chatBlacklist.length} configured
									</Badge>
								)}
							</div>
						</AccordionTrigger>
						<AccordionContent className="space-y-4 pt-2">
							{/* Whitelist */}
							<div className="space-y-2">
								<Label>{t('chatWhitelist', 'Chat Whitelist')}</Label>
								<p className="text-xs text-muted-foreground">
									{t('ifSetBotWillOnlyRespondInTheseChatsLeaveEmptyToAllowAll', "If set, bot will only respond in these chats. Leave empty to allow all.")}
								</p>
								{isEditing && (
									<div className="flex gap-2">
										<Input
											value={newWhitelistId}
											onChange={(e) => setNewWhitelistId(e.target.value)}
											placeholder={t('chatIdEg1001234567890', 'Chat ID (e.g., -1001234567890)')}
											className="font-mono"
										/>
										<Button
											type="button"
											variant="outline"
											size="icon"
											onClick={addToWhitelist}
										>
											<Plus className="h-4 w-4" />
										</Button>
									</div>
								)}
								<div className="flex flex-wrap gap-2">
									{chatWhitelist.map((chatId) => (
										<Badge
											key={chatId}
											variant="secondary"
											className="font-mono flex items-center gap-1"
										>
											{chatId}
											{isEditing && (
												<button
													type="button"
													onClick={() => removeFromWhitelist(chatId)}
													className="ml-1 hover:text-destructive"
												>
													<X className="h-3 w-3" />
												</button>
											)}
										</Badge>
									))}
									{chatWhitelist.length === 0 && (
										<span className="text-xs text-muted-foreground">
											{t('noWhitelistConfiguredAllChatsAllowed', 'No whitelist configured (all chats allowed)')}
										</span>
									)}
								</div>
							</div>

							{/* Blacklist */}
							<div className="space-y-2">
								<Label>{t('chatBlacklist', 'Chat Blacklist')}</Label>
								<p className="text-xs text-muted-foreground">
									{t('botWillNeverRespondInTheseChats', 'Bot will never respond in these chats')}
								</p>
								{isEditing && (
									<div className="flex gap-2">
										<Input
											value={newBlacklistId}
											onChange={(e) => setNewBlacklistId(e.target.value)}
											placeholder={t('chatIdEg1001234567890', 'Chat ID (e.g., -1001234567890)')}
											className="font-mono"
										/>
										<Button
											type="button"
											variant="outline"
											size="icon"
											onClick={addToBlacklist}
										>
											<Plus className="h-4 w-4" />
										</Button>
									</div>
								)}
								<div className="flex flex-wrap gap-2">
									{chatBlacklist.map((chatId) => (
										<Badge
											key={chatId}
											variant="destructive"
											className="font-mono flex items-center gap-1"
										>
											{chatId}
											{isEditing && (
												<button
													type="button"
													onClick={() => removeFromBlacklist(chatId)}
													className="ml-1 hover:text-destructive-foreground"
												>
													<X className="h-3 w-3" />
												</button>
											)}
										</Badge>
									))}
									{chatBlacklist.length === 0 && (
										<span className="text-xs text-muted-foreground">
											{t('noBlacklistConfigured', 'No blacklist configured')}
										</span>
									)}
								</div>
							</div>

							<p className="text-xs text-muted-foreground bg-muted p-2 rounded"><Trans i18nKey="strongtipstrongToGetAChatIdForwardAMessageFromTheChatToUserinfobotOrUseTheTelegramApi"><strong>Tip:</strong> To get a chat ID, forward a message from
								the chat to @userinfobot or use the Telegram API.</Trans></p>
						</AccordionContent>
					</AccordionItem>
				</Accordion>
			)}
		</div>
	);
}
