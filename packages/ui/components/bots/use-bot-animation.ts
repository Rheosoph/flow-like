"use client";

import { type RefObject, useEffect, useState } from "react";
import {
	BOT_TIMING,
	type BotGaze,
	clampGaze,
	getPointerGaze,
	getPoseGaze,
} from "./animation";
import type { BotPose } from "./types";

export interface UseBotAnimationOptions {
	pose?: BotPose;
	gx?: number;
	gy?: number;
	animated?: boolean;
	elementRef: RefObject<HTMLElement | null>;
}

interface RigState {
	gaze: BotGaze;
	blinking: boolean;
	hopping: boolean;
	emoteVisible: boolean;
}

const REST: RigState = {
	gaze: { x: 0, y: 0 },
	blinking: false,
	hopping: false,
	emoteVisible: false,
};

function between([min, max]: readonly [number, number]): number {
	return min + Math.random() * (max - min);
}

export function useBotAnimation({
	pose = "idle",
	gx,
	gy,
	animated = true,
	elementRef,
}: UseBotAnimationOptions) {
	// Start still on the server and during hydration until browser preferences are known.
	const [canAnimate, setCanAnimate] = useState(false);
	const [rig, setRig] = useState<RigState>(REST);
	const [pointerGaze, setPointerGaze] = useState<BotGaze>(REST.gaze);
	const motionEnabled = animated && canAnimate;

	useEffect(() => {
		if (!animated) {
			setCanAnimate(false);
			return;
		}
		const media = window.matchMedia?.("(prefers-reduced-motion: reduce)");
		let inView = true;
		const update = () =>
			setCanAnimate(!media?.matches && !document.hidden && inView);
		const observer =
			typeof IntersectionObserver === "undefined"
				? undefined
				: new IntersectionObserver((entries) => {
						const entry = entries.find(
							(entry) => entry.target === elementRef.current,
						);
						if (entry) {
							inView = entry.isIntersecting;
							update();
						}
					});
		if (elementRef.current) observer?.observe(elementRef.current);
		media?.addEventListener("change", update);
		document.addEventListener("visibilitychange", update);
		update();
		return () => {
			observer?.disconnect();
			media?.removeEventListener("change", update);
			document.removeEventListener("visibilitychange", update);
		};
	}, [elementRef, animated]);

	useEffect(() => {
		setRig(REST);
		if (!motionEnabled || (pose !== "idle" && pose !== "track")) return;
		const timers = new Set<ReturnType<typeof setTimeout>>();
		let disposed = false;
		const later = (callback: () => void, delay: number) => {
			const timer = setTimeout(() => {
				timers.delete(timer);
				if (!disposed) callback();
			}, delay);
			timers.add(timer);
		};
		const repeat = (
			callback: () => void,
			interval: readonly [number, number],
		) => {
			const run = () => {
				callback();
				later(run, between(interval));
			};
			later(run, between(interval));
		};
		repeat(() => {
			setRig((current) => ({ ...current, blinking: true }));
			later(
				() => setRig((current) => ({ ...current, blinking: false })),
				BOT_TIMING.blinkMs,
			);
		}, BOT_TIMING.blinkIntervalMs);

		if (pose === "idle") {
			repeat(
				() =>
					setRig((current) => ({
						...current,
						gaze: clampGaze(Math.random() * 2 - 1, Math.random() * 2 - 1),
					})),
				BOT_TIMING.gazeHoldMs,
			);
			repeat(() => {
				setRig((current) => ({ ...current, hopping: true }));
				later(
					() => setRig((current) => ({ ...current, hopping: false })),
					BOT_TIMING.hopMs,
				);
			}, BOT_TIMING.hopIntervalMs);
			repeat(() => {
				setRig((current) => ({ ...current, emoteVisible: true }));
				later(
					() => setRig((current) => ({ ...current, emoteVisible: false })),
					BOT_TIMING.emotePopMs + BOT_TIMING.emoteHoldMs,
				);
			}, BOT_TIMING.emoteIntervalMs);
		}

		return () => {
			disposed = true;
			for (const timer of timers) clearTimeout(timer);
		};
	}, [motionEnabled, pose]);

	const controlledGaze = gx !== undefined || gy !== undefined;
	useEffect(() => {
		setPointerGaze(REST.gaze);
		if (!motionEnabled || pose !== "track" || controlledGaze) return;
		let pendingFrame: number | undefined;
		let pointer: { x: number; y: number } | undefined;
		const move = (event: PointerEvent) => {
			if (event.pointerType === "touch") return;
			pointer = { x: event.clientX, y: event.clientY };
			if (pendingFrame !== undefined) return;
			pendingFrame = requestAnimationFrame(() => {
				pendingFrame = undefined;
				const element = elementRef.current;
				if (element && pointer)
					setPointerGaze(
						getPointerGaze(pointer, element.getBoundingClientRect()),
					);
			});
		};
		const leave = () => {
			if (pendingFrame !== undefined) cancelAnimationFrame(pendingFrame);
			pendingFrame = undefined;
			pointer = undefined;
			setPointerGaze(REST.gaze);
		};
		window.addEventListener("pointermove", move, { passive: true });
		window.addEventListener("blur", leave);
		document.documentElement.addEventListener("pointerleave", leave);
		return () => {
			if (pendingFrame !== undefined) cancelAnimationFrame(pendingFrame);
			window.removeEventListener("pointermove", move);
			window.removeEventListener("blur", leave);
			document.documentElement.removeEventListener("pointerleave", leave);
		};
	}, [motionEnabled, pose, controlledGaze, elementRef]);

	const gaze =
		motionEnabled && pose === "idle"
			? rig.gaze
			: motionEnabled && pose === "track" && !controlledGaze
				? pointerGaze
				: getPoseGaze(pose, gx, gy);

	return {
		gaze,
		blinking:
			pose === "blink" ||
			pose === "sleep" ||
			(motionEnabled && (pose === "idle" || pose === "track") && rig.blinking),
		hopping: motionEnabled && pose === "idle" && rig.hopping,
		emoteVisible: !motionEnabled || pose !== "idle" || rig.emoteVisible,
		motionEnabled,
	};
}
