/** The y-webrtc room every member of an account conversation shares. */
export function conversationRealtimeRoom(conversationId: string): string {
	return `inbox:${conversationId}:room-v1`;
}
