"use client";

import { AnimatePresence, motion } from "framer-motion";
import {
	ArrowDown,
	CameraIcon,
	CheckCircle2,
	ChevronDownIcon,
	ClockIcon,
	ImageIcon,
	LayoutGridIcon,
	Loader2,
	SendIcon,
	SparklesIcon,
	SquarePenIcon,
	XIcon,
} from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCopilotSDK, useInvoke } from "../../hooks";
import { IBitTypes } from "../../lib";
import {
	type IFlowPilotConversation,
	addMessage,
	createConversation,
	getMessages,
	updateConversation,
	updateMessage,
} from "../../lib/flowpilot-db";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";

import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { ScrollArea } from "../ui/scroll-area";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

import { ContextNodes } from "./ContextNodes";
import { HistoryPanel } from "./HistoryPanel";
import { MessageContent } from "./MessageContent";
import { PendingCommandsView } from "./PendingCommandsView";
import { PendingComponentsView } from "./PendingComponentsView";
import { PlanStepsView } from "./PlanStepsView";
import { ModelSelector, ProviderSelector } from "./ProviderSelector";
import { StatusPill } from "./StatusPill";
import { buildBudgetedHistory } from "./history-budget";
import type {
	AIProvider,
	AgentMode,
	AttachedImage,
	CopilotMessage,
	FlowPilotProps,
	LoadingPhase,
	UnifiedPlanStep,
} from "./types";
import {
	validateCanvasSettings,
	validateComponents,
} from "./validateComponents";

import type {
	CanvasSettings,
	CopilotScope,
	UnifiedChatMessage,
} from "../../lib/schema/copilot";
import type { BoardCommand, Suggestion } from "../../lib/schema/flow/copilot";
import type { SurfaceComponent } from "../a2ui/types";

const MAX_ATTACHED_IMAGES = 4;
const MAX_ATTACHED_IMAGE_BYTES = 5 * 1024 * 1024;
const ALLOWED_ATTACHED_IMAGE_TYPES = new Set([
	"image/png",
	"image/jpeg",
	"image/webp",
	"image/gif",
]);

function canAttachImage(file: File): boolean {
	return (
		ALLOWED_ATTACHED_IMAGE_TYPES.has(file.type) &&
		file.size <= MAX_ATTACHED_IMAGE_BYTES
	);
}

function normalizedDataUrlImageType(mediaType: string): string | null {
	if (mediaType === "image/jpg") return "image/jpeg";
	return ALLOWED_ATTACHED_IMAGE_TYPES.has(mediaType) ? mediaType : null;
}

function base64ByteLength(data: string): number {
	const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
	return Math.max(0, Math.floor((data.length * 3) / 4) - padding);
}

