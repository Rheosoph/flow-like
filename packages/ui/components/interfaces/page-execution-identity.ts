/**
 * Stable lifecycle identity for a Page execution target.
 *
 * Governed hosted Pages deliberately receive no backing Board id. Their Event
 * id is sufficient because lifecycle calls go through executeEvent. Legacy and
 * local Pages keep using their Board id.
 */
export function pageExecutionIdentity(
	boardId: string | undefined,
	governedEventId: string | undefined,
): string | undefined {
	if (boardId) return `board:${boardId}`;
	if (governedEventId) return `event:${governedEventId}`;
	return undefined;
}
