export type ChatColorScheme = "system" | "light" | "dark";

export const DEFAULT_CHAT_AI_DISCLOSURE =
	"You’re chatting with AI — brilliant at patterns, occasionally creative with facts. Double-check the important stuff.";

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
	return `linear-gradient(var(--fl-chat-background-overlay), var(--fl-chat-background-overlay)), url(${url})`;
}
