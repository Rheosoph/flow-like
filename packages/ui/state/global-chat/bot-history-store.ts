import { create } from "zustand";
import type { TranscriptPage } from "./bot-history";

/** Where the bot chat on screen sits in the bot's history. */
export interface BotHistoryView {
	/** The `to_seq` that loads the page before the oldest message shown; null at the start. */
	olderTo: number | null;
	/** True when the screen shows a window around a jump, not the newest messages. */
	windowed: boolean;
	seqs: Record<string, number>;
}

/** A "New topic" marker written in this session, listed before its summary exists. */
export interface NewTopicMark {
	seq: number;
	title: string | null;
	at: number;
}

interface BotHistoryState {
	views: Record<string, BotHistoryView>;
	topics: Record<string, NewTopicMark[]>;
	noteTopic: (botId: string, mark: NewTopicMark) => void;
	show: (botId: string, page: TranscriptPage, windowed: boolean) => void;
	prepend: (botId: string, page: TranscriptPage) => void;
	note: (botId: string, messageId: string, seq: number) => void;
}

export const useBotHistoryStore = create<BotHistoryState>((set) => ({
	views: {},
	topics: {},
	noteTopic: (botId, mark) =>
		set((state) => ({
			topics: {
				...state.topics,
				[botId]: [...(state.topics[botId] ?? []), mark],
			},
		})),
	show: (botId, page, windowed) =>
		set((state) => ({
			views: {
				...state.views,
				[botId]: { olderTo: page.olderTo, windowed, seqs: page.seqs },
			},
		})),
	prepend: (botId, page) =>
		set((state) => {
			const view = state.views[botId];
			return {
				views: {
					...state.views,
					[botId]: {
						olderTo: page.olderTo,
						windowed: view?.windowed ?? false,
						seqs: { ...page.seqs, ...view?.seqs },
					},
				},
			};
		}),
	note: (botId, messageId, seq) =>
		set((state) => {
			const view = state.views[botId];
			if (!view) return state;
			return {
				views: {
					...state.views,
					[botId]: { ...view, seqs: { ...view.seqs, [messageId]: seq } },
				},
			};
		}),
}));