export function FlowPilot({
	agentMode,
	title = "FlowPilot",
	className,
	onClose,
	// Provider props
	forceProvider,
	defaultProvider = "bits",
	// Board mode props
	board,
	selectedNodeIds = [],
	onAcceptSuggestion,
	onExecuteCommands,
	onFocusNode,
	onSelectNodes,
	runContext,
	initialPrompt,
	// UI mode props
	currentComponents = [],
	selectedComponentIds = [],
	onComponentsGenerated,
	onApplyComponents,
	// Screenshot prop
	captureScreenshot,
}: FlowPilotProps) {
	// Core state
	const [messages, setMessages] = useState<CopilotMessage[]>([]);
	const [input, setInput] = useState("");
	const [loading, setLoading] = useState(false);
	const [loadingPhase, setLoadingPhase] = useState<LoadingPhase>("idle");
	const [loadingStartTime, setLoadingStartTime] = useState<number | null>(null);
	const [elapsedSeconds, setElapsedSeconds] = useState(0);
	const [tokenCount, setTokenCount] = useState(0);
	const [planSteps, setPlanSteps] = useState<UnifiedPlanStep[]>([]);
	const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
	const [userScrolledUp, setUserScrolledUp] = useState(false);
	const [selectedModelId, setSelectedModelId] = useState("");

	// Provider state
	const [provider, setProvider] = useState<AIProvider>(
		forceProvider ?? defaultProvider,
	);

	// Board-specific state
	const [pendingCommands, setPendingCommands] = useState<BoardCommand[]>([]);
	const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
	const [currentToolCall, setCurrentToolCall] = useState<string | null>(null);

	// UI-specific state
	const [pendingComponents, setPendingComponents] = useState<
		SurfaceComponent[]
	>([]);
	const [pendingCanvasSettings, setPendingCanvasSettings] = useState<
		CanvasSettings | undefined
	>();
	const [validationWarnings, setValidationWarnings] = useState<string[]>([]);

	// History state
	const [showHistory, setShowHistory] = useState(false);
	const [currentConversationId, setCurrentConversationId] = useState<
		string | undefined
	>();
	const currentMessageIdRef = useRef<string | undefined>(undefined);

	// Refs
	const messagesEndRef = useRef<HTMLDivElement>(null);
	const scrollContainerRef = useRef<HTMLDivElement>(null);
	const imageInputRef = useRef<HTMLInputElement>(null);
	const initialPromptHandledRef = useRef(false);
	const handleSubmitRef = useRef<(() => void) | null>(null);

	// Backend context
	const backendContext = useBackend();

	// Copilot SDK hook
	const copilotSDK = useCopilotSDK();

	// Elapsed time tracking
	useEffect(() => {
		if (!loading || !loadingStartTime) {
			setElapsedSeconds(0);
			return;
		}
		const interval = setInterval(() => {
			setElapsedSeconds(Math.floor((Date.now() - loadingStartTime) / 1000));
		}, 1000);
		return () => clearInterval(interval);
	}, [loading, loadingStartTime]);

	// Fetch user profile
	const profile = useInvoke(
		backendContext.userState.getSettingsProfile,
		backendContext.userState,
		[],
		true,
	);

	// Fetch available models (bits)
	const foundBits = useInvoke(
		backendContext.bitState.searchBits,
		backendContext.bitState,
		[{ bit_types: [IBitTypes.Llm, IBitTypes.Vlm] }],
		!!profile.data,
		[profile.data?.hub_profile.id],
	);

	// Filter bits models to only include those in the user's profile
	const bitsModels = useMemo(() => {
		if (!foundBits.data || !profile.data?.hub_profile.bits) return [];
		const profileBitIds = new Set(profile.data.hub_profile.bits);
		const canHostLocal = backendContext.capabilities().canHostLlamaCPP;

		return foundBits.data.filter((model) => {
			const fullId = `${model.hub}:${model.id}`;
			if (!profileBitIds.has(fullId)) return false;

			if (!canHostLocal) {
				const providerName =
					model.parameters?.provider?.provider_name?.toLowerCase();
				if (
					providerName === "local" ||
					providerName === "llama.cpp" ||
					providerName === "llamacpp" ||
					providerName === "ollama"
				) {
					return false;
				}
			}

			return true;
		});
	}, [foundBits.data, profile.data?.hub_profile.bits]);

	// Get current models based on provider
	const currentModels = useMemo(() => {
		if (provider === "copilot") {
			return copilotSDK.models;
		}
		return bitsModels;
	}, [provider, copilotSDK.models, bitsModels]);

	// Set default model when models are loaded or provider changes
	useEffect(() => {
		if (currentModels.length === 0) return;

		// Check if current selection is valid for current provider
		const isCurrentValid = currentModels.some((m) => m.id === selectedModelId);
		if (isCurrentValid) return;

		// Select a default model
		if (provider === "copilot") {
			// Prefer claude or gpt-4 for Copilot
			const preferredModel =
				currentModels.find((m) => m.id.includes("claude")) ||
				currentModels.find((m) => m.id.includes("gpt-4")) ||
				currentModels[0];
			setSelectedModelId(preferredModel?.id || "");
		} else {
			// Bits provider - existing logic
			const hostedModel = bitsModels.find(
				(m) => m.parameters?.provider?.provider_name === "Hosted",
			);
			const gpt4o = bitsModels.find((m) => m.id.includes("gpt-4o"));
			const defaultModel = hostedModel || gpt4o || bitsModels[0];
			setSelectedModelId(defaultModel?.id || "");
		}
	}, [currentModels, selectedModelId, provider, bitsModels]);

	// Copilot connection handlers
	const handleStartCopilot = useCallback(
		async (serverUrl?: string) => {
			await copilotSDK.start({
				useStdio: !serverUrl,
				serverUrl,
			});
		},
		[copilotSDK],
	);

	const handleStopCopilot = useCallback(async () => {
		await copilotSDK.stop();
		setProvider("bits");
	}, [copilotSDK]);

	// Scroll handling
	const scrollToBottom = useCallback(
		(force = false) => {
			if (force || !userScrolledUp) {
				messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
			}
		},
		[userScrolledUp],
	);

	const handleScroll = useCallback(() => {
		const container = scrollContainerRef.current;
		if (!container) return;
		const { scrollTop, scrollHeight, clientHeight } = container;
		const isAtBottom = scrollHeight - scrollTop - clientHeight < 100;
		setUserScrolledUp(!isAtBottom);
	}, []);

	useEffect(() => {
		if (!userScrolledUp) {
			scrollToBottom();
		}
	}, [messages, userScrolledUp, scrollToBottom]);

	// New chat handler
	const handleNewChat = useCallback(() => {
		setMessages([]);
		setPlanSteps([]);
		setInput("");
		setAttachedImages([]);
		setPendingCommands([]);
		setPendingComponents([]);
		setValidationWarnings([]);
		setSuggestions([]);
		setCurrentConversationId(undefined);
		currentMessageIdRef.current = undefined;
		setShowHistory(false);
	}, []);

	// Select conversation from history
	const handleSelectConversation = useCallback(
		async (conversation: IFlowPilotConversation) => {
			try {
				const storedMessages = await getMessages(conversation.id);
				const loadedMessages: CopilotMessage[] = storedMessages.map((m) => ({
					role: m.role as "user" | "assistant",
					content: m.content,
					images: m.images?.map((image) => ({
						data: image.data,
						mediaType: image.mediaType,
						preview: `data:${image.mediaType};base64,${image.data}`,
					})),
					contextNodeIds: m.contextNodeIds,
					appliedComponents: m.appliedComponents,
					executedCommands: m.executedCommands,
				}));
				setMessages(loadedMessages);
				setCurrentConversationId(conversation.id);
				setPlanSteps([]);
				setPendingCommands([]);
				setPendingComponents([]);
				setValidationWarnings([]);
				currentMessageIdRef.current = undefined;
				setShowHistory(false);
			} catch (err) {
				console.error("Failed to load conversation:", err);
			}
		},
		[],
	);

	// Image handling
	const handleImageSelect = useCallback(
		(e: React.ChangeEvent<HTMLInputElement>) => {
			const files = e.target.files;
			if (!files) return;

			Array.from(files).forEach((file) => {
				if (!canAttachImage(file)) {
					console.warn(
						`Skipped unsupported or oversized FlowPilot image: ${file.name}`,
					);
					return;
				}

				const reader = new FileReader();
				reader.onload = (event) => {
					const dataUrl = event.target?.result as string;
					if (!dataUrl) return;

					const base64Data = dataUrl.split(",")[1];
					if (
						!base64Data ||
						base64ByteLength(base64Data) > MAX_ATTACHED_IMAGE_BYTES
					) {
						return;
					}
					setAttachedImages((prev) => {
						if (prev.length >= MAX_ATTACHED_IMAGES) return prev;
						return [
							...prev,
							{
								data: base64Data,
								mediaType: file.type,
								preview: dataUrl,
							},
						];
					});
				};
				reader.readAsDataURL(file);
			});

			if (imageInputRef.current) {
				imageInputRef.current.value = "";
			}
		},
		[],
	);

	const handleRemoveImage = useCallback((index: number) => {
		setAttachedImages((prev) => prev.filter((_, i) => i !== index));
	}, []);

	const handlePaste = useCallback((e: React.ClipboardEvent) => {
		const items = e.clipboardData?.items;
		if (!items) return;

		for (const item of Array.from(items)) {
			if (item.type.startsWith("image/")) {
				e.preventDefault();
				const file = item.getAsFile();
				if (!file) continue;
				if (!canAttachImage(file)) {
					console.warn(
						"Skipped unsupported or oversized pasted FlowPilot image",
					);
					continue;
				}

				const reader = new FileReader();
				reader.onload = (event) => {
					const dataUrl = event.target?.result as string;
					if (!dataUrl) return;

					const base64Data = dataUrl.split(",")[1];
					if (
						!base64Data ||
						base64ByteLength(base64Data) > MAX_ATTACHED_IMAGE_BYTES
					) {
						return;
					}
					setAttachedImages((prev) => {
						if (prev.length >= MAX_ATTACHED_IMAGES) return prev;
						return [
							...prev,
							{
								data: base64Data,
								mediaType: file.type,
								preview: dataUrl,
							},
						];
					});
				};
				reader.readAsDataURL(file);
			}
		}
	}, []);

	// Board mode handlers
	const handleExecuteCommands = useCallback(() => {
		if (onExecuteCommands && pendingCommands.length > 0) {
			const lastAssistantMessage = [...messages]
				.reverse()
				.find((message) => message.role === "assistant");
			const nextExecutedCommands = [
				...(lastAssistantMessage?.executedCommands ?? []),
				...pendingCommands,
			];
			onExecuteCommands(pendingCommands);
			setMessages((prev) => {
				const newMessages = [...prev];
				for (let i = newMessages.length - 1; i >= 0; i--) {
					if (newMessages[i].role === "assistant") {
						const existingCommands = newMessages[i].executedCommands || [];
						newMessages[i] = {
							...newMessages[i],
							executedCommands: [...existingCommands, ...pendingCommands],
						};
						break;
					}
				}
				return newMessages;
			});
			if (currentMessageIdRef.current) {
				void updateMessage(currentMessageIdRef.current, {
					executedCommands: nextExecutedCommands,
				});
			}
			setPendingCommands([]);
		}
	}, [messages, onExecuteCommands, pendingCommands]);

	const handleExecuteSingle = useCallback(
		(index: number) => {
			if (onExecuteCommands && pendingCommands[index]) {
				const command = pendingCommands[index];
				const lastAssistantMessage = [...messages]
					.reverse()
					.find((message) => message.role === "assistant");
				const nextExecutedCommands = [
					...(lastAssistantMessage?.executedCommands ?? []),
					command,
				];
				onExecuteCommands([command]);
				setMessages((prev) => {
					const newMessages = [...prev];
					for (let i = newMessages.length - 1; i >= 0; i--) {
						if (newMessages[i].role === "assistant") {
							const existingCommands = newMessages[i].executedCommands || [];
							newMessages[i] = {
								...newMessages[i],
								executedCommands: [...existingCommands, command],
							};
							break;
						}
					}
					return newMessages;
				});
				if (currentMessageIdRef.current) {
					void updateMessage(currentMessageIdRef.current, {
						executedCommands: nextExecutedCommands,
					});
				}
				setPendingCommands((prev) => prev.filter((_, i) => i !== index));
			}
		},
		[messages, onExecuteCommands, pendingCommands],
	);

	const handleDismissCommands = useCallback(() => {
		setPendingCommands([]);
	}, []);

	// UI mode handlers
	const handleApplyComponents = useCallback(() => {
		if (pendingComponents.length > 0) {
			const nextAppliedComponents = [...pendingComponents];
			onApplyComponents?.(pendingComponents, pendingCanvasSettings);
			setMessages((prev) => {
				const newMessages = [...prev];
				for (let i = newMessages.length - 1; i >= 0; i--) {
					if (newMessages[i].role === "assistant") {
						newMessages[i] = {
							...newMessages[i],
							appliedComponents: [...pendingComponents],
						};
						break;
					}
				}
				return newMessages;
			});
			if (currentMessageIdRef.current) {
				void updateMessage(currentMessageIdRef.current, {
					appliedComponents: nextAppliedComponents,
				});
			}
			setPendingComponents([]);
			setPendingCanvasSettings(undefined);
			setValidationWarnings([]);
		}
	}, [pendingComponents, pendingCanvasSettings, onApplyComponents]);

	const handleDismissComponents = useCallback(() => {
		setPendingComponents([]);
		setPendingCanvasSettings(undefined);
		setValidationWarnings([]);
	}, []);

	// Main submit handler
	const handleSubmit = useCallback(
		async (withScreenshot?: boolean) => {
			if (loading || (!input.trim() && attachedImages.length === 0)) return;

			let currentImages = [...attachedImages];
			const currentInput = input;
			const currentContextNodes = [...selectedNodeIds];
			const scope: CopilotScope =
				agentMode === "board"
					? "Board"
					: agentMode === "ui"
						? "Frontend"
						: "Both";

			if (scope === "Board" && !board) {
				setMessages((prev) => [
					...prev,
					{
						role: "assistant",
						content: "No board is currently loaded. Please load a board first.",
					},
				]);
				return;
			}

			if (
				provider === "copilot" &&
				(!copilotSDK.isRunning || !selectedModelId)
			) {
				setMessages((prev) => [
					...prev,
					{
						role: "assistant",
						content:
							"GitHub Copilot is not ready yet. Connect Copilot and select a model before sending.",
					},
				]);
				return;
			}

			// Capture screenshot if requested and captureScreenshot is provided
			if (withScreenshot && captureScreenshot) {
				try {
					const screenshotDataUrl = await captureScreenshot();
					if (screenshotDataUrl) {
						// Parse the data URL
						const match = screenshotDataUrl.match(
							/^data:(image\/(?:png|jpe?g|webp|gif));base64,(.+)$/,
						);
						const mediaType = match
							? normalizedDataUrlImageType(match[1])
							: null;
						if (
							match &&
							mediaType &&
							base64ByteLength(match[2]) <= MAX_ATTACHED_IMAGE_BYTES &&
							currentImages.length < MAX_ATTACHED_IMAGES
						) {
							currentImages = [
								{
									data: match[2],
									mediaType,
									preview: screenshotDataUrl,
								},
								...currentImages,
							];
						}
					}
				} catch (error) {
					console.error("Failed to capture screenshot:", error);
				}
			}

			// Reset state first
			setInput("");
			setAttachedImages([]);
			setLoading(true);
			setLoadingPhase("initializing");
			setLoadingStartTime(Date.now());
			setTokenCount(0);
			setPlanSteps([]);
			setUserScrolledUp(false);

			// In "both" mode, reset all pending states
			if (agentMode === "both") {
				setSuggestions([]);
				setPendingCommands([]);
				setPendingComponents([]);
				setValidationWarnings([]);
			} else if (agentMode === "board") {
				setSuggestions([]);
				setPendingCommands([]);
			} else {
				setPendingComponents([]);
				setValidationWarnings([]);
			}

			// Create or get conversation for persistence
			let conversationId = currentConversationId;
			if (!conversationId) {
				try {
					const newConversation = await createConversation(
						agentMode,
						board?.id,
						undefined,
					);
					conversationId = newConversation.id;
					setCurrentConversationId(conversationId);
					// Set initial title based on first message
					await updateConversation(conversationId, {
						title: currentInput.slice(0, 100) || "New conversation",
					});
				} catch (err) {
					console.error("Failed to create conversation:", err);
				}
			}

			// Save user message to DB
			if (conversationId) {
				try {
					await addMessage(conversationId, {
						role: "user",
						content: currentInput,
						images: currentImages.length > 0 ? currentImages : undefined,
						contextNodeIds:
							currentContextNodes.length > 0 ? currentContextNodes : undefined,
					});
					// Update conversation title if this is the first message
					if (messages.length === 0) {
						await updateConversation(conversationId, {
							title: currentInput.slice(0, 100),
						});
					}
				} catch (err) {
					console.error("Failed to save user message:", err);
				}
			}

			// Add user message and empty assistant message together
			setMessages((prev) => [
				...prev,
				{
					role: "user",
					content: currentInput,
					images: currentImages.length > 0 ? currentImages : undefined,
					contextNodeIds:
						currentContextNodes.length > 0 ? currentContextNodes : undefined,
				},
				{ role: "assistant", content: "" },
			]);

			// Store assistant message ID ref for updating later
			let assistantMessageId: string | undefined;
			if (conversationId) {
				try {
					const assistantMsg = await addMessage(conversationId, {
						role: "assistant",
						content: "",
					});
					assistantMessageId = assistantMsg.id;
					currentMessageIdRef.current = assistantMessageId;
				} catch (err) {
					console.error("Failed to create assistant message:", err);
				}
			}

			let phaseTimer: ReturnType<typeof setTimeout> | undefined;
			try {
				let currentMessageContent = "";
				let lastUpdateTime = 0;
				const UPDATE_INTERVAL = 100;
				let tagBuffer = ""; // Buffer for partial XML tags that might be split across tokens
				let currentPlanSteps: UnifiedPlanStep[] = [];

				phaseTimer = setTimeout(() => setLoadingPhase("analyzing"), 300);

				const flushMessageContent = () => {
					setMessages((prev) => {
						const newMessages = [...prev];
						const lastMessage = newMessages[newMessages.length - 1];
						if (lastMessage && lastMessage.role === "assistant") {
							lastMessage.content = currentMessageContent;
						}
						return newMessages;
					});
				};

				const onToken = (rawToken: string) => {
					setTokenCount((prev) => prev + 1);

					// Combine with buffer for partial tags
					let token = tagBuffer + rawToken;
					tagBuffer = "";

					// Check if we have an incomplete XML tag at the end
					const lastOpenTag = token.lastIndexOf("<");
					if (lastOpenTag !== -1 && !token.slice(lastOpenTag).includes(">")) {
						// Incomplete tag - buffer it for next token
						tagBuffer = token.slice(lastOpenTag);
						token = token.slice(0, lastOpenTag);
						if (!token) return; // Nothing to process yet
					}

					// Parse scope decision events (skip them - they're internal)
					const scopeDecisionMatch = token.match(
						/<scope_decision>([\s\S]*?)<\/scope_decision>/,
					);
					if (scopeDecisionMatch) {
						return;
					}

					// Parse tool start events (Copilot SDK)
					const toolStartMatch = token.match(
						/<tool_start>([\s\S]*?)<\/tool_start>/,
					);
					if (toolStartMatch) {
						try {
							const eventData = JSON.parse(toolStartMatch[1]);
							setCurrentToolCall(eventData.tool);
							// Update loading phase based on tool name
							if (
								eventData.tool?.includes("search") ||
								eventData.tool?.includes("catalog")
							) {
								setLoadingPhase("searching");
							} else if (
								eventData.tool === "get_node_details" ||
								eventData.tool === "list_board_nodes" ||
								eventData.tool === "get_component_schema"
							) {
								setLoadingPhase("reasoning");
							} else if (
								eventData.tool === "emit_commands" ||
								eventData.tool === "emit_ui"
							) {
								setLoadingPhase("generating");
							} else if (eventData.tool === "get_unconfigured_nodes") {
								setLoadingPhase("searching");
							}
						} catch {
							// Invalid JSON
						}
						return;
					}

					// Parse tool end events (Copilot SDK)
					const toolEndMatch = token.match(/<tool_end>([\s\S]*?)<\/tool_end>/);
					if (toolEndMatch) {
						try {
							const eventData = JSON.parse(toolEndMatch[1]);
							// Keep the tool name visible briefly, then clear
							setTimeout(() => setCurrentToolCall(null), 500);
						} catch {
							// Invalid JSON
						}
						return;
					}

					// Parse plan step events
					const planStepMatch = token.match(
						/<plan_step>([\s\S]*?)<\/plan_step>/,
					);
					if (planStepMatch) {
						try {
							const eventData = JSON.parse(planStepMatch[1]);
							if (eventData.PlanStep) {
								const step = eventData.PlanStep;
								// Update loading phase based on tool
								if (
									step.tool_name === "think" ||
									step.tool_name === "analyze"
								) {
									setLoadingPhase("reasoning");
								} else if (
									step.tool_name?.includes("search") ||
									step.tool_name?.includes("catalog") ||
									step.tool_name?.includes("schema") ||
									step.tool_name?.includes("style")
								) {
									setLoadingPhase("searching");
								} else if (
									step.tool_name === "emit_commands" ||
									step.tool_name === "emit_surface" ||
									step.tool_name === "modify_component"
								) {
									setLoadingPhase("generating");
								}

								const existingIndex = currentPlanSteps.findIndex(
									(s) => s.id === step.id,
								);
								currentPlanSteps =
									existingIndex >= 0
										? currentPlanSteps.map((existing, index) =>
												index === existingIndex ? step : existing,
											)
										: [...currentPlanSteps, step];
								setPlanSteps(currentPlanSteps);
							}
						} catch {
							// Invalid JSON
						}
						return;
					}

					// Handle tool calls (board mode)
					if (token.includes("tool_call:")) {
						const match = token.match(/tool_call:(\w+)/);
						if (match) {
							const toolName = match[1];
							setCurrentToolCall(toolName);
							if (
								toolName.includes("search") ||
								toolName.includes("catalog") ||
								toolName.includes("filter")
							) {
								setLoadingPhase("searching");
							} else if (toolName === "think") {
								setLoadingPhase("reasoning");
							} else if (
								toolName === "emit_commands" ||
								toolName === "emit_surface"
							) {
								setLoadingPhase("generating");
							}
						}
						return;
					}
					if (token.includes("tool_result:")) {
						setCurrentToolCall(null);
						return;
					}

					// Parse command blocks from Copilot SDK emit_commands tool
					const commandsMatch = token.match(/<commands>([\s\S]*?)<\/commands>/);
					if (commandsMatch) {
						try {
							const commands = JSON.parse(commandsMatch[1]);
							if (Array.isArray(commands) && commands.length > 0) {
								setPendingCommands((prev) => [...prev, ...commands]);
							}
						} catch {
							// Invalid JSON in commands
						}
						// Remove the commands tag from the token but keep any surrounding text
						const cleanedToken = token.replace(
							/<commands>[\s\S]*?<\/commands>/g,
							"",
						);
						if (!cleanedToken.trim()) return;
						// Continue with the cleaned token
						currentMessageContent += cleanedToken;
						flushMessageContent();
						return;
					}

					// Parse component blocks from Copilot SDK emit_ui tool
					const componentsMatch = token.match(
						/<components>([\s\S]*?)<\/components>/,
					);
					if (componentsMatch) {
						try {
							const components = JSON.parse(componentsMatch[1]);
							if (Array.isArray(components) && components.length > 0) {
								const { components: validatedBatch, warnings } =
									validateComponents(components);
								if (validatedBatch.length > 0) {
									setPendingComponents((prev) => [...prev, ...validatedBatch]);
								}
								if (warnings.length > 0) {
									setValidationWarnings((prev) => [...prev, ...warnings]);
								}
							}
						} catch {
							// Invalid JSON in components
						}
						// Remove the components tag from the token but keep any surrounding text
						const cleanedToken = token.replace(
							/<components>[\s\S]*?<\/components>/g,
							"",
						);
						if (!cleanedToken.trim()) return;
						currentMessageContent += cleanedToken;
						flushMessageContent();
						return;
					}

					// Parse canvas_settings blocks from Copilot SDK emit_ui tool
					const canvasSettingsMatch = token.match(
						/<canvas_settings>([\s\S]*?)<\/canvas_settings>/,
					);
					if (canvasSettingsMatch) {
						try {
							const settings = JSON.parse(canvasSettingsMatch[1]);
							setPendingCanvasSettings(validateCanvasSettings(settings));
						} catch {
							// Invalid JSON in canvas settings
						}
						// Remove the tag and continue
						const cleanedToken = token.replace(
							/<canvas_settings>[\s\S]*?<\/canvas_settings>/g,
							"",
						);
						if (!cleanedToken.trim()) return;
						currentMessageContent += cleanedToken;
						flushMessageContent();
						return;
					}

					// First token? Set loading phase immediately
					if (currentMessageContent.length === 0 && token.trim()) {
						setLoadingPhase("generating");
						// Flush immediately on first content to avoid losing it
						currentMessageContent += token;
						flushMessageContent();
						return;
					}

					currentMessageContent += token;

					// Throttle UI updates
					const now = Date.now();
					if (now - lastUpdateTime >= UPDATE_INTERVAL) {
						lastUpdateTime = now;
						flushMessageContent();
					}
				};

				// Build the prompt with context
				let userMsg = currentInput;
				if (runContext) {
					const runInfo = {
						run_id: runContext.run_id,
						app_id: runContext.app_id,
						board_id: runContext.board_id,
						event_id: runContext.event_id,
					};
					userMsg = `[RUN CONTEXT - User is asking about a flow execution run. Use the query_logs tool to fetch relevant logs.]
\`\`\`json
${JSON.stringify(runInfo, null, 2)}
\`\`\`

${currentInput}`;
				}

				if (scope === "Both") {
					userMsg = `[UNIFIED MODE - You can generate both workflow nodes AND UI components. If the user wants a UI, you can create A2UI components. If they want workflow automation, create nodes. You can also connect UI actions to workflows via action invokes.]

${userMsg}`;
				}

				const chatHistory: UnifiedChatMessage[] = buildBudgetedHistory({
					agentMode,
					messages,
					selectedNodeIds,
					selectedComponentIds,
					boardId: board?.id,
					boardName: board?.name,
					currentComponentsCount: currentComponents.length,
					runContext,
				});

				const requestImages = currentImages.map((img) => ({
					data: img.data,
					media_type: img.mediaType,
				}));

				const backendRunContext = runContext
					? {
							run_id: runContext.run_id,
							app_id: runContext.app_id,
							board_id: runContext.board_id,
						}
					: undefined;

				// Prefix model_id with "copilot:" when using Copilot provider
				// This tells the backend to use Copilot SDK instead of bits
				const effectiveModelId =
					provider === "copilot" && selectedModelId
						? `copilot:${selectedModelId}`
						: selectedModelId || undefined;

				const response = await backendContext.boardState.copilot_chat(
					scope,
					board ?? null,
					selectedNodeIds,
					currentComponents,
					selectedComponentIds,
					userMsg,
					chatHistory,
					requestImages,
					onToken,
					effectiveModelId,
					undefined,
					backendRunContext,
					undefined, // actionContext - can be added later
				);

				flushMessageContent();

				const finalAssistantContent =
					currentMessageContent || response.message || "";

				// Save final assistant message to DB
				if (assistantMessageId && finalAssistantContent) {
					try {
						await updateMessage(assistantMessageId, {
							content: finalAssistantContent,
						});
					} catch (err) {
						console.error("Failed to update assistant message:", err);
					}
				}

				setMessages((prev) => {
					const newMessages = [...prev];
					const lastMessage = newMessages[newMessages.length - 1];
					if (lastMessage && lastMessage.role === "assistant") {
						lastMessage.planSteps = currentPlanSteps.filter(
							(s) => s.status === "Completed",
						);
						if (!lastMessage.content.trim()) {
							lastMessage.content = finalAssistantContent;
						}
					}
					return newMessages;
				});

				// Handle board commands
				if (response.commands.length > 0) {
					setPendingCommands(response.commands);
				}

				// Handle suggestions
				if (response.suggestions?.length > 0) {
					setSuggestions(
						response.suggestions.map((s) => ({
							node_type: s.label,
							reason: s.prompt,
							connection_description: "",
							connections: [],
						})),
					);
				}

				const validatedCanvasSettings = validateCanvasSettings(
					response.canvas_settings,
				);
				if (validatedCanvasSettings) {
					setPendingCanvasSettings(validatedCanvasSettings);
				}

				// Handle generated components — validate before showing
				if (response.components.length > 0) {
					const { components: validatedFinal, warnings: finalWarnings } =
						validateComponents(response.components);
					if (validatedFinal.length > 0) {
						setPendingComponents(validatedFinal);
						onComponentsGenerated?.(validatedFinal);
					}
					if (finalWarnings.length > 0) {
						setValidationWarnings((prev) => [...prev, ...finalWarnings]);
					}
				}

				setLoadingPhase("finalizing");
			} catch (error) {
				console.error("FlowPilot error:", error);
				setMessages((prev) => {
					const newMessages = [...prev];
					const lastMessage = newMessages[newMessages.length - 1];
					if (lastMessage?.role === "assistant") {
						let errorMessage =
							error instanceof Error ? error.message : "Unknown error";

						if (
							errorMessage.includes("401 Unauthorized") ||
							errorMessage.includes("status code 401")
						) {
							errorMessage =
								"Authentication failed. Please check if you are signed in and your session is active.";
						}

						lastMessage.content = `Error: ${errorMessage}`;
						if (assistantMessageId) {
							void updateMessage(assistantMessageId, {
								content: lastMessage.content,
							});
						}
					}
					return newMessages;
				});
			} finally {
				if (phaseTimer) clearTimeout(phaseTimer);
				setLoading(false);
				setLoadingPhase("idle");
				setLoadingStartTime(null);
				setCurrentToolCall(null);
			}
		},
		[
			input,
			attachedImages,
			agentMode,
			messages,
			board,
			selectedNodeIds,
			selectedModelId,
			runContext,
			currentComponents,
			selectedComponentIds,
			onComponentsGenerated,
			backendContext.boardState,
			captureScreenshot,
			provider,
			copilotSDK.isRunning,
			currentConversationId,
			loading,
		],
	);

	// Keep ref updated for keydown handler
	useEffect(() => {
		handleSubmitRef.current = handleSubmit;
	}, [handleSubmit]);

	// Handle key down
	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLTextAreaElement>) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				handleSubmitRef.current?.();
			}
		},
		[],
	);

	// Handle initial prompt
	useEffect(() => {
		if (
			initialPrompt &&
			!initialPromptHandledRef.current &&
			selectedModelId &&
			(agentMode === "ui" || board)
		) {
			initialPromptHandledRef.current = true;
			setInput(initialPrompt);
			setTimeout(() => {
				handleSubmitRef.current?.();
			}, 100);
		}
	}, [initialPrompt, selectedModelId, agentMode, board]);

	// Get placeholder text based on mode
	const placeholderText = useMemo(() => {
		if (agentMode === "both") {
			if (runContext) return "Ask about logs or describe what to build...";
			const hasSelection =
				selectedNodeIds.length > 0 || selectedComponentIds.length > 0;
			if (hasSelection) return "Describe changes to selected items...";
			return "Describe a workflow, UI, or both together...";
		}
		if (agentMode === "board") {
			if (runContext) return "Ask about the logs...";
			if (selectedNodeIds.length > 0)
				return "Describe changes to selected nodes...";
			return "Ask anything about your flow...";
		}
		if (selectedComponentIds.length > 0) {
			return "Describe changes to selected components...";
		}
		return "Describe the UI you want to create...";
	}, [
		agentMode,
		runContext,
		selectedNodeIds.length,
		selectedComponentIds.length,
	]);

	// Get context indicator based on mode
	const contextIndicator = useMemo(() => {
		// In "both" mode, show combined context
		if (agentMode === "both") {
			const hasNodes = selectedNodeIds.length > 0;
			const hasComponents = selectedComponentIds.length > 0;
			if (!hasNodes && !hasComponents) return null;

			return (
				<div className="flex items-center gap-2 mb-2 flex-wrap">
					{hasNodes && (
						<ContextNodes
							nodeIds={selectedNodeIds}
							board={board ?? undefined}
							onSelectNodes={onSelectNodes}
							onFocusNode={onFocusNode}
							compact
						/>
					)}
					{hasComponents && (
						<div className="flex items-center gap-1.5 text-xs text-muted-foreground">
							<LayoutGridIcon className="w-3.5 h-3.5" />
							<span>
								{selectedComponentIds.length} component
								{selectedComponentIds.length !== 1 ? "s" : ""}
							</span>
						</div>
					)}
				</div>
			);
		}
		if (agentMode === "board" && selectedNodeIds.length > 0) {
			return (
				<ContextNodes
					nodeIds={selectedNodeIds}
					board={board ?? undefined}
					onSelectNodes={onSelectNodes}
					onFocusNode={onFocusNode}
					compact
				/>
			);
		}
		if (agentMode === "ui" && selectedComponentIds.length > 0) {
			return (
				<div className="flex items-center gap-1.5 mb-2 text-xs text-muted-foreground">
					<LayoutGridIcon className="w-3.5 h-3.5" />
					<span>
						{selectedComponentIds.length} component
						{selectedComponentIds.length !== 1 ? "s" : ""} selected
					</span>
				</div>
			);
		}
		return null;
	}, [
		agentMode,
		selectedNodeIds,
		selectedComponentIds,
		board,
		onSelectNodes,
		onFocusNode,
	]);

	return (
		<motion.div
			layoutId="flowpilot"
			initial={{ opacity: 0, x: 100 }}
			animate={{ opacity: 1, x: 0 }}
			exit={{ opacity: 0, x: 100 }}
			transition={{ type: "spring", stiffness: 400, damping: 30 }}
			className={cn(
				"h-full w-full flex flex-col bg-background/95 backdrop-blur-xl border-l border-border/40 overflow-hidden",
				className,
			)}
		>
			{/* Header */}
			<Header
				title={title}
				loading={loading}
				loadingPhase={loadingPhase}
				elapsedSeconds={elapsedSeconds}
				runContext={runContext}
				onNewChat={handleNewChat}
				onClose={onClose}
				showHistory={showHistory}
				setShowHistory={setShowHistory}
				// Provider props
				provider={provider}
				onProviderChange={setProvider}
				forceProvider={forceProvider}
				copilotSDK={copilotSDK}
				onStartCopilot={handleStartCopilot}
				onStopCopilot={handleStopCopilot}
				// Model props
				bitsModels={bitsModels}
				selectedModelId={selectedModelId}
				setSelectedModelId={setSelectedModelId}
			/>

			{/* Messages area */}
			<ScrollArea
				className="flex-1 min-h-0 px-3 overflow-x-hidden"
				viewportRef={scrollContainerRef}
				onScroll={handleScroll}
			>
				<div className="py-3 space-y-3 min-w-0 overflow-hidden">
					{messages.length === 0 ? (
						<EmptyState
							agentMode={agentMode}
							selectedCount={
								agentMode === "board"
									? selectedNodeIds.length
									: selectedComponentIds.length
							}
							setInput={setInput}
						/>
					) : (
						messages.map((message, index) => (
							<MessageBubble
								key={index}
								message={message}
								isLoading={loading && index === messages.length - 1}
								loadingPhase={loadingPhase}
								currentToolCall={currentToolCall}
								currentStep={
									loading && index === messages.length - 1
										? planSteps.find((s) => s.status === "InProgress")
										: undefined
								}
								agentMode={agentMode}
								board={board}
								onFocusNode={onFocusNode}
								onSelectNodes={onSelectNodes}
							/>
						))
					)}
					<div ref={messagesEndRef} />
				</div>
			</ScrollArea>

			{/* Scroll to bottom indicator */}
			<AnimatePresence>
				{userScrolledUp && messages.length > 0 && (
					<motion.div
						initial={{ opacity: 0, y: 10 }}
						animate={{ opacity: 1, y: 0 }}
						exit={{ opacity: 0, y: 10 }}
						className="absolute bottom-36 left-1/2 -translate-x-1/2 z-10"
					>
						<Button
							size="sm"
							variant="secondary"
							className="rounded-full shadow-lg border border-border/50 gap-1 px-2 h-6 text-[10px]"
							onClick={() => scrollToBottom(true)}
						>
							<ArrowDown className="w-3 h-3" />
							New
						</Button>
					</motion.div>
				)}
			</AnimatePresence>

			{/* Plan steps */}
			{planSteps.length > 0 && (
				<PlanStepsView steps={planSteps} loading={loading} compact />
			)}

			{/* Pending commands (board mode or both mode) */}
			{(agentMode === "board" || agentMode === "both") &&
				pendingCommands.length > 0 && (
					<div className="px-3 pb-2">
						<PendingCommandsView
							commands={pendingCommands}
							onExecute={handleExecuteCommands}
							onExecuteSingle={handleExecuteSingle}
							onDismiss={handleDismissCommands}
						/>
					</div>
				)}

			{/* Pending components (UI mode or both mode) */}
			{(agentMode === "ui" || agentMode === "both") &&
				pendingComponents.length > 0 && (
					<PendingComponentsView
						components={pendingComponents}
						canvasSettings={pendingCanvasSettings}
						warnings={validationWarnings}
						onApply={handleApplyComponents}
						onDismiss={handleDismissComponents}
					/>
				)}

			{/* Input area */}
			<div className="shrink-0 p-2.5 border-t border-border/30 bg-background/80 backdrop-blur-sm">
				{/* Context indicator */}
				{contextIndicator}

				{/* Image previews */}
				{attachedImages.length > 0 && (
					<div className="flex gap-1.5 mb-2 flex-wrap">
						{attachedImages.map((img, idx) => (
							<div key={idx} className="relative group">
								<img
									src={img.preview}
									alt={`Attached ${idx + 1}`}
									className="h-12 w-12 object-cover rounded-md border border-border/50"
								/>
								<button
									type="button"
									onClick={() => handleRemoveImage(idx)}
									className="absolute -top-1 -right-1 w-4 h-4 bg-destructive text-destructive-foreground rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
								>
									<XIcon className="w-2.5 h-2.5" />
								</button>
							</div>
						))}
					</div>
				)}

				<div className="relative flex items-center gap-1.5">
					<input
						type="file"
						ref={imageInputRef}
						accept="image/png,image/jpeg,image/webp,image/gif"
						multiple
						onChange={handleImageSelect}
						className="hidden"
					/>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								type="button"
								variant="ghost"
								size="icon"
								className="h-10 w-10 shrink-0 rounded-lg hover:bg-accent/50"
								onClick={() => imageInputRef.current?.click()}
								disabled={
									loading || attachedImages.length >= MAX_ATTACHED_IMAGES
								}
							>
								<ImageIcon className="w-4 h-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="top" className="text-xs">
							Attach image (paste supported)
						</TooltipContent>
					</Tooltip>
					<textarea
						value={input}
						onChange={(e) => setInput(e.target.value)}
						onKeyDown={handleKeyDown}
						onPaste={handlePaste}
						placeholder={placeholderText}
						className={cn(
							"flex-1 min-h-10 max-h-30 text-sm py-2.5 px-3 rounded-lg bg-background/80 border border-border/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/20 focus-visible:border-primary/50 resize-none",
							captureScreenshot ? "pr-18" : "pr-12",
						)}
						disabled={loading}
						rows={1}
					/>
					{captureScreenshot ? (
						// Split button: Send + dropdown with screenshot option
						<div className="absolute right-1 top-1/2 -translate-y-1/2 flex items-center">
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										size="icon"
										onClick={() => handleSubmit(false)}
										disabled={
											loading || (!input.trim() && attachedImages.length === 0)
										}
										className="h-8 w-8 rounded-l-lg rounded-r-none shadow-md bg-linear-to-br from-primary to-purple-600 hover:shadow-lg hover:shadow-primary/20 transition-all duration-200 disabled:opacity-50"
									>
										{loading ? (
											<Loader2 className="w-3.5 h-3.5 animate-spin" />
										) : (
											<SendIcon className="w-3.5 h-3.5" />
										)}
									</Button>
								</TooltipTrigger>
								<TooltipContent side="top" className="text-xs">
									Send message
								</TooltipContent>
							</Tooltip>
							<DropdownMenu>
								<DropdownMenuTrigger asChild>
									<Button
										size="icon"
										disabled={
											loading || (!input.trim() && attachedImages.length === 0)
										}
										className="h-8 w-6 rounded-l-none rounded-r-lg shadow-md bg-linear-to-br from-purple-600 to-pink-600 hover:shadow-lg hover:shadow-primary/20 transition-all duration-200 disabled:opacity-50 border-l border-white/20"
									>
										<ChevronDownIcon className="w-3 h-3" />
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent align="end" className="min-w-45">
									<DropdownMenuItem
										onClick={() => handleSubmit(false)}
										disabled={loading}
									>
										<SendIcon className="w-4 h-4 mr-2" />
										Send
									</DropdownMenuItem>
									<DropdownMenuItem
										onClick={() => handleSubmit(true)}
										disabled={loading}
									>
										<CameraIcon className="w-4 h-4 mr-2" />
										Send with screenshot
									</DropdownMenuItem>
								</DropdownMenuContent>
							</DropdownMenu>
						</div>
					) : (
						// Simple send button when no screenshot function
						<Button
							size="icon"
							onClick={() => handleSubmit(false)}
							disabled={
								loading || (!input.trim() && attachedImages.length === 0)
							}
							className="absolute right-1 top-1/2 -translate-y-1/2 h-8 w-8 rounded-lg shadow-md bg-linear-to-br from-primary to-purple-600 hover:shadow-lg hover:shadow-primary/20 transition-all duration-200 disabled:opacity-50"
						>
							{loading ? (
								<Loader2 className="w-3.5 h-3.5 animate-spin" />
							) : (
								<SendIcon className="w-3.5 h-3.5" />
							)}
						</Button>
					)}
				</div>
			</div>

			{/* History Panel */}
			<HistoryPanel
				mode={agentMode}
				currentConversationId={currentConversationId}
				onSelectConversation={handleSelectConversation}
				onNewConversation={handleNewChat}
				isOpen={showHistory}
				onClose={() => setShowHistory(false)}
			/>
		</motion.div>
	);
}

