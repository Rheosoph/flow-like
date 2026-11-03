// @vitest-environment happy-dom

import { StrictMode, act, useRef } from "react";
import { type Root, createRoot } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BOT_TIMING } from "./animation";
import {
	type UseBotAnimationOptions,
	useBotAnimation,
} from "./use-bot-animation";

type Options = Omit<UseBotAnimationOptions, "elementRef">;
type Rig = ReturnType<typeof useBotAnimation>;

class MotionPreference extends EventTarget {
	matches = false;
}

class VisibilityObserver {
	static latest: VisibilityObserver;
	target: Element | undefined;
	disconnect = vi.fn();
	constructor(private callback: IntersectionObserverCallback) {
		VisibilityObserver.latest = this;
	}
	observe(target: Element) {
		this.target = target;
	}
	setVisible(isIntersecting: boolean) {
		this.callback(
			[{ target: this.target, isIntersecting } as IntersectionObserverEntry],
			this as unknown as IntersectionObserver,
		);
	}
}

let container: HTMLDivElement;
let root: Root;
let media: MotionPreference;
let hidden: boolean;
let mounted: boolean;

function Probe(options: Options) {
	const elementRef = useRef<HTMLDivElement>(null);
	const rig = useBotAnimation({ ...options, elementRef });
	return (
		<div ref={elementRef}>
			<output>{JSON.stringify(rig)}</output>
		</div>
	);
}

function state(): Rig {
	return JSON.parse(container.querySelector("output")?.textContent ?? "{}");
}

async function render(options: Options = {}) {
	await act(async () =>
		root.render(
			<StrictMode>
				<Probe {...options} />
			</StrictMode>,
		),
	);
}

async function advance(ms: number) {
	await act(async () => vi.advanceTimersByTime(ms));
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.spyOn(Math, "random").mockReturnValue(0);
	media = new MotionPreference();
	vi.stubGlobal(
		"matchMedia",
		vi.fn(() => media),
	);
	vi.stubGlobal("IntersectionObserver", VisibilityObserver);
	hidden = false;
	vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	mounted = true;
});

