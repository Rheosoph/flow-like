"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo, useRef, useState } from "react";
import type { IEvent, IEventPayloadChat } from "../../../lib";
import { DEFAULT_CHAT_EXAMPLE_MESSAGES } from "../../../lib/chat-appearance";
import { createComposerActivity } from "../../../lib/composer-activity";
import { VoiceMode } from "./VoiceMode";
import { ChatAiDisclosure } from "./ai-disclosure";
import { ChatPlaceholder } from "./chat-placeholder";
import { ChatBox, type ChatBoxRef, type ISendMessageFunction } from "./chatbox";
import { isVoiceEnabled, resolveChatVoiceConfig } from "./voice-config";

interface ChatWelcomeProps {
	onSendMessage: ISendMessageFunction;
	event: IEvent;
	config?: Partial<IEventPayloadChat>;
	isSending?: boolean;
	/** Needed to resolve a storage-backed placeholder image. */
	appId?: string;
}

const defaultExamples: readonly string[] = DEFAULT_CHAT_EXAMPLE_MESSAGES;

export function ChatWelcome({
	onSendMessage,
	event,
	config = {},
	isSending = false,
	appId,
}: Readonly<ChatWelcomeProps>) {
	const { t } = useTranslation("chat");
	const [currentMessage, setCurrentMessage] = useState("");
	const [voiceModeOpen, setVoiceModeOpen] = useState(false);
	const chatBox = useRef<ChatBoxRef>(null);
	// One channel per welcome screen — the mark must answer this composer, not another chat's.
	const activity = useRef(createComposerActivity()).current;
	const description = event.description?.trim();
	const voiceConfig = useMemo(() => resolveChatVoiceConfig(config), [config]);
	const voiceEnabled = isVoiceEnabled(voiceConfig);

	const handleVoiceModeSend = useCallback(
		(content: string, audioFile?: File) => {
			setVoiceModeOpen(false);
			void onSendMessage(content, undefined, undefined, audioFile);
		},
		[onSendMessage],
	);

	// Fuzzy search function
	const fuzzyScore = useCallback((text: string, searchTerm: string): number => {
		const textLower = text.toLowerCase();
		const searchLower = searchTerm.toLowerCase();

		if (!searchLower) return 0;
		if (textLower.includes(searchLower)) return 100; // Exact substring match gets highest score

		let score = 0;
		let searchIndex = 0;
		let lastMatchIndex = -1;

		for (
			let i = 0;
			i < textLower.length && searchIndex < searchLower.length;
			i++
		) {
			if (textLower[i] === searchLower[searchIndex]) {
				// Award points for character match
				score += 10;

				// Bonus for consecutive matches
				if (lastMatchIndex === i - 1) {
					score += 5;
				}

				// Bonus for matches at word boundaries
				if (i === 0 || textLower[i - 1] === " ") {
					score += 3;
				}

				lastMatchIndex = i;
				searchIndex++;
			}
		}

		// Only return score if all search characters were found
		if (searchIndex === searchLower.length) {
			// Bonus for shorter text (more relevant matches)
			score += Math.max(0, 50 - textLower.length);
			return score;
		}

		return 0;
	}, []);

	// Filter examples based on current message and show max 5
	const filteredExamples = useMemo(() => {
		const examples =
			(config?.example_messages?.length ?? 0) === 0
				? defaultExamples
				: (config?.example_messages ?? []);
		if (!currentMessage.trim()) {
			return examples.slice(0, 4);
		}

		const searchTerm = currentMessage.toLowerCase();

		// Score all examples and sort by relevance
		const scoredExamples = examples
			.map((example) => ({
				text: example,
				score: fuzzyScore(example, searchTerm),
			}))
			.filter((item) => item.score > 0)
			.sort((a, b) => b.score - a.score)
			.map((item) => item.text);

		return scoredExamples.slice(0, 5);
	}, [currentMessage, config?.example_messages, fuzzyScore]);

	// Function to highlight matching text with fuzzy highlighting
	const highlightMatch = (text: string, searchTerm: string) => {
		if (!searchTerm.trim()) return text;

		const textLower = text.toLowerCase();
		const searchLower = searchTerm.toLowerCase();

		// For exact substring matches, use the original highlighting
		if (searchLower && textLower.includes(searchLower)) {
			const result: React.ReactNode[] = [];
			let cursor = 0;
			let matchStart = textLower.indexOf(searchLower, cursor);

			while (matchStart !== -1) {
				if (matchStart > cursor) {
					result.push(text.slice(cursor, matchStart));
				}
				const matchEnd = matchStart + searchLower.length;
				const match = text.slice(matchStart, matchEnd);
				result.push(
					<span
						key={`${matchStart}-${match}`}
						className="bg-primary/20 text-primary rounded-sm"
					>
						{match}
					</span>,
				);
				cursor = matchEnd;
				matchStart = textLower.indexOf(searchLower, cursor);
			}

			if (cursor < text.length) {
				result.push(text.slice(cursor));
			}

			return result;
		}

		// For fuzzy matches, highlight individual matching characters
		const result: React.ReactNode[] = [];
		let searchIndex = 0;

		for (let i = 0; i < text.length && searchIndex < searchLower.length; i++) {
			const char = text[i];
			const isMatch = textLower[i] === searchLower[searchIndex];

			if (isMatch) {
				result.push(
					<span
						key={i}
						className="bg-primary/20 text-primary rounded-sm px-0.5"
					>
						{char}
					</span>,
				);
				searchIndex++;
			} else {
				result.push(char);
			}
		}

		// Add remaining characters
		if (result.length < text.length) {
			result.push(text.slice(result.length));
		}

		return result;
	};

	return (
		<div
			className="fl-chat-surface relative flex h-full grow flex-col bg-transparent"
			data-fl-chat-surface
			data-fl-chat-welcome
			style={{
				backgroundColor: "var(--fl-chat-surface-background, var(--background))",
			}}
		>
			{voiceModeOpen && (
				<VoiceMode
					open={voiceModeOpen}
					onClose={() => setVoiceModeOpen(false)}
					onSend={handleVoiceModeSend}
					voice={voiceConfig}
				/>
			)}
			{/* Loading Overlay */}
			{isSending && (
				<div className="absolute inset-0 bg-linear-to-br from-background via-background/95 to-background/90 backdrop-blur-md z-50 flex items-center justify-center overflow-hidden">
					{/* Animated background elements */}
					<div className="absolute inset-0 overflow-hidden">
						<div className="absolute top-1/4 left-1/4 w-96 h-96 bg-primary/5 rounded-full blur-3xl animate-pulse" />
						<div className="absolute bottom-1/4 right-1/4 w-80 h-80 bg-blue-500/5 rounded-full blur-3xl animate-pulse delay-150" />
						<div className="absolute top-1/3 right-1/3 w-72 h-72 bg-purple-500/5 rounded-full blur-3xl animate-pulse delay-300" />
					</div>

					{/* Main loading content */}
					<div className="relative z-10 flex flex-col items-center gap-8 px-8 max-w-md">
						{/* Spinning rings with gradient */}
						<div className="relative w-32 h-32">
							{/* Outer ring */}
							<div className="absolute inset-0 rounded-full border-4 border-transparent border-t-primary border-r-primary/50 animate-spin" />
							{/* Middle ring */}
							<div className="absolute inset-3 rounded-full border-4 border-transparent border-b-blue-500 border-l-blue-500/50 animate-spin-slow" />
							{/* Inner ring */}
							<div className="absolute inset-6 rounded-full border-4 border-transparent border-t-purple-500 border-r-purple-500/50 animate-spin-reverse" />
							{/* Center glow */}
							<div className="absolute inset-0 flex items-center justify-center">
								<div className="w-12 h-12 bg-linear-to-br from-primary via-blue-500 to-purple-500 rounded-full animate-pulse blur-sm" />
								<div className="absolute w-8 h-8 bg-linear-to-br from-primary via-blue-500 to-purple-500 rounded-full animate-pulse" />
							</div>
						</div>

						{/* Text content with animations */}
						<div className="text-center space-y-4">
							<h3 className="text-2xl font-bold bg-linear-to-r from-primary via-blue-500 to-purple-500 bg-clip-text text-transparent animate-pulse">
								{t("processingYourMessage", "Processing Your Message")}
							</h3>
							<div className="space-y-2">
								<div className="flex items-center justify-center gap-2">
									<div className="w-2 h-2 bg-primary rounded-full animate-bounce" />
									<div className="w-2 h-2 bg-blue-500 rounded-full animate-bounce delay-75" />
									<div className="w-2 h-2 bg-purple-500 rounded-full animate-bounce delay-150" />
								</div>
								<p className="text-sm text-muted-foreground animate-pulse">
									{t(
										"uploadingFilesAndPreparingAttachments",
										"Uploading files and preparing attachments",
									)}
								</p>
							</div>
						</div>

						{/* Progress bar */}
						<div className="w-full h-1 bg-muted/30 rounded-full overflow-hidden">
							<div className="h-full bg-linear-to-r from-primary via-blue-500 to-purple-500 animate-progress rounded-full" />
						</div>
					</div>
				</div>
			)}
			{/* Welcome Content */}
			<div className="flex min-h-0 flex-1 justify-center overflow-y-auto p-3 sm:p-6 lg:p-8">
				<div
					className="my-auto w-full space-y-6"
					data-fl-chat-welcome-panel
					style={{
						maxWidth: "min(var(--fl-chat-content-width, 64rem), 40rem)",
					}}
				>
					{/* The mark carries the invitation; the app's own name stays as the
					    single line of context a user needs to know where they are. */}
					<div
						className="flex flex-col items-center gap-3"
						data-fl-chat-welcome-header
					>
						<ChatPlaceholder
							config={config}
							appId={appId}
							size={168}
							activity={activity}
						/>
						<div className="space-y-1 text-center">
							<h1 className="text-base font-semibold tracking-tight">
								{event.name}
							</h1>
							{description && (
								<p className="line-clamp-2 text-sm text-muted-foreground">
									{description}
								</p>
							)}
						</div>
					</div>

					<div className="mx-auto max-w-2xl space-y-3">
						<ChatBox
							ref={chatBox}
							availableTools={config?.tools ?? []}
							defaultActiveTools={config?.default_tools ?? []}
							onSendMessage={onSendMessage}
							onContentChange={(content) => {
								setCurrentMessage(content);
								activity.report(content);
							}}
							fileUpload={config?.allow_file_upload ?? false}
							audioInput={voiceEnabled}
							voiceMode={voiceConfig.mode === "stt" ? "stt" : "record"}
							voiceInvoke={voiceConfig.invoke}
							voiceMaxDuration={voiceConfig.maxDuration}
							onVoiceModeToggle={
								voiceEnabled && voiceConfig.invoke === "auto"
									? () => setVoiceModeOpen(true)
									: undefined
							}
						/>

						{/* Example Prompts List */}
						{(filteredExamples.length > 0 || currentMessage.trim()) && (
							<div className="space-y-2" data-fl-chat-suggestions>
								{filteredExamples.length > 0 && (
									<p className="text-xs text-muted-foreground uppercase tracking-wide">
										{t("suggestions", "Suggestions")}
									</p>
								)}
								<div className="grid max-h-[min(15rem,30dvh)] grid-cols-1 gap-1.5 overflow-y-auto sm:grid-cols-2">
									{filteredExamples.map((example) => (
										<button
											key={example}
											className="min-h-11 w-full cursor-pointer rounded-lg border border-transparent bg-muted/10 px-3 py-2 text-left text-sm text-muted-foreground transition-all hover:bg-muted/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
											data-fl-chat-suggestion
											onClick={() => {
												chatBox.current?.setInput(example);
												chatBox.current?.focusInput?.();
											}}
											type="button"
										>
											{highlightMatch(example, currentMessage.trim())}
										</button>
									))}
								</div>
							</div>
						)}
					</div>
				</div>
			</div>
			<div
				className="mx-auto w-full shrink-0 px-3"
				style={{
					maxWidth: "var(--fl-chat-content-width, 64rem)",
					paddingBottom:
						"calc(var(--fl-chat-pad-bottom, 0.75rem) + var(--fl-safe-bottom, env(safe-area-inset-bottom, 0px)))",
				}}
			>
				<ChatAiDisclosure text={config.ai_disclosure} />
			</div>
		</div>
	);
}
