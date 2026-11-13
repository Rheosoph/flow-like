import { beforeEach, describe, expect, test } from "bun:test";
import {
	type ReplyStream,
	deliverRealtimeReply,
	resetReplyStreams,
	watchReplyStreams,
} from "./reply-streams";
import type { RealtimeReply } from "./types";

const message = (patch: Partial<RealtimeReply> = {}): RealtimeReply => ({
	protocol_version: 1,
	kind: "reply",
	conversation_id: "c1",
	message_id: "re-1",
	reply_to: "t1",
	sender_id: "bot-1",
	offset: 0,
	text: "",
	state: "writing",
	...patch,
});

function watch(conversationId = "c1") {
	const seen: ReplyStream[] = [];
	const stop = watchReplyStreams(conversationId, (reply) => seen.push(reply));
	return { seen, stop, last: () => seen.at(-1) };
}

describe("reply streams", () => {
	beforeEach(() => resetReplyStreams());

	test("a reply grows from whole-text messages and ends with its last one", () => {
		const { seen, last } = watch();
		deliverRealtimeReply(message({ text: "Hello" }));
		deliverRealtimeReply(message({ text: "Hello there." }));
		deliverRealtimeReply(message({ text: "Hello there. Bye.", state: "done" }));
		expect(seen.map((reply) => [reply.text, reply.state])).toEqual([
			["Hello", "writing"],
			["Hello there.", "writing"],
			["Hello there. Bye.", "done"],
		]);
		expect(last()).toMatchObject({
			conversationId: "c1",
			messageId: "re-1",
			replyTo: "t1",
			senderId: "bot-1",
		});
	});

	test("a lost message is made up for by the next one, and a late one changes nothing", () => {
		const { seen, last } = watch();
		deliverRealtimeReply(message({ text: "One." }));
		deliverRealtimeReply(message({ text: "One. Two. Three." }));
		deliverRealtimeReply(message({ text: "One. Two." }));
		expect(seen).toHaveLength(2);
		expect(last()?.text).toBe("One. Two. Three.");
	});

	test("a long reply continues from its tail when the tail reaches what is held", () => {
		const { last } = watch();
		deliverRealtimeReply(message({ text: "abcdef" }));
		deliverRealtimeReply(message({ offset: 4, text: "efghij" }));
		expect(last()?.text).toBe("abcdefghij");
		deliverRealtimeReply(message({ offset: 20, text: "xyz" }));
		expect(last()?.text).toBe("abcdefghij");
		deliverRealtimeReply(message({ offset: 20, text: "xyz", state: "done" }));
		expect(last()).toMatchObject({ text: "abcdefghij", state: "done" });
	});

	test("a reply this device missed the start of is left to the stored message", () => {
		const { seen } = watch();
		deliverRealtimeReply(message({ offset: 5, text: " there." }));
		deliverRealtimeReply(message({ state: "dropped" }));
		expect(seen).toEqual([]);
		deliverRealtimeReply(message({ text: "Hello there.", state: "done" }));
		expect(seen.map((reply) => reply.state)).toEqual(["done"]);
	});

	test("a pause is reported once and writing resumes with new text only", () => {
		const { seen } = watch();
		deliverRealtimeReply(message({ text: "Let me check." }));
		deliverRealtimeReply(message({ text: "Let me check.", state: "working" }));
		deliverRealtimeReply(message({ text: "Let me check.", state: "working" }));
		deliverRealtimeReply(message({ text: "Let me check." }));
		deliverRealtimeReply(message({ text: "Let me check.\n\nIt is five." }));
		expect(seen.map((reply) => reply.state)).toEqual([
			"writing",
			"working",
			"writing",
		]);
	});

	test("nothing follows a posted reply", () => {
		const { seen } = watch();
		deliverRealtimeReply(message({ text: "Done.", state: "done" }));
		deliverRealtimeReply(message({ text: "Done. And more." }));
		deliverRealtimeReply(message({ state: "dropped" }));
		expect(seen).toHaveLength(1);
	});

	test("a dropped reply ignores late messages and may be written again later", () => {
		const { seen, last } = watch();
		deliverRealtimeReply(message({ text: "First try" }), "c1", 1_000);
		deliverRealtimeReply(message({ state: "dropped" }), "c1", 1_100);
		expect(last()).toMatchObject({ text: "First try", state: "dropped" });
		deliverRealtimeReply(message({ text: "First try again" }), "c1", 1_200);
		expect(seen).toHaveLength(2);
		deliverRealtimeReply(message({ text: "Second" }), "c1", 12_000);
		expect(last()).toMatchObject({ text: "Second", state: "writing" });
	});

	test("a rewrite ends the draft held and starts at once from its own beginning", () => {
		const { seen } = watch();
		deliverRealtimeReply(message({ text: "Sure, the answer" }), "c1", 1_000);
		deliverRealtimeReply(message({ state: "dropped" }), "c1", 1_100);
		deliverRealtimeReply(
			message({ text: "Anna said it.", draft: 1 }),
			"c1",
			1_200,
		);
		deliverRealtimeReply(
			message({ text: "Anna said it.", state: "done", draft: 1 }),
			"c1",
			1_300,
		);
		expect(seen.map((reply) => [reply.draft, reply.text, reply.state])).toEqual(
			[
				[0, "Sure, the answer", "writing"],
				[0, "Sure, the answer", "dropped"],
				[1, "Anna said it.", "writing"],
				[1, "Anna said it.", "done"],
			],
		);
	});

	test("a rewrite whose drop was lost still ends the draft held first", () => {
		const { seen } = watch();
		deliverRealtimeReply(message({ text: "Sure, the answer" }));
		deliverRealtimeReply(message({ offset: 4, text: "Anna", draft: 1 }));
		deliverRealtimeReply(message({ text: "Anna said it.", draft: 2 }));
		expect(seen.map((reply) => [reply.draft, reply.text, reply.state])).toEqual(
			[
				[0, "Sure, the answer", "writing"],
				[0, "Sure, the answer", "dropped"],
				[2, "Anna said it.", "writing"],
			],
		);
	});

	test("messages of a replaced draft stay ignored after the restart window", () => {
		const { seen } = watch();
		deliverRealtimeReply(message({ text: "Sure" }), "c1", 1_000);
		deliverRealtimeReply(message({ text: "Anna", draft: 1 }), "c1", 1_100);
		deliverRealtimeReply(message({ text: "Sure, the" }), "c1", 12_000);
		deliverRealtimeReply(message({ state: "dropped" }), "c1", 12_100);
		expect(seen.at(-1)).toMatchObject({
			draft: 1,
			text: "Anna",
			state: "writing",
		});
		expect(seen).toHaveLength(3);
	});

	test("only the watched conversation is told, under the id this device uses", () => {
		const other = watch("c2");
		const local = watch("local-1");
		deliverRealtimeReply(message({ text: "Hi" }), "local-1");
		expect(other.seen).toEqual([]);
		expect(local.last()?.conversationId).toBe("local-1");
		local.stop();
		deliverRealtimeReply(message({ text: "Hi there" }), "local-1");
		expect(local.seen).toHaveLength(1);
	});

	test("a watcher that starts late gets the replies under way, not the finished ones", () => {
		deliverRealtimeReply(message({ text: "Still writing" }));
		deliverRealtimeReply(
			message({ message_id: "re-2", text: "Over.", state: "done" }),
		);
		const { seen } = watch();
		expect(seen.map((reply) => reply.messageId)).toEqual(["re-1"]);
	});

	test("an abandoned reply is forgotten", () => {
		deliverRealtimeReply(message({ text: "Half" }), "c1", 0);
		deliverRealtimeReply(
			message({ message_id: "re-2", text: "x" }),
			"c1",
			121_000,
		);
		const { seen } = watch();
		expect(seen.map((reply) => reply.messageId)).toEqual(["re-2"]);
	});

	test("one failing listener does not stop the others", () => {
		const stop = watchReplyStreams("c1", () => {
			throw new Error("broken");
		});
		const { seen } = watch();
		const warn = console.warn;
		console.warn = () => {};
		try {
			deliverRealtimeReply(message({ text: "Hi" }));
		} finally {
			console.warn = warn;
		}
		expect(seen).toHaveLength(1);
		stop();
	});
});
