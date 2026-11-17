import { type RefObject, useEffect } from "react";
import { create } from "zustand";

/** A surface that wants the bot den; the den hugs the right edge of `anchor`, else the window's. */
export interface BotDenRequest {
	id: number;
	anchor: RefObject<HTMLElement | null> | null;
}

interface BotDenState {
	requests: readonly BotDenRequest[];
	/** Adds a request and returns its release. */
	acquire: (anchor: RefObject<HTMLElement | null> | null) => () => void;
}

let nextRequest = 0;

/**
 * The den is opt-in: only the surfaces bots can act on ask for it (the board, the widget and page
 * builder, app settings, Data Studio, Home). The newest request decides where it sits.
 */
export const useBotDenStore = create<BotDenState>((set) => ({
	requests: [],
	acquire: (anchor) => {
		const request: BotDenRequest = { id: ++nextRequest, anchor };
		set((state) => ({ requests: [...state.requests, request] }));
		return () =>
			set((state) => ({
				requests: state.requests.filter((item) => item.id !== request.id),
			}));
	},
}));

/** Shows the bot den while the calling component is mounted and `active`. */
export function useRequestBotDen(
	active = true,
	anchor: RefObject<HTMLElement | null> | null = null,
): void {
	useEffect(() => {
		if (!active) return;
		return useBotDenStore.getState().acquire(anchor);
	}, [active, anchor]);
}

/** The request the den follows, or null while no surface wants it. */
export function useBotDenRequest(): BotDenRequest | null {
	return useBotDenStore((state) => state.requests.at(-1) ?? null);
}
