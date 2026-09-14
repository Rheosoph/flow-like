import { beforeEach, expect, test } from "bun:test";
import { useGlobalChatStore } from "./global-chat-store";

const native = (id: string) => ({
	prompt: `Question ${id}`,
	nativeRequest: { id, scope: "workspace", responseMode: "text" as const },
});
beforeEach(() =>
	useGlobalChatStore.setState({ draft: null, nativeDrafts: [], queue: [] }),
);

test("accepted native questions remain FIFO while model readiness is pending", () => {
	const store = useGlobalChatStore.getState();
	store.setDraft(native("one"));
	store.setDraft(native("two"));
	store.setDraft(native("three"));
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("one");
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("two");
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("three");
	expect(store.consumeDraft()).toBeNull();
});

test("replaying a queued native request does not enqueue it twice", () => {
	const store = useGlobalChatStore.getState();
	store.setDraft(native("one"));
	store.setDraft(native("two"));
	store.setDraft(native("one"));
	store.setDraft(native("two"));
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("one");
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("two");
	expect(store.consumeDraft()).toBeNull();
});

test("ordinary hero replacements preserve already accepted native questions", () => {
	const store = useGlobalChatStore.getState();
	store.setDraft(native("one"));
	store.setDraft({ prompt: "First hero" });
	store.setDraft({ prompt: "Edited hero" });
	store.setDraft(native("two"));
	expect(store.consumeDraft()?.prompt).toBe("Edited hero");
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("one");
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("two");
});

test("pending questions are bounded without dropping accepted requests", () => {
	const store = useGlobalChatStore.getState();
	for (let index = 0; index < 32; index++)
		store.setDraft(native(String(index)));
	expect(() => store.setDraft(native("overflow"))).toThrow("Too many pending");
	for (let index = 0; index < 32; index++)
		expect(store.consumeDraft()?.nativeRequest?.id).toBe(String(index));
});

test("capacity queues preserve the native response identity", () => {
	const store = useGlobalChatStore.getState();
	const draft = native("capacity");
	store.enqueueMessage({
		conversationId: "chat",
		content: draft.prompt,
		nativeRequest: draft.nativeRequest,
	});
	expect(store.takeNextQueuedMessage("chat")?.nativeRequest).toEqual(
		draft.nativeRequest,
	);
});

test("a stale page or overlay consumer cannot remove the next pending native request", () => {
	const store = useGlobalChatStore.getState();
	const first = native("expired");
	store.setDraft(first);
	store.setDraft(native("next"));
	expect(store.consumeDraft(first)).toBe(first);
	expect(store.consumeDraft(first)).toBeNull();
	expect(store.consumeDraft()?.nativeRequest?.id).toBe("next");
});
