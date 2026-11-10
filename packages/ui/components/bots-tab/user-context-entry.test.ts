// @vitest-environment happy-dom
import { describe, expect, test } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import type {
	IInboxEntry,
	IUserContextRequest,
} from "../../state/backend-state/inbox-state";
import { transcriptOf } from "../../state/global-chat/bot-history";
import {
	createMessageReference,
	historyMessageForModel,
} from "../interfaces/chat-default/chat-message-interaction";
import { foldInboxEntries } from "./fold-entries";
import { roomChatMessage } from "./room-message-body";
import {
	userContextRequestOf,
	userContextRevokeOf,
} from "./user-context-entry";

const request: IUserContextRequest = {
	id: "context_1",
	bot_id: "bot_1",
	subject_id: "person_1",
	conversation_id: "room_1",
	purpose: "Find nearby places",
	fields: ["location"],
	precision: "approximate",
	audience: "group",
	audience_member_ids: ["bot_1", "person_1", "person_2"],
	created_at: 1000,
	expires_at: 301000,
};

function entry(
	id: string,
	sender: string,
	body: unknown,
	seq: number,
): IInboxEntry {
	return {
		id,
		message_id: id,
		conversation_id: "room_1",
		sender_id: sender,
		kind: "message",
		envelope: { v: 1, enc: "none", body },
		seq,
		cursor: `${seq}:${id}`,
		created_at: new Date(seq * 1000).toISOString(),
	};
}

const card = () =>
	entry(request.id, "bot_1", { text: "", user_context_request: request }, 1);
const reply = (sender = "person_1") =>
	entry(
		"uctx_reply_context_1",
		sender,
		{
			text: "Context shared for this request.",
			user_context_response: {
				request_id: request.id,
				bot_id: request.bot_id,
				subject_id: "person_1",
				conversation_id: "room_1",
				decision: "once",
				captured_at: 5000,
				expires_at: 305000,
				source: { session_id: "s", platform: "web" },
				values: { location: { place: "Private place" } },
				unavailable: {},
			},
		},
		2,
	);

describe("context transcript cards", () => {
	test("status proposals reject unbounded durations, unknown fields and non-availability requests", () => {
		const parse = (proposal: unknown, fields = ["availability"]) =>
			userContextRequestOf(
				entry(
					request.id,
					"bot_1",
					{
						user_context_request: {
							...request,
							fields,
							status_change: proposal,
						},
					},
					1,
				),
			);
		expect(parse({ status: "focus", duration_minutes: 240 })).toBeDefined();
		for (const duration_minutes of [0, 241, 1.5])
			expect(parse({ status: "focus", duration_minutes })).toBeUndefined();
		expect(
			parse({ status: "focus", duration_minutes: 60, hidden: true }),
		).toBeUndefined();
		expect(
			parse({ status: "focus", duration_minutes: 60 }, ["location"]),
		).toBeUndefined();
		expect(
			parse({ status: "focus", duration_minutes: 60 }, [
				"availability",
				"availability",
			]),
		).toBeUndefined();
	});
	test("request cards are bound to their actual bot sender and conversation", () => {
		expect(userContextRequestOf(card())).toEqual(request);
		expect(
			userContextRequestOf({ ...card(), sender_id: "person_2" }),
		).toBeUndefined();
		expect(
			userContextRequestOf({ ...card(), conversation_id: "other" }),
		).toBeUndefined();
		expect(
			userContextRequestOf({ ...card(), message_id: "other" }),
		).toBeUndefined();
	});

	test("an authenticated response answers its card without putting context values in chat messages", () => {
		const transcript = transcriptOf([card(), reply()], "bot_1", "room_1");
		expect(transcript.messages).toHaveLength(1);
		expect(transcript.messages[0].userContext?.answer).toBe("once");
		expect(transcript.messages[0].inner.content).toBe("");
		expect(transcript.messages[0].userContext?.response?.values).toEqual({
			location: { place: "Private place" },
		});
		const room = foldInboxEntries([card(), reply()], "person_1")[0];
		if (room.type !== "message") throw new Error("Expected a room message");
		expect(roomChatMessage(room, "room_1").userContext).toEqual(
			transcript.messages[0].userContext,
		);
	});

	test("another participant cannot answer the requested person's card", () => {
		const folded = foldInboxEntries([card(), reply("person_2")], "person_1");
		const first = folded.find(
			(item) => item.type === "message" && item.id === request.id,
		);
		expect(
			first?.type === "message" && first.userContext?.answer,
		).toBeUndefined();
	});

	test("derived answers stay visible but cannot reenter later model history or references", () => {
		const derived = entry(
			"answer",
			"bot_1",
			{ text: "Meet near Private place", user_context_derived: request.id },
			3,
		);
		const transcript = transcriptOf(
			[card(), reply(), derived],
			"bot_1",
			"room_1",
		);
		const answer = transcript.messages[1];
		expect(answer.inner.content).toBe("Meet near Private place");
		expect(historyMessageForModel(answer).content).toBe("");
		const reference = createMessageReference(answer);
		const followup = {
			...answer,
			userContextDerived: undefined,
			inner: { role: IRole.User, content: "Thanks" },
			interaction: { type: "reply" as const, target: reference },
		};
		expect(historyMessageForModel(followup).content).toBe("Thanks");
	});

	test("session responses are kept and a later revoke marks the card", () => {
		const response = (decision: string) =>
			entry(
				"uctx_reply_context_1",
				"person_1",
				{
					text: "x",
					user_context_response: {
						request_id: request.id,
						bot_id: "bot_1",
						subject_id: "person_1",
						conversation_id: "room_1",
						decision,
						captured_at: 5000,
						expires_at: 305000,
						source: { session_id: "s", platform: "web" },
						values: { location: { place: "Munich" } },
						unavailable: {},
					},
				},
				2,
			);
		const revoke = (sender = "person_1") =>
			entry(
				"uctx_revoke_context_1",
				sender,
				{
					text: "Context session access revoked.",
					user_context_revoke: {
						request_id: request.id,
						bot_id: "bot_1",
						subject_id: "person_1",
						conversation_id: "room_1",
						revoked_at: 9000,
					},
				},
				3,
			);
		const first = (items: ReturnType<typeof foldInboxEntries>) =>
			items[0].type === "message" ? items[0].userContext : undefined;
		const live = foldInboxEntries([card(), response("session")], "person_1");
		expect(first(live)?.response?.captured_at).toBe(5000);
		expect(first(live)?.revoked).toBeUndefined();
		const revoked = foldInboxEntries(
			[card(), response("session"), revoke()],
			"person_1",
		);
		expect(first(revoked)?.revoked).toBe(true);
		expect(revoked).toHaveLength(1);
		const once = foldInboxEntries(
			[card(), response("once"), revoke()],
			"person_1",
		);
		expect(first(once)?.revoked).toBeUndefined();
		expect(userContextRevokeOf(revoke("person_2"))).toBeUndefined();
		expect(userContextRevokeOf(revoke())?.revoked_at).toBe(9000);
	});
});
