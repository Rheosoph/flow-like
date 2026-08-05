import type {
	IChatPlaceholderBubbleState,
	IChatPlaceholderVisual,
} from "./schema/flow/event-payload-chat";

export type ChatColorScheme = "system" | "light" | "dark";

export const DEFAULT_CHAT_AI_DISCLOSURE =
	"You’re chatting with AI — brilliant at patterns, occasionally creative with facts. Double-check the important stuff.";

/**
 * Shown in the chat's empty state when an event configures no example messages.
 * The config screen renders these too, so it's obvious where the prompts users
 * are already seeing actually come from.
 */
export const DEFAULT_CHAT_EXAMPLE_MESSAGES: readonly string[] = [
	"Help me brainstorm ideas for a new project",
	"Explain how machine learning works",
	"Help me debug this code issue",
	"What's the latest in technology?",
	"Write a professional email",
	"Create a workout plan",
	"Explain quantum computing",
	"Help with meal planning",
];

export const CHAT_COLOR_SCHEMES: ReadonlyArray<{
	value: ChatColorScheme;
	label: string;
}> = [
	{ value: "system", label: "App theme" },
	{ value: "light", label: "Light" },
	{ value: "dark", label: "Dark" },
];

export function resolveChatColorScheme(value: unknown): ChatColorScheme {
	return value === "light" || value === "dark" ? value : "system";
}

export const CHAT_PLACEHOLDER_VISUALS: ReadonlyArray<{
	value: IChatPlaceholderVisual;
	label: string;
	description: string;
}> = [
	{
		value: "planet",
		label: "Planet",
		description:
			"A slowly rotating point sphere that leans toward the pointer.",
	},
	{
		value: "bubble",
		label: "Bubble",
		description: "The iridescent soap-film orb, resting in the state you pick.",
	},
	{
		value: "image",
		label: "Custom image",
		description: "Your own square image — a logo or an assistant avatar.",
	},
	{
		value: "none",
		label: "None",
		description: "No mark at all. The example prompts carry the empty state.",
	},
];

/** Mirrors the orb's activity states, so the placeholder is a pose of the live mark. */
export const CHAT_PLACEHOLDER_BUBBLE_STATES: ReadonlyArray<{
	value: IChatPlaceholderBubbleState;
	label: string;
	description: string;
}> = [
	{ value: "idle", label: "Idle", description: "Resting, almost still." },
	{ value: "ready", label: "Ready", description: "Awake and expectant." },
	{
		value: "thinking",
		label: "Thinking",
		description: "Churning, with three satellites in orbit.",
	},
	{
		value: "working",
		label: "Working",
		description: "Turning, with a cog rim.",
	},
];

export function resolveChatPlaceholderVisual(
	value: unknown,
): IChatPlaceholderVisual {
	return CHAT_PLACEHOLDER_VISUALS.some((option) => option.value === value)
		? (value as IChatPlaceholderVisual)
		: "planet";
}

export function resolveChatPlaceholderBubbleState(
	value: unknown,
): IChatPlaceholderBubbleState {
	return CHAT_PLACEHOLDER_BUBBLE_STATES.some((option) => option.value === value)
		? (value as IChatPlaceholderBubbleState)
		: "idle";
}

/**
 * Whether the mark answers the composer while the user writes. Opt-in: an interface that never
 * asked for it keeps a mark that only breathes and follows the pointer.
 */
export function resolveChatPlaceholderTypingMotion(value: unknown): boolean {
	return value === true;
}

/** The marks that can react — an image or no mark at all has nothing to animate. */
export function chatPlaceholderSupportsTypingMotion(
	visual: IChatPlaceholderVisual,
): boolean {
	return visual === "planet" || visual === "bubble";
}

export function resolveChatAiDisclosure(value: unknown): string {
	if (typeof value !== "string") return DEFAULT_CHAT_AI_DISCLOSURE;
	return value.trim() || DEFAULT_CHAT_AI_DISCLOSURE;
}

export function escapeCssAttributeValue(value: string): string {
	return value
		.replace(/\\/g, "\\\\")
		.replace(/"/g, '\\"')
		.replace(/\r\n|\r|\n/g, "\\a ");
}

export function createChatBackgroundImage(value: unknown): string | undefined {
	if (typeof value !== "string" || !value.trim()) return undefined;
	const trimmed = value.trim();
	const protocol = trimmed.match(/^([a-z][a-z0-9+.-]*):/i)?.[1]?.toLowerCase();
	const safeDataImage =
		/^data:image\/(?:png|jpe?g|webp|gif|svg\+xml|x-icon|vnd\.microsoft\.icon|bmp|avif);base64,[a-z0-9+/=\s]+$/i.test(
			trimmed,
		);
	if (
		protocol &&
		!["http", "https", "blob", "asset"].includes(protocol) &&
		!safeDataImage
	) {
		return undefined;
	}

	const url = JSON.stringify(trimmed);
	return `linear-gradient(to bottom, var(--fl-chat-background-overlay) 0%, var(--fl-chat-background-overlay) 48%, var(--fl-chat-background-overlay-strong, var(--fl-chat-background-overlay)) 100%), url(${url})`;
}
