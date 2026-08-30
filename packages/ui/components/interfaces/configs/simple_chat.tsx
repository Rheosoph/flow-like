"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import {
	CHAT_PLACEHOLDER_BUBBLE_STATES,
	CHAT_PLACEHOLDER_VISUALS,
	DEFAULT_CHAT_AI_DISCLOSURE,
	DEFAULT_CHAT_EXAMPLE_MESSAGES,
	chatPlaceholderSupportsTypingMotion,
	resolveChatPlaceholderBubbleState,
	resolveChatPlaceholderTypingMotion,
	resolveChatPlaceholderVisual,
} from "../../../lib/chat-appearance";
import {
	CHAT_THEME_PRESETS,
	CUSTOM_CHAT_THEME_VALUE,
	resolveChatThemePreset,
} from "../../../lib/chat-theme-presets";
import { useBackend } from "../../../state/backend-state";
import type { IRouteMapping } from "../../../state/backend-state/route-state";
import { AssetPicker } from "../../builder/AssetPicker";
import {
	Checkbox,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Switch,
	Textarea,
} from "../../ui";
import { MonacoCodeEditor } from "../../ui/monaco-code-editor";
import type { IConfigInterfaceProps } from "../interfaces";

function ThemeSwatch({
	preview,
}: Readonly<{
	preview?: { readonly background: string; readonly accent: string };
}>) {
	return (
		<span
			aria-hidden="true"
			className="relative size-6 shrink-0 overflow-hidden rounded-md border border-border/70 shadow-sm"
			style={{
				background:
					preview?.background ??
					"repeating-linear-gradient(135deg, transparent 0 4px, currentColor 4px 5px)",
			}}
		>
			<span
				className="absolute right-0.5 bottom-0.5 size-2 rounded-full border border-white/70 shadow-sm"
				style={{ backgroundColor: preview?.accent ?? "currentColor" }}
			/>
		</span>
	);
}

