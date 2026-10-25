"use client";

import { useEffect, useState } from "react";

const expandedByCard = new Map<string, boolean>();

/**
 * Expanded state of an inline app card that outlives one mount. A card anchored inside a reply
 * remounts when that reply moves from the live bubble into the settled transcript; without this it
 * would replay its entrance animation (`remounted`) and forget whether the user collapsed it.
 */
export function useInlineCardExpanded(cardId: string, initial: boolean) {
	const [remounted] = useState(() => expandedByCard.has(cardId));
	const [expanded, setExpanded] = useState(
		() => expandedByCard.get(cardId) ?? initial,
	);
	useEffect(() => {
		expandedByCard.set(cardId, expanded);
	}, [cardId, expanded]);
	return { expanded, setExpanded, remounted };
}
