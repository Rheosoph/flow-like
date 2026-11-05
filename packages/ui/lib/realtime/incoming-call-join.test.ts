import { afterEach, beforeEach, expect, test, vi } from "vitest";
import {
	clearIncomingCallJoin,
	getIncomingCallJoin,
	getIncomingCallScope,
	registerIncomingCallScope,
	requestIncomingCallJoin,
	takeIncomingCallJoin,
} from "./incoming-call-join";

const request = {
	scope: "hub:user",
	userId: "user",
	conversationId: "conversation",
	callId: "call",
};
let contexts: {
	resume: ReturnType<typeof vi.fn>;
	close: ReturnType<typeof vi.fn>;
}[];

beforeEach(() => {
	vi.useFakeTimers();
	contexts = [];
	vi.stubGlobal(
		"AudioContext",
		class {
			resume = vi.fn(async () => {});
			close = vi.fn(async () => {});
			constructor() {
				contexts.push(this);
			}
		},
	);
});
afterEach(() => {
	clearIncomingCallJoin();
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

test("only the matching user's live call consumes a Join click, once", () => {
	requestIncomingCallJoin(request);
	expect(contexts[0].resume).toHaveBeenCalledOnce();
	expect(
		takeIncomingCallJoin("different-hub", "user", "conversation", "call"),
	).toBeUndefined();
	expect(
		takeIncomingCallJoin("hub:user", "other", "conversation", "call"),
	).toBeUndefined();
	expect(
		takeIncomingCallJoin("hub:user", "user", "other", "call"),
	).toBeUndefined();
	expect(
		takeIncomingCallJoin("hub:user", "user", "conversation", "old-call"),
	).toBeUndefined();
	expect(
		takeIncomingCallJoin("hub:user", "user", "conversation", "call")?.context,
	).toBe(contexts[0]);
	expect(
		takeIncomingCallJoin("hub:user", "user", "conversation", "call"),
	).toBeUndefined();
	expect(contexts[0].close).not.toHaveBeenCalled();
});

test("navigation that never reaches the call expires and releases audio", () => {
	requestIncomingCallJoin(request);
	vi.advanceTimersByTime(30_000);
	expect(getIncomingCallJoin()).toBeUndefined();
	expect(contexts[0].close).toHaveBeenCalledOnce();
});

test("account cleanup cannot remove a different account's newer intent", () => {
	requestIncomingCallJoin(request);
	requestIncomingCallJoin({ ...request, scope: "other-hub:user" });
	expect(contexts[0].close).toHaveBeenCalledOnce();
	clearIncomingCallJoin("hub:user");
	expect(getIncomingCallJoin()?.scope).toBe("other-hub:user");
	clearIncomingCallJoin("other-hub:user");
	expect(getIncomingCallJoin()).toBeUndefined();
	expect(contexts[1].close).toHaveBeenCalledOnce();
});

test("a missing AudioContext does not prevent a requested join", () => {
	vi.stubGlobal("AudioContext", undefined);
	requestIncomingCallJoin(request);
	expect(
		takeIncomingCallJoin("hub:user", "user", "conversation", "call")?.context,
	).toBeNull();
});

test("retiring a shell clears its pending join without removing a newer registration", () => {
	const backend = {};
	const retireFirst = registerIncomingCallScope(backend, "hub:user");
	const retireSecond = registerIncomingCallScope(backend, "next-hub:user");
	retireFirst();
	expect(getIncomingCallScope(backend)).toBe("next-hub:user");
	requestIncomingCallJoin({ ...request, scope: "next-hub:user" });
	retireSecond();
	expect(getIncomingCallScope(backend)).toBeUndefined();
	expect(getIncomingCallJoin()).toBeUndefined();
	expect(contexts[0].close).toHaveBeenCalledOnce();
});
