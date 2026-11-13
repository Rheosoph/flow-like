import { expect, test } from "bun:test";
import type { StreamedReply } from "./bot-call-speech";
import { followPushedReplies } from "./pushed-reply-speech";
import type { ReplyStream } from "./reply-streams";

function manualTimers() {
	const pending = new Map<number, { callback: () => void; ms: number }>();
	let next = 0;
	return {
		timers: {
			set: (callback: () => void, ms: number) => {
				pending.set(++next, { callback, ms });
				return next;
			},
			clear: (handle: unknown) => {
				pending.delete(handle as number);
			},
		},
		pending,
		fire() {
			for (const [id, timer] of [...pending]) {
				pending.delete(id);
				timer.callback();
			}
		},
	};
}

function setup(available = true, stallMs?: number) {
	const calls: string[] = [];
	const opened: unknown[][] = [];
	const clock = manualTimers();
	let allow = available;
	const follower = followPushedReplies({
		timers: clock.timers,
		stallMs,
		stream: (...args) => {
			opened.push(args);
			if (!allow) return null;
			const handle: StreamedReply = {
				update: (text) => calls.push(`update:${text}`),
				flush: () => calls.push("flush"),
				end: () => calls.push("end"),
				cancel: () => calls.push("cancel"),
			};
			return handle;
		},
	});
	return {
		follower,
		calls,
		opened,
		clock,
		allow: (value: boolean) => {
			allow = value;
		},
	};
}

function pushed(
	text: string,
	state: ReplyStream["state"],
	messageId = "re-1",
	draft = 0,
): ReplyStream {
	return {
		conversationId: "room",
		messageId,
		replyTo: "spoken",
		senderId: "bot",
		draft,
		text,
		state,
	};
}

test("states map to handle calls in order", () => {
	const s = setup();
	s.follower.apply(pushed("Hello", "writing"));
	s.follower.apply(pushed("Hello there", "working"));
	s.follower.apply(pushed("Hello there.", "done"));
	expect(s.opened).toEqual([["spoken", "bot", { messageId: "re-1" }]]);
	expect(s.calls).toEqual([
		"update:Hello",
		"update:Hello there",
		"flush",
		"update:Hello there.",
		"end",
	]);
	expect(s.clock.pending.size).toBe(0);
});

test("a dropped stream cancels its handle without handing it more text", () => {
	const s = setup();
	s.follower.apply(pushed("Hello", "writing"));
	s.follower.apply(pushed("Hello there", "dropped"));
	expect(s.calls).toEqual(["update:Hello", "cancel"]);
	expect(s.clock.pending.size).toBe(0);
});

test("a rewrite opens a new speech stream after its draft was dropped", () => {
	const s = setup();
	s.follower.apply(pushed("Hello", "writing"));
	s.follower.apply(pushed("Hello", "dropped"));
	s.follower.apply(pushed("Anna", "writing", "re-1", 1));
	s.follower.apply(pushed("Anna said it.", "done", "re-1", 1));
	expect(s.opened).toEqual([
		["spoken", "bot", { messageId: "re-1" }],
		["spoken", "bot", { messageId: "re-1" }],
	]);
	expect(s.calls).toEqual([
		"update:Hello",
		"cancel",
		"update:Anna",
		"update:Anna said it.",
		"end",
	]);
	expect(s.clock.pending.size).toBe(0);
});

test("a finished draft and the ones before it are not followed again", () => {
	const s = setup();
	s.follower.apply(pushed("Hello", "dropped"));
	s.follower.apply(pushed("Anna", "writing", "re-1", 2));
	s.follower.apply(pushed("Anna", "dropped", "re-1", 2));
	s.follower.apply(pushed("Anna again", "writing", "re-1", 2));
	s.follower.apply(pushed("Late", "writing", "re-1", 1));
	expect(s.opened).toHaveLength(1);
	expect(s.calls).toEqual(["update:Anna", "cancel"]);
});

test("opening is retried until the stream returns a handle", () => {
	const s = setup(false);
	s.follower.apply(pushed("Hello", "writing"));
	s.follower.apply(pushed("Hello t", "writing"));
	expect(s.opened).toHaveLength(2);
	expect(s.calls).toEqual([]);
	s.allow(true);
	s.follower.apply(pushed("Hello th", "writing"));
	expect(s.opened).toHaveLength(3);
	expect(s.calls).toEqual(["update:Hello th"]);
});

test("nothing opens for empty text or a dropped stream without a handle", () => {
	const s = setup();
	s.follower.apply(pushed("  ", "writing"));
	s.follower.apply(pushed("Hello", "dropped"));
	s.follower.apply(pushed("Hello", "writing"));
	expect(s.opened).toHaveLength(0);
	expect(s.calls).toEqual([]);
});

test("done without a handle is not retried", () => {
	const s = setup(false);
	s.follower.apply(pushed("Hello", "done"));
	s.allow(true);
	s.follower.apply(pushed("Hello", "done"));
	expect(s.opened).toHaveLength(1);
	expect(s.calls).toEqual([]);
});

test("the stall guard ends the handle once and ignores later updates", () => {
	const s = setup(true, 1234);
	s.follower.apply(pushed("Hello", "writing"));
	expect([...s.clock.pending.values()].map((t) => t.ms)).toEqual([1234]);
	s.follower.apply(pushed("Hello there", "writing"));
	expect(s.clock.pending.size).toBe(1);
	s.clock.fire();
	expect(s.calls).toEqual(["update:Hello", "update:Hello there", "end"]);
	s.follower.apply(pushed("Hello there again", "writing"));
	s.follower.apply(pushed("Hello there again", "done"));
	s.clock.fire();
	expect(s.calls).toHaveLength(3);
	expect(s.opened).toHaveLength(1);
});

test("the default stall time is twenty seconds", () => {
	const s = setup();
	s.follower.apply(pushed("Hello", "writing"));
	expect([...s.clock.pending.values()].map((t) => t.ms)).toEqual([20000]);
});

test("updates after done are ignored", () => {
	const s = setup();
	s.follower.apply(pushed("Hello", "done"));
	s.follower.apply(pushed("Hello more", "writing"));
	expect(s.calls).toEqual(["update:Hello", "end"]);
});

test("dispose clears timers without calling the handles", () => {
	const s = setup();
	s.follower.apply(pushed("Hello", "writing"));
	s.follower.apply(pushed("Other", "writing", "re-2"));
	expect(s.clock.pending.size).toBe(2);
	s.follower.dispose();
	expect(s.clock.pending.size).toBe(0);
	s.clock.fire();
	expect(s.calls).toEqual(["update:Hello", "update:Other"]);
});