// Header component
interface HeaderProps {
	title: string;
	loading: boolean;
	loadingPhase: LoadingPhase;
	elapsedSeconds: number;
	runContext?: { run_id: string };
	onNewChat: () => void;
	onClose?: () => void;
	// History props
	showHistory: boolean;
	setShowHistory: (show: boolean) => void;
	// Provider props
	provider: AIProvider;
	onProviderChange: (provider: AIProvider) => void;
	forceProvider?: AIProvider;
	copilotSDK: ReturnType<typeof useCopilotSDK>;
	onStartCopilot: (serverUrl?: string) => Promise<void>;
	onStopCopilot: () => Promise<void>;
	// Model props
	bitsModels: any[];
	selectedModelId: string;
	setSelectedModelId: (id: string) => void;
}

const Header = memo(function Header({
	title,
	loading,
	loadingPhase,
	elapsedSeconds,
	runContext,
	onNewChat,
	onClose,
	showHistory,
	setShowHistory,
	provider,
	onProviderChange,
	forceProvider,
	copilotSDK,
	onStartCopilot,
	onStopCopilot,
	bitsModels,
	selectedModelId,
	setSelectedModelId,
}: HeaderProps) {
	const currentModels = provider === "copilot" ? copilotSDK.models : bitsModels;

	return (
		<div className="relative overflow-hidden shrink-0">
			<div className="absolute inset-0 bg-linear-to-br from-primary/8 via-violet-500/5 to-pink-500/5" />
			{loading && (
				<motion.div
					className="absolute inset-0 opacity-30"
					style={{
						background:
							"radial-gradient(circle at 30% 50%, rgba(139, 92, 246, 0.3), transparent 50%), radial-gradient(circle at 70% 50%, rgba(236, 72, 153, 0.3), transparent 50%)",
					}}
					animate={{
						background: [
							"radial-gradient(circle at 30% 50%, rgba(139, 92, 246, 0.3), transparent 50%), radial-gradient(circle at 70% 50%, rgba(236, 72, 153, 0.3), transparent 50%)",
							"radial-gradient(circle at 70% 50%, rgba(139, 92, 246, 0.3), transparent 50%), radial-gradient(circle at 30% 50%, rgba(236, 72, 153, 0.3), transparent 50%)",
						],
					}}
					transition={{
						duration: 3,
						repeat: Number.POSITIVE_INFINITY,
						repeatType: "reverse",
					}}
				/>
			)}

			<div className="relative px-3 py-2.5 flex items-center justify-between">
				<div className="flex items-center gap-2">
					<div className="relative">
						<motion.div
							className="absolute inset-0 bg-linear-to-br from-primary to-violet-600 rounded-lg blur-md opacity-50"
							animate={
								loading ? { scale: [1, 1.2, 1], opacity: [0.5, 0.8, 0.5] } : {}
							}
							transition={{ duration: 2, repeat: Number.POSITIVE_INFINITY }}
						/>
						<div className="relative p-1.5 bg-linear-to-br from-primary via-violet-600 to-pink-600 rounded-lg shadow-md">
							<SparklesIcon className="w-3.5 h-3.5 text-white" />
						</div>
					</div>
					<div>
						<h3 className="text-sm font-bold">{title}</h3>
						{loading ? (
							<StatusPill
								phase={loadingPhase}
								elapsed={elapsedSeconds}
								compact
							/>
						) : (
							<div className="flex items-center gap-1 text-xs text-muted-foreground">
								<span className="relative flex h-1.5 w-1.5">
									<span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75" />
									<span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-green-500" />
								</span>
								{runContext ? "Log context active" : "Ready"}
							</div>
						)}
					</div>
				</div>
				<div className="flex items-center gap-2">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-7 w-7 rounded-md hover:bg-accent/50"
								onClick={() => setShowHistory(!showHistory)}
							>
								<ClockIcon className="w-4 h-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							History
						</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-7 w-7 rounded-md hover:bg-accent/50"
								onClick={onNewChat}
							>
								<SquarePenIcon className="w-4 h-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							New chat
						</TooltipContent>
					</Tooltip>
					{onClose && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7 rounded-md hover:bg-accent/50"
									onClick={onClose}
								>
									<XIcon className="w-4 h-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom" className="text-xs">
								Close
							</TooltipContent>
						</Tooltip>
					)}
				</div>
			</div>

			{/* Provider and Model selector */}
			<div className="relative px-3 pb-2 flex items-center gap-2">
				{/* Provider selector (only show if not forced) */}
				{!forceProvider && (
					<ProviderSelector
						provider={provider}
						onProviderChange={onProviderChange}
						copilotModels={copilotSDK.models}
						copilotAuthStatus={copilotSDK.authStatus}
						copilotRunning={copilotSDK.isRunning}
						copilotConnecting={copilotSDK.isConnecting}
						onStartCopilot={onStartCopilot}
						onStopCopilot={onStopCopilot}
						disabled={loading}
					/>
				)}

				{/* Model selector */}
				<ModelSelector
					provider={provider}
					bitsModels={bitsModels}
					copilotModels={copilotSDK.models}
					selectedModelId={selectedModelId}
					onModelChange={setSelectedModelId}
					disabled={loading}
					className="flex-1"
				/>
			</div>

			{loading && (
				<motion.div
					className="absolute bottom-0 left-0 right-0 h-0.5 bg-muted/30"
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
				>
					<motion.div
						className="h-full bg-linear-to-r from-primary via-violet-500 to-pink-500"
						initial={{ width: "0%" }}
						animate={{ width: "100%" }}
						transition={{ duration: 30, ease: "linear" }}
					/>
				</motion.div>
			)}
		</div>
	);
});