afterEach(async () => {
	if (mounted) await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

describe("bot animation lifecycle", () => {
	it("reacts once to a changed expression without replaying on rerender", async () => {
		await render({ expression: "happy" });
		expect(state().reaction).toBeUndefined();
		await render({ expression: "curious" });
		expect(state().reaction).toBe("curious");
		await advance(300);
		await render({ expression: "curious", gx: 0.5 });
		await advance(BOT_TIMING.reactionMs - 300);
		expect(state().reaction).toBeUndefined();
		await render({ expression: "surprised" });
		expect(state().reaction).toBe("surprised");
		await render({ expression: "neutral" });
		expect(state().reaction).toBeUndefined();
		expect(vi.getTimerCount()).toBe(4);
	});

	it("cancels reactions when hidden or offscreen and does not replay on return", async () => {
		await render();
		await render({ expression: "love" });
		expect(state().reaction).toBe("love");
		await act(async () => VisibilityObserver.latest.setVisible(false));
		expect(state().reaction).toBeUndefined();
		expect(vi.getTimerCount()).toBe(0);
		await render({ expression: "wink" });
		await act(async () => VisibilityObserver.latest.setVisible(true));
		expect(state().reaction).toBeUndefined();
		await render({ expression: "happy" });
		expect(state().reaction).toBe("happy");
		await act(async () => {
			hidden = true;
			document.dispatchEvent(new Event("visibilitychange"));
		});
		expect(state().reaction).toBeUndefined();
		expect(vi.getTimerCount()).toBe(0);
		await act(async () => {
			hidden = false;
			document.dispatchEvent(new Event("visibilitychange"));
		});
		expect(state().reaction).toBeUndefined();
	});

	it("respects reduced motion and static or sleeping avatars during expression changes", async () => {
		await render();
		await render({ expression: "love" });
		await act(async () => {
			media.matches = true;
			media.dispatchEvent(new Event("change"));
		});
		expect(state().reaction).toBeUndefined();
		expect(vi.getTimerCount()).toBe(0);
		await render({ expression: "surprised" });
		expect(state().reaction).toBeUndefined();
		await act(async () => {
			media.matches = false;
			media.dispatchEvent(new Event("change"));
		});
		expect(state().reaction).toBeUndefined();
		await render({ expression: "angry", animated: false });
		expect(state().reaction).toBeUndefined();
		await render({ expression: "sad", pose: "sleep" });
		expect(state().reaction).toBeUndefined();
		expect(vi.getTimerCount()).toBe(0);
	});

	it.each(["sad", "concerned", "angry", "oops", "focused"] as const)(
		"keeps a %s bot from doing cheerful idle hops",
		async (expression) => {
			await render({ expression });
			await advance(BOT_TIMING.hopIntervalMs[0]);
			expect(state().hopping).toBe(false);
			await render({ expression: "neutral" });
			expect(state().hopping).toBe(true);
		},
	);

	it("keeps static gallery thumbnails free of motion subscriptions and timers", async () => {
		await render({ animated: false });
		expect(window.matchMedia).not.toHaveBeenCalled();
		expect(vi.getTimerCount()).toBe(0);
		await render({ animated: true });
		expect(window.matchMedia).toHaveBeenCalled();
		expect(vi.getTimerCount()).toBe(4);
	});

	it("runs one set of idle timers under StrictMode, with brief blinks, hops and emotes", async () => {
		await render();
		expect(state().motionEnabled).toBe(true);
		expect(vi.getTimerCount()).toBe(4);
		await advance(600);
		expect(state().gaze).toEqual({ x: -3.5, y: -3 });
		await advance(3000);
		expect(state().blinking).toBe(true);
		await advance(150);
		expect(state().blinking).toBe(false);
		await advance(1050);
		expect(state().emoteVisible).toBe(true);
		await advance(1700);
		expect(state().hopping).toBe(true);
		await advance(100);
		expect(state().emoteVisible).toBe(false);
		await advance(900);
		expect(state().hopping).toBe(false);
	});

	it("cancels idle work when motion is disabled and keeps an emote still", async () => {
		await render();
		await advance(3600);
		expect(state().blinking).toBe(true);
		await render({ animated: false });
		expect(state()).toEqual({
			gaze: { x: 0, y: 0 },
			blinking: false,
			hopping: false,
			emoteVisible: true,
			motionEnabled: false,
		});
		expect(vi.getTimerCount()).toBe(0);
		await render();
		expect(vi.getTimerCount()).toBe(4);
		await act(async () => {
			media.matches = true;
			media.dispatchEvent(new Event("change"));
		});
		expect(state().motionEnabled).toBe(false);
		expect(state().emoteVisible).toBe(true);
		expect(vi.getTimerCount()).toBe(0);
	});

	it("stays still when reduced motion is already enabled on mount", async () => {
		media.matches = true;
		await render();
		expect(state().motionEnabled).toBe(false);
		expect(state().emoteVisible).toBe(true);
		expect(vi.getTimerCount()).toBe(0);
	});

	it("pauses offscreen and hidden bots and clears timers and observation on unmount", async () => {
		await render();
		const observer = VisibilityObserver.latest;
		await act(async () => observer.setVisible(false));
		expect(vi.getTimerCount()).toBe(0);
		await act(async () => observer.setVisible(true));
		expect(vi.getTimerCount()).toBe(4);
		await act(async () => {
			hidden = true;
			document.dispatchEvent(new Event("visibilitychange"));
		});
		expect(state().motionEnabled).toBe(false);
		expect(vi.getTimerCount()).toBe(0);
		await act(async () => {
			hidden = false;
			document.dispatchEvent(new Event("visibilitychange"));
		});
		expect(vi.getTimerCount()).toBe(4);
		await act(async () => root.unmount());
		mounted = false;
		expect(vi.getTimerCount()).toBe(0);
		expect(observer.disconnect).toHaveBeenCalledOnce();
	});

	it("does not schedule idle actions for fixed poses and honors controlled gaze", async () => {
		await render({ pose: "left" });
		expect(state().gaze).toEqual({ x: -3.5, y: 0 });
		expect(vi.getTimerCount()).toBe(0);
		await render({ pose: "sleep" });
		expect(state().blinking).toBe(true);
		await render({ pose: "track", gx: 4, gy: -0.5 });
		expect(state().gaze).toEqual({ x: 3.5, y: -1.5 });
		expect(vi.getTimerCount()).toBe(1);
		await render({ pose: "track", gx: -1, animated: false });
		expect(state().gaze).toEqual({ x: -3.5, y: 0 });
		expect(vi.getTimerCount()).toBe(0);
	});

	it("batches pointer updates, ignores touch and removes pending frames on a pose change", async () => {
		vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
			left: 0,
			top: 0,
			width: 100,
			height: 100,
		} as DOMRect);
		await render({ pose: "track" });
		await act(async () => {
			window.dispatchEvent(
				new PointerEvent("pointermove", {
					clientX: 160,
					clientY: 50,
					pointerType: "mouse",
				}),
			);
			window.dispatchEvent(
				new PointerEvent("pointermove", {
					clientX: 270,
					clientY: 50,
					pointerType: "mouse",
				}),
			);
		});
		await advance(20);
		expect(state().gaze).toEqual({ x: 3.5, y: 0 });
		await act(async () =>
			window.dispatchEvent(
				new PointerEvent("pointermove", {
					clientX: -100,
					clientY: 50,
					pointerType: "touch",
				}),
			),
		);
		await advance(20);
		expect(state().gaze).toEqual({ x: 3.5, y: 0 });
		await act(async () => window.dispatchEvent(new Event("blur")));
		expect(state().gaze).toEqual({ x: 0, y: 0 });
		await act(async () =>
			window.dispatchEvent(
				new PointerEvent("pointermove", {
					clientX: 160,
					clientY: 50,
					pointerType: "mouse",
				}),
			),
		);
		await render({ pose: "center" });
		expect(vi.getTimerCount()).toBe(0);
		await advance(1000);
		expect(state().gaze).toEqual({ x: 0, y: 0 });
	});

	it("renders a stationary initial state on the server", () => {
		const markup = renderToString(<Probe />);
		expect(markup).toContain("&quot;motionEnabled&quot;:false");
		expect(markup).toContain("&quot;emoteVisible&quot;:true");
		expect(vi.getTimerCount()).toBe(0);
	});
});
