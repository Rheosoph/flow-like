import { describe, expect, test } from "bun:test";
import type {
	IConversation,
	IInboxEntry,
	IInboxKey,
	IInboxState,
	ISendInboxEntryRequest,
} from "../state/backend-state/inbox-state";
import {
	type IInboxCrypto,
	INBOX_KEY_ALGORITHM,
	InboxKeyMissingError,
	sealMembers,
	withEndToEnd,
} from "./inbox-e2e";

const ME = "user-a";
const BOT = "bot_1";

function conversation(encryption: "plain" | "e2e"): IConversation {
	return {
		id: "conv",
		kind: "direct",
		title: null,
		encryption,
		created_by: ME,
		created_at: "2026-11-09T10:00:00Z",
		last_activity_at: "2026-11-09T10:00:00Z",
		members: [
			{ principal_id: ME, kind: "user" },
			{ principal_id: BOT, kind: "bot" },
		] as IConversation["members"],
		unread_count: 0,
		last_entry: null,
	};
}

function key(principal: string, keyId: string): IInboxKey {
	return {
		principal_id: principal,
		key_id: keyId,
		algorithm: INBOX_KEY_ALGORITHM,
		public_key: `pk-${keyId}`,
		created_at: "2026-11-09T10:00:00Z",
	};
}

function sealedEntry(id: string, body: unknown, to = ME): IInboxEntry {
	return {
		id,
		message_id: `m-${id}`,
		conversation_id: "conv",
		sender_id: BOT,
		kind: "message",
		envelope: {
			v: 1,
			enc: INBOX_KEY_ALGORITHM,
			ct: JSON.stringify({ to, body }),
			keys: [],
		},
		created_at: "2026-11-09T10:00:00Z",
		cursor: id,
	};
}

/** Seals as readable JSON naming the member, and opens only what names the reader. */
function fakeCrypto(calls: string[]): IInboxCrypto {
	return {
		key: async (principal) => {
			calls.push(`key:${principal}`);
			return {
				key_id: "k-device",
				algorithm: INBOX_KEY_ALGORITHM,
				public_key: "pk",
				fingerprint: "f",
			};
		},
		seal: async (scope, members, body) => {
			calls.push(
				`seal:${scope.sender_id}:${members.map((member) => `${member.principal_id}=${member.keys.map((k) => k.key_id).join("+")}`).join(",")}`,
			);
			return Object.fromEntries(
				members.map((member) => [
					member.principal_id,
					{
						v: 1 as const,
						enc: INBOX_KEY_ALGORITHM,
						ct: JSON.stringify({ to: member.principal_id, body }),
						keys: [],
					},
				]),
			);
		},
		open: async (principal, entries) => {
			calls.push(`open:${entries.length}`);
			return entries.map((entry) => {
				const sealed = JSON.parse((entry.envelope as { ct: string }).ct);
				return sealed.to === principal ? sealed.body : null;
			});
		},
	};
}

interface FakeInbox {
	inbox: IInboxState;
	sent: ISendInboxEntryRequest[];
	registered: string[];
	setEncryption(value: "plain" | "e2e"): void;
	setKeys(keys: IInboxKey[]): void;
	setEntries(entries: IInboxEntry[]): void;
}

function fakeInbox(initial: "plain" | "e2e"): FakeInbox {
	let encryption = initial;
	let keys = [key(ME, "k-device"), key(BOT, "k-bot")];
	let entries: IInboxEntry[] = [];
	const sent: ISendInboxEntryRequest[] = [];
	const registered: string[] = [];
	const inbox = {
		getConversation: async () => conversation(encryption),
		listConversations: async () => [conversation(encryption)],
		getKeys: async (ids: string[]) =>
			keys.filter((known) => ids.includes(known.principal_id)),
		registerKey: async (request: { key_id: string }) => {
			registered.push(request.key_id);
			return key(ME, request.key_id);
		},
		listEntries: async () => ({ entries, next_before: null }),
		sendEntry: async (_: string, request: ISendInboxEntryRequest) => {
			const sealed = Boolean(request.envelopes);
			if (sealed !== (encryption === "e2e" && request.kind !== "call")) {
				throw Object.assign(new Error("mismatch"), {
					code: "INBOX_ENCRYPTION_MISMATCH",
				});
			}
			sent.push(request);
			return {
				id: `entry-${sent.length}`,
				message_id: request.message_id,
				conversation_id: "conv",
				sender_id: ME,
				kind: request.kind,
				envelope: request.envelopes?.[ME] ?? request.envelope,
				created_at: "2026-11-09T10:00:00Z",
				cursor: "c",
			} as IInboxEntry;
		},
		markRead: async () => ({ last_read_at: "now" }),
	} as unknown as IInboxState;
	return {
		inbox,
		sent,
		registered,
		setEncryption: (value) => {
			encryption = value;
		},
		setKeys: (value) => {
			keys = value;
		},
		setEntries: (value) => {
			entries = value;
		},
	};
}

const message = (id: string): ISendInboxEntryRequest => ({
	message_id: id,
	kind: "message",
	envelope: { v: 1, enc: "none", body: { text: "hello" } },
});

