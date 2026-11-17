"use client";

import { useCallback, useEffect, useState } from "react";
import { notchCommands } from "../../lib/bot-notch/commands";
import { perchAnchor } from "../../lib/bot-notch/model";
import type {
	NotchTab,
	NotchWindow,
	PerchFrame,
	PerchSpec,
} from "../../lib/bot-notch/protocol";

export interface Perch {
	botId: string;
	window: NotchWindow;
	/** The tab it was dropped on; undefined while the browser is still asked. */
	tab: NotchTab | null | undefined;
	anchorX: number;
}

function specOf(perch: Perch): PerchSpec {
	return {
		botId: perch.botId,
		windowId: perch.window.id,
		anchorX: perch.anchorX,
		tab: perch.tab ?? null,
	};
}

/** Which bot sits on which window, mirrored to the tracker in Rust. */
export function usePerches(
	shown: ReadonlySet<string>,
	frames: ReadonlyMap<string, PerchFrame>,
) {
	const [perches, setPerches] = useState<ReadonlyMap<string, Perch>>(
		() => new Map(),
	);

	useEffect(() => {
		void notchCommands
			.setPerches([...perches.values()].map(specOf))
			.catch((error) =>
				console.warn("[BotNotch] The perches were not tracked:", error),
			);
	}, [perches]);

	useEffect(() => {
		setPerches((current) => {
			let next: Map<string, Perch> | null = null;
			for (const [botId, seat] of current) {
				const frame = frames.get(botId);
				const gone = frame?.windowId === seat.window.id && frame.gone;
				if (!shown.has(botId) || gone) {
					next ??= new Map(current);
					next.delete(botId);
				}
			}
			return next ?? current;
		});
	}, [shown, frames]);

	const perch = useCallback(
		(botId: string, window: NotchWindow, dropX: number) => {
			setPerches((current) => {
				const taken = [...current.values()]
					.filter(
						(other) => other.window.id === window.id && other.botId !== botId,
					)
					.map((other) => other.anchorX);
				const next = new Map(current);
				next.set(botId, {
					botId,
					window,
					tab: window.browser ? undefined : null,
					anchorX: perchAnchor(window.bounds, dropX, taken),
				});
				return next;
			});
			if (!window.browser) return;
			notchCommands
				.activeTab(window.id)
				.catch(() => null)
				.then((tab) =>
					setPerches((current) => {
						const perch = current.get(botId);
						if (perch?.window.id !== window.id || perch.tab !== undefined)
							return current;
						const next = new Map(current);
						next.set(botId, {
							...perch,
							tab: tab ?? { url: null, title: window.title },
						});
						return next;
					}),
				);
		},
		[],
	);

	const unperch = useCallback((botId: string) => {
		setPerches((current) => {
			if (!current.has(botId)) return current;
			const next = new Map(current);
			next.delete(botId);
			return next;
		});
	}, []);

	return { perches, perch, unperch };
}
