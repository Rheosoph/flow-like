const EVENT_TYPE_LABELS: Readonly<Record<string, string>> = {
	simple_chat: "Chat UI",
};

export function formatEventTypeLabel(eventType: string): string {
	return (
		EVENT_TYPE_LABELS[eventType] ??
		eventType
			.replace(/_/g, " ")
			.replace(/\b\w/g, (character) => character.toUpperCase())
	);
}