describe("withEndToEnd", () => {
	test("a plain conversation sends the plain envelope untouched", async () => {
		const fake = fakeInbox("plain");
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		await inbox.sendEntry("conv", message("m1"));
		expect(fake.sent[0]).toEqual(message("m1"));
		expect(calls).toEqual([]);
	});

	test("an encrypted conversation is sealed to every member and read back in plain", async () => {
		const fake = fakeInbox("e2e");
		fake.setKeys([key(ME, "k-device"), key(ME, "k-phone"), key(BOT, "k-bot")]);
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		const entry = await inbox.sendEntry("conv", message("m1"));
		await inbox.sendEntry("conv", message("m2"));

		expect(fake.sent[0]?.envelope).toBeUndefined();
		expect(Object.keys(fake.sent[0]?.envelopes ?? {}).sort()).toEqual([
			BOT,
			ME,
		]);
		expect(calls.filter((call) => call.startsWith("seal"))[0]).toBe(
			`seal:${ME}:${ME}=k-device+k-phone,${BOT}=k-bot`,
		);
		expect(entry.envelope).toEqual({
			v: 1,
			enc: "none",
			body: { text: "hello" },
		});
		expect(fake.registered).toEqual(["k-device"]);
		expect(calls.filter((call) => call.startsWith("key"))).toHaveLength(1);
		expect(calls.some((call) => call.startsWith("open"))).toBe(false);
	});

	test("a call entry stays plain in an encrypted conversation and what was said is sealed", async () => {
		const fake = fakeInbox("e2e");
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		const started: ISendInboxEntryRequest = {
			message_id: "call_1",
			kind: "call",
			envelope: { v: 1, enc: "none", body: { op: "started" } },
		};
		const entry = await inbox.sendEntry("conv", started);
		expect(fake.sent[0]).toEqual(started);
		expect(entry.envelope).toEqual({
			v: 1,
			enc: "none",
			body: { op: "started" },
		});
		expect(calls).toEqual([]);

		await inbox.sendEntry("conv", {
			message_id: "t_1",
			kind: "transcript",
			envelope: {
				v: 1,
				enc: "none",
				body: { call: "call_1", text: "hello", offset_ms: 0, duration_ms: 900 },
			},
		});
		expect(fake.sent[1]?.kind).toBe("transcript");
		expect(fake.sent[1]?.envelope).toBeUndefined();
		expect(Object.keys(fake.sent[1]?.envelopes ?? {}).sort()).toEqual([
			BOT,
			ME,
		]);
	});

	test("a member without a key stops the send and is named", async () => {
		const fake = fakeInbox("e2e");
		fake.setKeys([key(ME, "k-device")]);
		const inbox = withEndToEnd(fake.inbox, fakeCrypto([]), () => ME);
		const error = await inbox
			.sendEntry("conv", message("m1"))
			.catch((caught) => caught);
		expect(error).toBeInstanceOf(InboxKeyMissingError);
		expect((error as InboxKeyMissingError).principals).toEqual([BOT]);
		expect(fake.sent).toHaveLength(0);
	});

	test("a conversation that became encrypted is reloaded and the send sealed", async () => {
		const fake = fakeInbox("plain");
		const inbox = withEndToEnd(fake.inbox, fakeCrypto([]), () => ME);
		await inbox.listConversations();
		fake.setEncryption("e2e");
		await inbox.sendEntry("conv", message("m1"));
		expect(fake.sent).toHaveLength(1);
		expect(fake.sent[0]?.envelopes).toBeDefined();
	});

	test("entries open where this device holds a key and stay sealed elsewhere", async () => {
		const fake = fakeInbox("e2e");
		const mine = sealedEntry("e1", { text: "for me" });
		const theirs = sealedEntry("e2", { text: "for the bot" }, BOT);
		fake.setEntries([mine, theirs]);
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);

		const page = await inbox.listEntries("conv");
		expect(page.entries[0]?.envelope).toEqual({
			v: 1,
			enc: "none",
			body: { text: "for me" },
		});
		expect(page.entries[1]?.envelope).toEqual(theirs.envelope);

		await inbox.listEntries("conv");
		expect(calls.filter((call) => call.startsWith("open"))).toEqual(["open:2"]);

		const asBot = await inbox.listEntries("conv", { as: BOT });
		expect(asBot.entries[0]?.envelope).toEqual(mine.envelope);
	});

	test("signed out, nothing is opened and other calls pass through", async () => {
		const fake = fakeInbox("e2e");
		fake.setEntries([sealedEntry("e1", { text: "x" })]);
		const inbox = withEndToEnd(fake.inbox, fakeCrypto([]), () => undefined);
		const page = await inbox.listEntries("conv");
		expect(page.entries[0]?.envelope.enc).toBe(INBOX_KEY_ALGORITHM);
		expect(await inbox.markRead("conv")).toEqual({ last_read_at: "now" });
	});
});

describe("sealMembers", () => {
	test("groups keys by member, newest twenty, of the inbox algorithm only", () => {
		const many = Array.from({ length: 22 }, (_, index) => key(ME, `k${index}`));
		const members = sealMembers(
			[ME, BOT],
			[
				...many,
				{ ...key(BOT, "other"), algorithm: "rsa" },
				key("stranger", "s"),
			],
		);
		expect(members.get(ME)?.keys.map((k) => k.key_id)).toEqual(
			many.slice(2).map((k) => k.key_id),
		);
		expect(members.has(BOT)).toBe(false);
		expect(members.has("stranger")).toBe(false);
	});
});
