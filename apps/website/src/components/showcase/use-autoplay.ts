import { type RefObject, useEffect, useRef, useState } from "react";

export type TimelineStep =
	| { kind: "wait"; ms: number }
	| { kind: "reset" }
	| { kind: "type"; text: string; cps?: number }
	| { kind: "send" }
	| { kind: "stream"; text: string; cps?: number }
	| {
			kind: "focusNodes";
			ids: string[];
			padding?: number;
			duration?: number;
			hold?: number;
	  }
	| { kind: "fitAll"; duration?: number; hold?: number }
	| { kind: "setData"; data: unknown }
	| { kind: "callout"; text: string; ms: number };

export interface ShowcaseDriver {
	chat?: {
		setInput(text: string): void;
		focus(): void;
		send(): void;
		reset(): void;
		beginStream(): void;
		pushStreamChunk(full: string): void;
		endStream(full: string): void;
	};
	board?: {
		focus(ids: string[], padding?: number, duration?: number): void;
		fitAll(duration?: number): void;
	};
	data?: {
		set(model: unknown): void;
	};
	onCallout?(text: string | null): void;
}

const CANCELLED = Symbol("autoplay-cancelled");

/**
 * Runs a guided timeline against a variant-supplied driver. Gating mirrors the
 * hand-rolled hero-v4 engine: only plays while in view + tab visible, pauses
 * permanently on visitor interaction, and collapses to a single instant pass
 * (no loop) under prefers-reduced-motion.
 */
export function useAutoplay(
	rootRef: RefObject<HTMLElement | null>,
	steps: TimelineStep[] | undefined,
	driver: ShowcaseDriver | null,
	opts?: Readonly<{ loopDelayMs?: number; rootMargin?: string }>,
): { inView: boolean; userPaused: boolean } {
	const [inView, setInView] = useState(false);
	const [userPaused, setUserPaused] = useState(false);

	const state = useRef({
		inView: false,
		hidden: false,
		userPaused: false,
		reduced: false,
		cancelled: false,
	});
	const waiters = useRef(new Set<() => void>());
	const started = useRef(false);

	const loopDelayMs = opts?.loopDelayMs ?? 3200;
	const rootMargin = opts?.rootMargin ?? "0px 0px -15% 0px";

	useEffect(() => {
		const root = rootRef.current;
		if (!root || !steps?.length || !driver) return;

		const ctrl = state.current;
		ctrl.cancelled = false;
		const notify = () => {
			for (const w of waiters.current) w();
		};

		const media =
			typeof window !== "undefined" && window.matchMedia
				? window.matchMedia("(prefers-reduced-motion: reduce)")
				: null;
		ctrl.reduced = media?.matches ?? false;
		const onReduced = (e: MediaQueryListEvent) => {
			ctrl.reduced = e.matches;
			notify();
		};
		media?.addEventListener?.("change", onReduced);

		const runnable = () =>
			ctrl.inView && !ctrl.hidden && !ctrl.userPaused && !ctrl.cancelled;

		const gate = () =>
			new Promise<void>((res, rej) => {
				if (ctrl.cancelled) return rej(CANCELLED);
				if (ctrl.reduced || runnable()) return res();
				const check = () => {
					if (ctrl.cancelled) {
						waiters.current.delete(check);
						rej(CANCELLED);
					} else if (ctrl.reduced || runnable()) {
						waiters.current.delete(check);
						res();
					}
				};
				waiters.current.add(check);
			});

		const sleep = (ms: number) =>
			new Promise<void>((res, rej) => {
				if (ctrl.reduced) return res();
				if (ctrl.cancelled) return rej(CANCELLED);
				const timer = setTimeout(() => {
					waiters.current.delete(onCancel);
					res();
				}, ms);
				const onCancel = () => {
					if (ctrl.cancelled) {
						clearTimeout(timer);
						waiters.current.delete(onCancel);
						rej(CANCELLED);
					}
				};
				waiters.current.add(onCancel);
			});

		const typeText = async (text: string, cps: number) => {
			// Do not focus the composer from autoplay: focusin is deliberately
			// treated as visitor takeover and would pause the script itself.
			if (ctrl.reduced) {
				driver.chat?.setInput(text);
				return;
			}
			for (let i = 1; i <= text.length; i++) {
				await gate();
				driver.chat?.setInput(text.slice(0, i));
				await sleep(1000 / cps);
			}
		};

		const streamText = async (text: string, cps: number) => {
			driver.chat?.beginStream();
			if (ctrl.reduced) {
				driver.chat?.pushStreamChunk(text);
				driver.chat?.endStream(text);
				return;
			}
			const tokens = text.match(/\S+\s*/g) ?? [text];
			let shown = "";
			for (const tok of tokens) {
				await gate();
				shown += tok;
				driver.chat?.pushStreamChunk(shown);
				await sleep(Math.max(1000 / cps, 18) * (0.5 + tok.length * 0.35));
			}
			driver.chat?.endStream(text);
		};

		const play = async () => {
			try {
				do {
					for (const s of steps) {
						await gate();
						switch (s.kind) {
							case "wait":
								await sleep(s.ms);
								break;
							case "reset":
								driver.chat?.reset();
								break;
							case "type":
								await typeText(s.text, s.cps ?? 26);
								break;
							case "send":
								driver.chat?.send();
								break;
							case "stream":
								await streamText(s.text, s.cps ?? 42);
								break;
							case "focusNodes":
								driver.board?.focus(s.ids, s.padding, s.duration);
								await sleep(s.hold ?? 1600);
								break;
							case "fitAll":
								driver.board?.fitAll(s.duration);
								await sleep(s.hold ?? 1400);
								break;
							case "setData":
								driver.data?.set(s.data);
								break;
							case "callout":
								driver.onCallout?.(s.text);
								await sleep(s.ms);
								driver.onCallout?.(null);
								break;
						}
					}
					await sleep(loopDelayMs);
				} while (!ctrl.reduced && !ctrl.cancelled);
			} catch (err) {
				if (err !== CANCELLED) throw err;
			}
		};

		const observer = new IntersectionObserver(
			([entry]) => {
				ctrl.inView = entry.isIntersecting;
				setInView(entry.isIntersecting);
				if (entry.isIntersecting && !started.current) {
					started.current = true;
					void play();
				}
				notify();
			},
			{ rootMargin, threshold: 0.01 },
		);
		observer.observe(root);

		const pause = () => {
			if (ctrl.userPaused) return;
			ctrl.userPaused = true;
			setUserPaused(true);
			notify();
		};
		const onVisibility = () => {
			ctrl.hidden = document.hidden;
			notify();
		};

		root.addEventListener("pointerdown", pause);
		root.addEventListener("keydown", pause);
		root.addEventListener("focusin", pause);
		document.addEventListener("visibilitychange", onVisibility);

		return () => {
			ctrl.cancelled = true;
			notify();
			observer.disconnect();
			media?.removeEventListener?.("change", onReduced);
			root.removeEventListener("pointerdown", pause);
			root.removeEventListener("keydown", pause);
			root.removeEventListener("focusin", pause);
			document.removeEventListener("visibilitychange", onVisibility);
			started.current = false;
		};
	}, [rootRef, steps, driver, loopDelayMs, rootMargin]);

	return { inView, userPaused };
}
