const EVENT_TYPE_LABELS: Readonly<Record<string, string>> = {
	simple_chat: "Chat UI",
	teams: "Teams Bot",
	geolocation: "Location Region",
	inbound_email: "Inbound Email",
};

export function formatEventTypeLabel(eventType: string): string {
	return (
		EVENT_TYPE_LABELS[eventType] ??
		eventType
			.replace(/_/g, " ")
			.replace(/\b\w/g, (character) => character.toUpperCase())
	);
}