// Empty state component
interface EmptyStateProps {
	agentMode: AgentMode;
	selectedCount: number;
	setInput: (v: string) => void;
}

const EmptyState = memo(function EmptyState({
	agentMode,
	selectedCount,
	setInput,
}: EmptyStateProps) {
	const suggestions = useMemo(() => {
		if (agentMode === "both") {
			return selectedCount > 0
				? ["Explain this", "Create UI for it", "Add workflow step"]
				: [
						"Create a dashboard with API",
						"Build form + workflow",
						"Design UI with data flow",
					];
		}
		if (agentMode === "board") {
			return selectedCount > 0
				? ["Explain this node", "Connect to output", "Add error handling"]
				: ["Create a REST API node", "Build a data pipeline", "Add logging"];
		}
		return selectedCount > 0
			? ["Make it larger", "Change the color", "Add a border"]
			: ["Create a login form", "Build a card component", "Design a navbar"];
	}, [agentMode, selectedCount]);

	const description = useMemo(() => {
		if (selectedCount > 0) {
			if (agentMode === "both")
				return "Describe what to do with the selected items";
			return `Describe what to do with the selected ${agentMode === "board" ? "nodes" : "components"}`;
		}
		if (agentMode === "both") return "Build workflows, UIs, or both together";
		if (agentMode === "board") return "Ask questions or build your flow";
		return "Describe the UI you want to create";
	}, [agentMode, selectedCount]);

	return (
		<div className="flex flex-col items-center justify-center py-8 text-center">
			<motion.div
				initial={{ scale: 0 }}
				animate={{ scale: 1 }}
				transition={{ type: "spring", stiffness: 400, damping: 20 }}
			>
				<div className="relative">
					<motion.div
						className="absolute inset-0 bg-linear-to-br from-primary/30 to-violet-500/30 rounded-full blur-xl"
						animate={{ scale: [1, 1.2, 1], opacity: [0.5, 0.8, 0.5] }}
						transition={{ duration: 3, repeat: Number.POSITIVE_INFINITY }}
					/>
					<SparklesIcon className="w-12 h-12 relative text-primary/50" />
				</div>
			</motion.div>
			<p className="text-sm font-medium text-foreground mt-3 mb-1">
				How can I help?
			</p>
			<p className="text-xs text-muted-foreground max-w-50">{description}</p>
			<div className="flex flex-wrap gap-2 justify-center pt-4">
				{suggestions.map((suggestion, i) => (
					<motion.button
						key={suggestion}
						initial={{ opacity: 0, y: 5 }}
						animate={{ opacity: 1, y: 0 }}
						transition={{ delay: 0.1 * i }}
						onClick={() => setInput(suggestion)}
						className="px-3 py-1.5 text-xs rounded-full bg-muted/50 hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
					>
						{suggestion}
					</motion.button>
				))}
			</div>
		</div>
	);
});

