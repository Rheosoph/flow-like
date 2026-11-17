"use client";

import { type RefObject, useEffect, useMemo, useState } from "react";
import { useBotLabels } from "../../hooks/use-bots";
import { type DenBot, type DenBounds, denCrew } from "../../lib/bot-den/model";
import { DEN_AVOID_ATTRIBUTE } from "../../lib/bot-den/targets";
import type { GooRect } from "../../lib/bot-goo/scene";
import { useHiddenBots } from "../bots-tab/use-bot-visibility";
import {
	describeChatBot,
	useChatBotList,
} from "../global-chat/use-global-chat-bot";

const LAYOUT_POLL_MS = 250;

/** Flux and the builders the profile shows, named as the chat names them. */
export function useDenCrew(): DenBot[] {
	const bots = useChatBotList();
	const hidden = useHiddenBots();
	const labels = useBotLabels();
	return useMemo(() => {
		const list = bots.data ?? [];
		const flux = describeChatBot(null, list, labels, "");
		return denCrew(
			list,
			hidden.keys,
			{ name: flux.name, appearance: flux.appearance },
			(bot) => labels(bot).name || bot.id,
		);
	}, [bots.data, hidden.keys, labels]);
}

export interface DenLayout {
	bounds: DenBounds;
	avoid: GooRect[];
	screenWidth: number;
}

function sameRect(a: GooRect, b: GooRect): boolean {
	return (
		a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height
	);
}

function sameLayout(a: DenLayout | null, b: DenLayout): boolean {
	return (
		a !== null &&
		a.screenWidth === b.screenWidth &&
		a.bounds.right === b.bounds.right &&
		a.bounds.top === b.bounds.top &&
		a.bounds.bottom === b.bounds.bottom &&
		a.avoid.length === b.avoid.length &&
		a.avoid.every((rect, index) => sameRect(rect, b.avoid[index]))
	);
}

function measure(anchor: HTMLElement | null): DenLayout {
	const width = window.innerWidth;
	const height = window.innerHeight;
	const box = anchor?.getBoundingClientRect();
	const bounds =
		box && box.width > 0 && box.height > 0
			? {
					right: Math.round(Math.min(box.right, width)),
					top: Math.round(Math.max(box.top, 0)),
					bottom: Math.round(Math.min(box.bottom, height)),
				}
			: { right: width, top: 0, bottom: height };
	const avoid = [
		...document.querySelectorAll(`[${DEN_AVOID_ATTRIBUTE}]`),
	].flatMap((element) => {
		const rect = element.getBoundingClientRect();
		return rect.width > 0
			? [
					{
						x: Math.round(rect.x),
						y: Math.round(rect.y),
						width: Math.round(rect.width),
						height: Math.round(rect.height),
					},
				]
			: [];
	});
	return { bounds, avoid, screenWidth: width };
}

/**
 * Where the den may sit: the right edge of its surface's anchor, and what covers that edge. The
 * anchor's panels resize without a window resize, so it is measured on a short interval too.
 */
export function useDenLayout(
	anchor: RefObject<HTMLElement | null> | null,
): DenLayout | null {
	const [layout, setLayout] = useState<DenLayout | null>(null);
	useEffect(() => {
		const update = () => {
			const next = measure(anchor?.current ?? null);
			setLayout((current) => (sameLayout(current, next) ? current : next));
		};
		update();
		const timer = window.setInterval(update, LAYOUT_POLL_MS);
		window.addEventListener("resize", update);
		const element = anchor?.current;
		const observer =
			element && typeof ResizeObserver !== "undefined"
				? new ResizeObserver(update)
				: null;
		if (element) observer?.observe(element);
		return () => {
			window.clearInterval(timer);
			window.removeEventListener("resize", update);
			observer?.disconnect();
		};
	}, [anchor]);
	return layout;
}

/** Whether the app shows its dark theme: the den's goo lightens a little to stay visible there. */
export function useDarkTheme(): boolean {
	const [dark, setDark] = useState(false);
	useEffect(() => {
		const root = document.documentElement;
		const update = () => setDark(root.classList.contains("dark"));
		update();
		const observer = new MutationObserver(update);
		observer.observe(root, { attributes: true, attributeFilter: ["class"] });
		return () => observer.disconnect();
	}, []);
	return dark;
}
