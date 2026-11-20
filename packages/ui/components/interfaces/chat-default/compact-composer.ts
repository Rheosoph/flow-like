"use client";

import { useIsMobile } from "../../../hooks/use-mobile";
import { useTouchOnly } from "./message-touch-menu";

/** A phone: the composer keeps to one row and the chat's footer to one line. */
export function useCompactComposer(): boolean {
	const touch = useTouchOnly();
	const narrow = useIsMobile();
	return touch && narrow;
}