// Message bubble component
interface MessageBubbleProps {
	message: CopilotMessage;
	isLoading?: boolean;
	loadingPhase?: LoadingPhase;
	currentToolCall?: string | null;
	currentStep?: UnifiedPlanStep;
	agentMode: AgentMode;
	board?: any;
	onFocusNode?: (nodeId: string) => void;
	onSelectNodes?: (nodeIds: string[]) => void;
}

const MessageBubble = memo(function MessageBubble({
	message,
	isLoading,
	loadingPhase,
	currentToolCall,
	currentStep,
	agentMode,
	board,
	onFocusNode,
	onSelectNodes,
}: MessageBubbleProps) {
	const isUser = message.role === "user";

	const getLoadingContent = () => {
		// Show current step with details
		if (currentStep) {
			const hasContent = message.content && message.content.trim().length > 0;
			return (
				<div
					className={`space-y-1.5 ${hasContent ? "mt-3 pt-2 border-t border-border/30" : ""}`}
				>
					<div className="flex items-center gap-2">
						<Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
						<span className="text-xs font-medium text-foreground">
							{currentStep.tool_name === "think" ||
							currentStep.tool_name === "analyze"
								? "Thinking"
								: currentStep.tool_name === "emit_surface"
									? "Generating UI"
									: currentStep.tool_name === "emit_commands"
										? "Building flow"
										: currentStep.tool_name === "get_component_schema"
											? "Looking up schema"
											: currentStep.tool_name === "get_style_examples"
												? "Fetching styles"
												: currentStep.tool_name?.replace(/_/g, " ") ||
													"Processing"}
						</span>
					</div>
					{currentStep.description && (
						<p className="text-xs text-muted-foreground pl-5 whitespace-pre-wrap line-clamp-4">
							{currentStep.description}
						</p>
					)}
				</div>
			);
		}

		// Fallback to phase-based loading
		const hasContent = message.content && message.content.trim().length > 0;
		return (
			<div
				className={`flex items-center gap-2 ${hasContent ? "mt-3 pt-2 border-t border-border/30" : ""}`}
			>
				<Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
				<span className="text-xs text-muted-foreground">
					{currentToolCall
						? `Using ${currentToolCall.replace(/_/g, " ")}...`
						: loadingPhase && loadingPhase !== "idle"
							? loadingPhase.charAt(0).toUpperCase() +
								loadingPhase.slice(1) +
								"..."
							: "Processing..."}
				</span>
			</div>
		);
	};

	return (
		<motion.div
			initial={{ opacity: 0, y: 10 }}
			animate={{ opacity: 1, y: 0 }}
			className={cn("flex", isUser ? "justify-end" : "justify-start")}
		>
			<div
				className={cn(
					"px-3 py-2 rounded-xl text-sm min-w-0 max-w-[85%] overflow-hidden",
					isUser
						? "bg-muted/60 text-foreground rounded-br-sm border border-border/40"
						: "bg-background border border-border/40 rounded-bl-sm",
				)}
				style={{ wordBreak: "break-word", overflowWrap: "anywhere" }}
			>
				{/* Images */}
				{message.images && message.images.length > 0 && (
					<div className="flex gap-1.5 mb-2 flex-wrap">
						{message.images.map((img, idx) => (
							<img
								key={idx}
								src={img.preview}
								alt={`Attached ${idx + 1}`}
								className="h-16 rounded-md"
							/>
						))}
					</div>
				)}

				{/* Context nodes (board mode or both mode, user messages) */}
				{isUser &&
					(agentMode === "board" || agentMode === "both") &&
					message.contextNodeIds &&
					message.contextNodeIds.length > 0 && (
						<ContextNodes
							nodeIds={message.contextNodeIds}
							board={board}
							onSelectNodes={onSelectNodes}
							onFocusNode={onFocusNode}
							compact
						/>
					)}

				{/* Content */}
				{message.content ? (
					<MessageContent
						content={message.content}
						onFocusNode={onFocusNode}
						board={
							agentMode === "board" || agentMode === "both" ? board : undefined
						}
						enableMarkdown={true}
					/>
				) : isLoading ? null : (
					<p className="text-muted-foreground italic text-xs">No response</p>
				)}

				{/* Loading indicator - show when loading, regardless of content */}
				{isLoading && getLoadingContent()}

				{/* Applied components badge (UI mode) */}
				{message.appliedComponents && message.appliedComponents.length > 0 && (
					<div className="mt-2 flex items-center gap-1 text-green-600 text-xs">
						<CheckCircle2 className="w-3 h-3" />
						<span>{message.appliedComponents.length} components applied</span>
					</div>
				)}

				{/* Executed commands badge (board mode) */}
				{message.executedCommands && message.executedCommands.length > 0 && (
					<div className="mt-2 flex items-center gap-1 text-green-600 text-xs">
						<CheckCircle2 className="w-3 h-3" />
						<span>{message.executedCommands.length} changes applied</span>
					</div>
				)}
			</div>
		</motion.div>
	);
});