export function SimpleChatConfig({
	isEditing,
	appId,
	boardId,
	config,
	nodeId,
	node,
	onConfigUpdate,
	section,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const routesQuery = useInvoke<IRouteMapping[], [appId: string]>(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		!!appId,
		[appId],
	);

	const routes = useMemo(() => {
		const list = routesQuery.data ?? [];
		return list.slice().sort((a, b) => a.path.localeCompare(b.path));
	}, [routesQuery.data]);

	const setValue = (key: string, value: any, deleteKeys: string[] = []) => {
		if (!onConfigUpdate) return;
		const next = { ...(config as any) };
		for (const k of deleteKeys) {
			delete next[k];
		}
		next[key] = value;
		onConfigUpdate(next);
	};

	const normalizeRoute = (value: string): string => {
		const trimmed = value.trim();
		if (!trimmed) return "";
		return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
	};

	const selectedRoutes = useMemo(() => {
		const rawArray = (config as any)?.navigate_to_routes;
		const raw: string[] = Array.isArray(rawArray) ? rawArray : [];
		const normalized = raw
			.map((r) => normalizeRoute(String(r)))
			.filter((r) => !!r);
		return Array.from(new Set(normalized));
	}, [config]);

	const voice = ((config as any)?.voice ?? {}) as Record<string, any>;
	const setVoice = (key: string, value: any) => {
		setValue("voice", { ...voice, [key]: value });
	};
	const voiceMode: string =
		voice.mode ?? ((config as any)?.allow_voice_input ? "record" : "disabled");
	const selectedThemeValue = resolveChatThemePreset(config?.custom_css);
	const selectedTheme = CHAT_THEME_PRESETS.find(
		(theme) => theme.value === selectedThemeValue,
	);
	const placeholderVisual = resolveChatPlaceholderVisual(
		config?.placeholder_visual,
	);
	const placeholderBubbleState = resolveChatPlaceholderBubbleState(
		config?.placeholder_bubble_state,
	);
	const placeholderTypingMotion = resolveChatPlaceholderTypingMotion(
		config?.placeholder_typing_motion,
	);

	const renderVoiceSelect = (
		id: string,
		current: string,
		onChange: (value: string) => void,
		options: { value: string; label: string }[],
	) => (
		<Select value={current} onValueChange={onChange} disabled={!isEditing}>
			<SelectTrigger id={id} className="w-full">
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				{options.map((o) => (
					<SelectItem key={o.value} value={o.value}>
						{o.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);

	// The events surface renders one section at a time; anywhere else (and for
	// any section this component doesn't know) it renders whole.
	const shows = (id: string) => !section || section === id;

	return (
		<div className="w-full space-y-6">
			{shows("appearance") && (
				<>
					<section className="space-y-5 rounded-lg border border-border p-4">
						<div className="space-y-1">
							<h3 className="font-medium">{t("appearance", "Appearance")}</h3>
							<p className="text-sm text-muted-foreground">
								{t(
									"brandTheChatWithoutChangingTheRestOfYourApp",
									"Brand the chat without changing the rest of your app.",
								)}
							</p>
						</div>

						<div className="space-y-2">
							<Label htmlFor="chat_theme_preset">{t("theme", "Theme")}</Label>
							<Select
								disabled={!isEditing}
								value={selectedThemeValue}
								onValueChange={(value) => {
									const preset = CHAT_THEME_PRESETS.find(
										(theme) => theme.value === value,
									);
									if (!preset) return;
									setValue("custom_css", preset.css, ["color_scheme"]);
								}}
							>
								<SelectTrigger id="chat_theme_preset" className="h-11 w-full">
									<SelectValue aria-label={selectedTheme?.label ?? "Custom"}>
										<span className="flex min-w-0 items-center gap-2.5">
											<ThemeSwatch preview={selectedTheme?.preview} />
											<span className="truncate font-medium">
												{selectedTheme?.label ?? "Custom"}
											</span>
										</span>
									</SelectValue>
								</SelectTrigger>
								<SelectContent className="max-w-[min(32rem,calc(100vw-2rem))]">
									{CHAT_THEME_PRESETS.map((theme) => (
										<SelectItem
											key={theme.value}
											value={theme.value}
											className="py-2"
										>
											<span className="flex min-w-0 items-center gap-3">
												<ThemeSwatch preview={theme.preview} />
												<span className="min-w-0">
													<span className="flex items-center gap-2">
														<span className="font-medium">{theme.label}</span>
														<span className="rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
															{theme.badge}
														</span>
													</span>
													<span className="block truncate text-xs text-muted-foreground">
														{theme.description}
													</span>
												</span>
											</span>
										</SelectItem>
									))}
									<SelectItem
										value={CUSTOM_CHAT_THEME_VALUE}
										disabled
										className="py-2"
									>
										<span className="flex min-w-0 items-center gap-3">
											<ThemeSwatch />
											<span className="min-w-0">
												<span className="font-medium">
													{t("custom", "Custom")}
												</span>
												<span className="block truncate text-xs text-muted-foreground">
													{t(
														"shownAutomaticallyAfterYouEditAPreset",
														"Shown automatically after you edit a preset.",
													)}
												</span>
											</span>
										</span>
									</SelectItem>
								</SelectContent>
							</Select>
							<p className="text-sm text-muted-foreground">
								{selectedTheme?.description ??
									t(
										"thisCssNoLongerMatchesAPreset",
										"This CSS no longer matches a preset.",
									)}{" "}
								{t(
									"presetsFollowTheAppModeAndCopyTheirFullSourceIntoTheEditorBelow",
									"Presets follow the app mode and copy their full source into the editor below.",
								)}
							</p>
						</div>

						<div className="space-y-2">
							<Label>{t("backgroundImage", "Background Image")}</Label>
							<AssetPicker
								accept="image"
								appId={appId}
								disabled={!isEditing}
								placeholder={`Select from storage or enter an image URL...`}
								value={config?.background_image ?? ""}
								onChange={(value) => setValue("background_image", value)}
							/>
							<p className="text-sm text-muted-foreground">
								{`Choose an image from this app's storage or paste an external URL. Storage images are securely resolved whenever the chat opens.`}
							</p>
						</div>

						<div className="space-y-2">
							<Label>{t("emptyChatMark", "Empty Chat Mark")}</Label>
							<Select
								disabled={!isEditing}
								value={placeholderVisual}
								onValueChange={(value) => setValue("placeholder_visual", value)}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{CHAT_PLACEHOLDER_VISUALS.map((option) => (
										<SelectItem key={option.value} value={option.value}>
											{option.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<p className="text-sm text-muted-foreground">
								{
									CHAT_PLACEHOLDER_VISUALS.find(
										(option) => option.value === placeholderVisual,
									)?.description
								}
							</p>
						</div>

						{placeholderVisual === "bubble" && (
							<div className="space-y-2">
								<Label>{t("bubbleState", "Bubble State")}</Label>
								<Select
									disabled={!isEditing}
									value={placeholderBubbleState}
									onValueChange={(value) =>
										setValue("placeholder_bubble_state", value)
									}
								>
									<SelectTrigger>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										{CHAT_PLACEHOLDER_BUBBLE_STATES.map((option) => (
											<SelectItem key={option.value} value={option.value}>
												{option.label}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								<p className="text-sm text-muted-foreground">
									{
										CHAT_PLACEHOLDER_BUBBLE_STATES.find(
											(option) => option.value === placeholderBubbleState,
										)?.description
									}{" "}
									{t(
										"theOrbHoldsThisPoseItDoesNotFollowWhatTheAssistantIsActuallyDoing",
										"The orb holds this pose — it does not follow what the assistant is actually doing.",
									)}
								</p>
							</div>
						)}

						{chatPlaceholderSupportsTypingMotion(placeholderVisual) && (
							<div className="space-y-2">
								<div className="flex items-center space-x-2">
									<Switch
										disabled={!isEditing}
										id="placeholder_typing_motion"
										checked={placeholderTypingMotion}
										onCheckedChange={(checked) =>
											setValue("placeholder_typing_motion", checked)
										}
									/>
									<Label htmlFor="placeholder_typing_motion">
										{t("reactWhileTyping", "React While Typing")}
									</Label>
								</div>
								<p className="text-sm text-muted-foreground">
									{t(
										"theMarkAnswersTheComposerItPerksUpAsWritingStartsLeansTowardTheDraftAndStirsInProportionToHowFastTheUserTypesThenSettlesWhenTheyStopOffByDefaultAndAlwaysStillForAnyoneWhoseSystemAsksForReducedMotion",
										"The mark answers the composer: it perks up as writing starts, leans toward the draft and stirs in proportion to how fast the user types, then settles when they stop. Off by default, and always still for anyone whose system asks for reduced motion.",
									)}
								</p>
							</div>
						)}

						{placeholderVisual === "image" && (
							<div className="space-y-2">
								<Label>{t("placeholderImage", "Placeholder Image")}</Label>
								<AssetPicker
									accept="image"
									appId={appId}
									disabled={!isEditing}
									placeholder={`Select from storage or enter an image URL...`}
									value={config?.placeholder_image ?? ""}
									onChange={(value) => setValue("placeholder_image", value)}
								/>
								<p className="text-sm text-muted-foreground">
									{t(
										"shownAsACircleSoASquareImageWorksBestStorageImagesAreSecurelyResolvedWheneverTheChatOpens",
										"Shown as a circle, so a square image works best. Storage images are securely resolved whenever the chat opens.",
									)}
								</p>
							</div>
						)}

						<div className="space-y-2">
							<Label htmlFor="chat_ai_disclosure">
								{t("aiDisclosure", "AI Disclosure")}
							</Label>
							<Textarea
								disabled={!isEditing}
								id="chat_ai_disclosure"
								placeholder={DEFAULT_CHAT_AI_DISCLOSURE}
								value={config?.ai_disclosure ?? ""}
								onChange={(event) =>
									setValue("ai_disclosure", event.target.value)
								}
							/>
							<p className="text-sm text-muted-foreground">
								{t(
									"alwaysShownBelowTheComposerSoPeopleKnowAnAiIsOnTheOtherSideLeavingThisEmptyUsesTheFriendlyDefault",
									"Always shown below the composer so people know an AI is on the other side. Leaving this empty uses the friendly default.",
								)}
							</p>
						</div>

						<div className="space-y-2">
							<Label>{t("customCss", "Custom CSS")}</Label>
							<p className="text-sm text-muted-foreground">
								{t(
									"cssIsSanitizedAndScopedToThisChatUse",
									"CSS is sanitized and scoped to this chat. Use",
								)}{" "}
								<Trans i18nKey="codeClassnameroundedBgmutedPx1Py05rootcodeToOverrideThemeTokensSuchAs">
									<code className="rounded bg-muted px-1 py-0.5">:root</code> to
									override theme tokens such as
								</Trans>{" "}
								<Trans i18nKey="codeClassnameroundedBgmutedPx1Py05primarycode">
									<code className="rounded bg-muted px-1 py-0.5">
										--primary
									</code>
									,
								</Trans>{" "}
								<Trans i18nKey="codeClassnameroundedBgmutedPx1Py05BackgroundCodeAndTheNew">
									<code className="rounded bg-muted px-1 py-0.5">
										--background
									</code>
									, and the new
								</Trans>{" "}
								<code className="rounded bg-muted px-1 py-0.5">
									{t("flchat", "--fl-chat-*")}
								</code>{" "}
								{t(
									"tokensEditingAnyCharacterSwitchesTheThemeToCustom",
									"tokens. Editing any character switches the theme to Custom.",
								)}
							</p>
							<MonacoCodeEditor
								allowFullscreen
								autoFocus={false}
								disabled={!isEditing}
								height="220px"
								language="css"
								value={config?.custom_css ?? ""}
								onChange={(value) =>
									setValue("custom_css", value, ["color_scheme"])
								}
							/>
							<p className="text-xs text-muted-foreground">
								{t(
									"chatTokensFlchatcontentwidthFlchatmessageradiusFlchatsurfacebackgroundFlchatcomposerbackgroundFlchatusermessagebackgroundFlchataimessagebackgroundAndFlchatdisclosurebackgroundImageOverlaysFlchatbackgroundoverlayAndFlchatbackgroundoverlaystrong",
									"Chat tokens: --fl-chat-content-width, --fl-chat-message-radius, --fl-chat-surface-background, --fl-chat-composer-background, --fl-chat-user-message-background, --fl-chat-ai-message-background, and --fl-chat-disclosure-background. Image overlays: --fl-chat-background-overlay and --fl-chat-background-overlay-strong.",
								)}
							</p>
						</div>
					</section>

					<div className="space-y-3">
						<Label>{t("navigateTo", "Navigate To")}</Label>
						{isEditing ? (
							<div className="rounded-md border border-input bg-background">
								<ScrollArea className="max-h-48">
									<div className="p-2 space-y-1">
										{routesQuery.isLoading ? (
											<div className="px-2 py-2 text-sm text-muted-foreground">
												{t("loadingRoutes", "Loading routes…")}
											</div>
										) : routes.length === 0 ? (
											<div className="px-2 py-2 text-sm text-muted-foreground">
												{t("noRoutesConfigured", "No routes configured.")}
											</div>
										) : (
											routes
												.filter(
													(r) =>
														typeof r.path === "string" && r.path.length > 0,
												)
												.map((route) => {
													const checked = selectedRoutes.includes(route.path);
													const label = route.path;
													return (
														<label
															key={route.path}
															className="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-muted cursor-pointer"
														>
															<Checkbox
																checked={checked}
																onCheckedChange={(nextChecked) => {
																	const normalized = normalizeRoute(route.path);
																	const next = new Set(selectedRoutes);
																	if (nextChecked) {
																		next.add(normalized);
																	} else {
																		next.delete(normalized);
																	}
																	const nextArr =
																		Array.from(next).filter(Boolean);
																	setValue(
																		"navigate_to_routes",
																		nextArr.length > 0 ? nextArr : null,
																	);
																}}
															/>
															<span className="text-sm">{label}</span>
														</label>
													);
												})
										)}
									</div>
								</ScrollArea>
								<div className="border-t border-input p-2">
									<button
										type="button"
										className="text-xs text-muted-foreground hover:text-foreground"
										onClick={() => setValue("navigate_to_routes", null)}
									>
										{t("clearSelection", "Clear selection")}
									</button>
								</div>
							</div>
						) : (
							<div className="flex min-h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
								{selectedRoutes.length > 0
									? selectedRoutes.join(", ")
									: t("noDestinations", "No destinations")}
							</div>
						)}
						<p className="text-sm text-muted-foreground">
							{t(
								"optionalRouteDestinationsThisChatCanNavigateTo",
								"Optional route destinations this chat can navigate to.",
							)}
						</p>
					</div>
				</>
			)}

			{shows("capabilities") && (
				<>
					<div className="space-y-4">
						<div className="flex items-center space-x-2">
							<Switch
								disabled={!isEditing}
								id="allow_file_upload"
								checked={config?.allow_file_upload ?? true}
								onCheckedChange={(checked) => {
									setValue("allow_file_upload", checked);
								}}
							/>
							<Label htmlFor="allow_file_upload">
								{t("allowFileUpload", "Allow File Upload")}
							</Label>
						</div>
						<p className="text-sm text-muted-foreground">
							{t(
								"enableUsersToUploadFilesDuringChatConversations",
								"Enable users to upload files during chat conversations",
							)}
						</p>
					</div>

					<div className="space-y-4">
						<div className="flex items-center space-x-2">
							<Switch
								disabled={!isEditing}
								id="attach_widget_snapshots"
								checked={config?.attach_widget_snapshots ?? true}
								onCheckedChange={(checked) => {
									setValue("attach_widget_snapshots", checked);
								}}
							/>
							<Label htmlFor="attach_widget_snapshots">
								{t("widgetSnapshots", "Widget Snapshots")}
							</Label>
						</div>
						<p className="text-sm text-muted-foreground">
							{t(
								"attachImagesOfEmbeddedWidgetsToTheModelapossContextSoVisioncapableModelsCanReactToTheRenderedUi",
								"Attach images of embedded widgets to the model's context so vision-capable models can react to the rendered UI",
							)}
						</p>
					</div>
				</>
			)}

			{shows("voice") && (
				<div className="space-y-4">
					<div className="space-y-3">
						<Label htmlFor="voice_mode">{t("voiceInput", "Voice Input")}</Label>
						{renderVoiceSelect(
							"voice_mode",
							voiceMode,
							(v) => setVoice("mode", v),
							[
								{ value: "disabled", label: t("disabled", "Disabled") },
								{
									value: "record",
									label: t(
										"recordAudioSendRecording",
										"Record audio (send recording)",
									),
								},
								{
									value: "stt",
									label: t(
										"platformSpeechtotextSendText",
										"Platform speech-to-text (send text)",
									),
								},
							],
						)}
						<p className="text-sm text-muted-foreground">
							{t(
								"howUsersSpeakToTheChatSpeechtotextUsesTheBrowserEngineWhenAvailableAndFallsBackToRecording",
								"How users speak to the chat. Speech-to-text uses the browser engine when available and falls back to recording.",
							)}
						</p>
					</div>

					{voiceMode !== "disabled" && (
						<div className="space-y-4">
							<div className="grid gap-4 md:grid-cols-2">
								<div className="space-y-2">
									<Label htmlFor="voice_invoke">
										{t("invokeMode", "Invoke Mode")}
									</Label>
									{renderVoiceSelect(
										"voice_invoke",
										voice.invoke ?? "manual",
										(v) => setVoice("invoke", v),
										[
											{
												value: "manual",
												label: t(
													"manualTapToStartstop",
													"Manual (tap to start/stop)",
												),
											},
											{
												value: "hold",
												label: t("holdToRecord", "Hold to record"),
											},
											{
												value: "auto",
												label: t(
													"automaticPauseDetection",
													"Automatic (pause detection)",
												),
											},
										],
									)}
								</div>
								<div className="space-y-2">
									<Label htmlFor="voice_playback">
										{t("answerPlayback", "Answer Playback")}
									</Label>
									{renderVoiceSelect(
										"voice_playback",
										voice.playback ?? "text",
										(v) => setVoice("playback", v),
										[
											{ value: "text", label: t("textOnly", "Text only") },
											{ value: "audio", label: t("audioOnly", "Audio only") },
											{ value: "both", label: t("textAudio", "Text + audio") },
										],
									)}
								</div>
								<div className="space-y-2">
									<Label htmlFor="voice_variant">
										{t("visualStyle", "Visual Style")}
									</Label>
									{renderVoiceSelect(
										"voice_variant",
										voice.variant ?? "conservative",
										(v) => setVoice("variant", v),
										[
											{
												value: "conservative",
												label: t(
													"conservativeMicIcon",
													"Conservative (mic icon)",
												),
											},
											{ value: "waveform", label: t("waveform", "Waveform") },
											{ value: "orb", label: t("orb", "Orb") },
											{ value: "vortex", label: t("vortex", "Vortex") },
											{ value: "shader", label: t("shader", "Shader") },
										],
									)}
								</div>
								<div className="space-y-2">
									<Label htmlFor="voice_size">{t("size", "Size")}</Label>
									{renderVoiceSelect(
										"voice_size",
										voice.size ?? "md",
										(v) => setVoice("size", v),
										[
											{ value: "sm", label: t("small", "Small") },
											{ value: "md", label: t("medium", "Medium") },
											{ value: "lg", label: t("large", "Large") },
										],
									)}
								</div>
							</div>

							<div className="grid gap-4 md:grid-cols-2">
								<div className="space-y-2">
									<Label htmlFor="voice_color">
										{t("accentColor", "Accent Color")}
									</Label>
									<Input
										id="voice_color"
										type="color"
										disabled={!isEditing}
										value={voice.color ?? "#8b5cf6"}
										onChange={(e) => setVoice("color", e.target.value)}
									/>
								</div>
								<div className="space-y-2">
									<Label htmlFor="voice_recording_color">
										{t("recordingColor", "Recording Color")}
									</Label>
									<Input
										id="voice_recording_color"
										type="color"
										disabled={!isEditing}
										value={voice.recording_color ?? "#ef4444"}
										onChange={(e) =>
											setVoice("recording_color", e.target.value)
										}
									/>
								</div>
							</div>

							<div className="space-y-2">
								<Label htmlFor="voice_max_duration">
									{t("maxDurationSeconds", "Max Duration (seconds)")}
								</Label>
								<Input
									id="voice_max_duration"
									type="number"
									min={0}
									disabled={!isEditing}
									value={voice.max_duration ?? 300}
									onChange={(e) =>
										setVoice(
											"max_duration",
											e.target.value ? Number.parseInt(e.target.value, 10) : 0,
										)
									}
								/>
							</div>

							<div className="flex items-center space-x-2">
								<Switch
									disabled={!isEditing}
									id="voice_auto_stop"
									checked={voice.auto_stop ?? false}
									onCheckedChange={(checked) => setVoice("auto_stop", checked)}
								/>
								<Label htmlFor="voice_auto_stop">
									{t("autostopOnSilence", "Auto-stop on silence")}
								</Label>
							</div>
						</div>
					)}
				</div>
			)}

			{shows("capabilities") && (
				<div className="space-y-3">
					<Label htmlFor="history_elements">
						{t("historyElements", "History Elements")}
					</Label>
					{isEditing ? (
						<Input
							value={config?.history_elements ?? 5}
							onChange={(e) => {
								const value = e.target.value
									? Number.parseInt(e.target.value, 10)
									: 5;
								setValue("history_elements", value);
							}}
							type="number"
							id="history_elements"
							placeholder="5"
							min="1"
							max="100"
						/>
					) : (
						<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
							{config?.history_elements ?? 5}
						</div>
					)}
					<p className="text-sm text-muted-foreground">
						{t(
							"numberOfPreviousMessagesToIncludeInChatContext",
							"Number of previous messages to include in chat context",
						)}
					</p>
				</div>
			)}

			{shows("tools") && (
				<>
					<div className="space-y-3">
						<Label htmlFor="tools">
							{t("availableTools", "Available Tools")}
						</Label>
						{isEditing ? (
							<div className="space-y-2">
								<div className="flex flex-wrap gap-2">
									{(config?.tools ?? []).map((tool, index) => (
										<div
											key={index + tool}
											className="inline-flex items-center gap-1 bg-secondary text-secondary-foreground px-2 py-1 rounded-md text-sm"
										>
											<span>{tool}</span>
											<button
												type="button"
												onClick={() => {
													const newTools = [...(config?.tools ?? [])];
													newTools.splice(index, 1);
													setValue("tools", newTools);
												}}
												className="text-secondary-foreground/70 hover:text-secondary-foreground"
											>
												×
											</button>
										</div>
									))}
								</div>
								<Input
									placeholder={t(
										"typeAToolNameAndPressEnter",
										"Type a tool name and press Enter",
									)}
									onKeyDown={(e) => {
										if (e.key === "Enter" && e.currentTarget.value.trim()) {
											e.preventDefault();
											const newTool = e.currentTarget.value.trim();
											const currentTools = config?.tools ?? [];
											if (!currentTools.includes(newTool)) {
												setValue("tools", [...currentTools, newTool]);
											}
											e.currentTarget.value = "";
										}
									}}
								/>
							</div>
						) : (
							<div className="space-y-2">
								{(config?.tools ?? []).length > 0 ? (
									<div className="flex flex-wrap gap-2">
										{(config?.tools ?? []).map((tool, index) => (
											<div
												key={index + tool}
												className="inline-flex items-center bg-muted text-muted-foreground px-2 py-1 rounded-md text-sm"
											>
												{tool}
											</div>
										))}
									</div>
								) : (
									<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
										{t("noToolsConfigured", "No tools configured")}
									</div>
								)}
							</div>
						)}
						<p className="text-sm text-muted-foreground">
							{`Tools available for this chat. Press Enter to add a new tool.`}
						</p>
					</div>

					<div className="space-y-3">
						<Label htmlFor="default_tools">
							{t("defaultTools", "Default Tools")}
						</Label>
						{isEditing ? (
							<div className="space-y-2">
								<div className="flex flex-wrap gap-2">
									{(config?.default_tools ?? []).map((tool, index) => (
										<div
											key={index + tool}
											className="inline-flex items-center gap-1 bg-primary text-primary-foreground px-2 py-1 rounded-md text-sm"
										>
											<span>{tool}</span>
											<button
												type="button"
												onClick={() => {
													const newTools = [...(config?.default_tools ?? [])];
													newTools.splice(index, 1);
													setValue("default_tools", newTools);
												}}
												className="text-primary-foreground/70 hover:text-primary-foreground"
											>
												×
											</button>
										</div>
									))}
								</div>
								<Input
									placeholder={t(
										"typeAToolNameAndPressEnter",
										"Type a tool name and press Enter",
									)}
									onKeyDown={(e) => {
										if (e.key === "Enter" && e.currentTarget.value.trim()) {
											e.preventDefault();
											const newTool = e.currentTarget.value.trim();
											const currentTools = config?.default_tools ?? [];
											if (!currentTools.includes(newTool)) {
												setValue("default_tools", [...currentTools, newTool]);
											}
											e.currentTarget.value = "";
										}
									}}
								/>
							</div>
						) : (
							<div className="space-y-2">
								{(config?.default_tools ?? []).length > 0 ? (
									<div className="flex flex-wrap gap-2">
										{(config?.default_tools ?? []).map((tool, index) => (
											<div
												key={index + tool}
												className="inline-flex items-center bg-muted text-muted-foreground px-2 py-1 rounded-md text-sm"
											>
												{tool}
											</div>
										))}
									</div>
								) : (
									<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
										{t("noDefaultTools", "No default tools")}
									</div>
								)}
							</div>
						)}
						<p className="text-sm text-muted-foreground">
							{`Tools enabled by default. Press Enter to add a new tool.`}
						</p>
					</div>

					<div className="space-y-3">
						<Label htmlFor="example_messages">
							{t("exampleMessages", "Example Messages")}
						</Label>
						{isEditing ? (
							<div className="space-y-2">
								<div className="flex flex-wrap gap-2">
									{(config?.example_messages ?? []).map((message, index) => (
										<div
											key={index + message}
											className="inline-flex items-center gap-1 bg-secondary text-secondary-foreground px-2 py-1 rounded-md text-sm max-w-xs"
										>
											<span className="truncate">{message}</span>
											<button
												type="button"
												onClick={() => {
													const newMessages = [
														...(config?.example_messages ?? []),
													];
													newMessages.splice(index, 1);
													setValue("example_messages", newMessages);
												}}
												className="text-secondary-foreground/70 hover:text-secondary-foreground shrink-0"
											>
												×
											</button>
										</div>
									))}
								</div>
								<Input
									placeholder={t(
										"typeAnExampleMessageAndPressEnter",
										"Type an example message and press Enter",
									)}
									onKeyDown={(e) => {
										if (e.key === "Enter" && e.currentTarget.value.trim()) {
											e.preventDefault();
											const newMessage = e.currentTarget.value.trim();
											const currentMessages = config?.example_messages ?? [];
											if (!currentMessages.includes(newMessage)) {
												setValue("example_messages", [
													...currentMessages,
													newMessage,
												]);
											}
											e.currentTarget.value = "";
										}
									}}
								/>
							</div>
						) : (
							<div className="space-y-2">
								{(config?.example_messages ?? []).length > 0 ? (
									<div className="flex flex-wrap gap-2">
										{(config?.example_messages ?? []).map((message, index) => (
											<div
												key={index + message}
												className="inline-flex items-center bg-muted text-muted-foreground px-2 py-1 rounded-md text-sm max-w-xs"
											>
												<span className="truncate">{message}</span>
											</div>
										))}
									</div>
								) : null}
							</div>
						)}

						{/* An empty list is not "nothing" — the chat falls back to a
						    built-in set, so show it here or nobody can find where the
						    prompts they can see are configured. */}
						{(config?.example_messages ?? []).length === 0 && (
							<div className="space-y-2 rounded-md border border-dashed border-input p-3">
								<div className="flex flex-wrap items-center gap-2">
									<span className="text-sm font-medium">
										{t(
											"currentlyShowingTheBuiltinExamples",
											"Currently showing the built-in examples",
										)}
									</span>
									{isEditing && (
										<button
											type="button"
											className="text-sm text-primary underline underline-offset-2"
											onClick={() =>
												setValue("example_messages", [
													...DEFAULT_CHAT_EXAMPLE_MESSAGES,
												])
											}
										>
											{`Start from these`}
										</button>
									)}
								</div>
								<div className="flex flex-wrap gap-2">
									{DEFAULT_CHAT_EXAMPLE_MESSAGES.slice(0, 4).map((message) => (
										<div
											key={message}
											className="inline-flex max-w-xs items-center rounded-md border border-dashed border-input px-2 py-1 text-sm text-muted-foreground"
										>
											<span className="truncate">{message}</span>
										</div>
									))}
								</div>
								<p className="text-xs text-muted-foreground">
									{t(
										"theseAreGenericAddingYourOwnReplacesThemEntirely",
										"These are generic. Adding your own replaces them entirely.",
									)}
								</p>
							</div>
						)}

						<p className="text-sm text-muted-foreground">
							{`Example messages to show users. Press Enter to add a new message.`}
						</p>
					</div>
				</>
			)}
		</div>
	);
}
