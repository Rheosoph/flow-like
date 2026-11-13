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
	SEALED_FILE_ALGORITHM,
	SEALED_FILE_NAME,
	sealConversationFile,
	sealMembers,
	sealedFileBytes,
	withEndToEnd,
} from "./inbox-e2e";

const ME = "user-a";
const BOT = "bot_1";

function conversation(
	encryption: "plain" | "e2e",
	memberIds: readonly string[] = [ME, BOT],
): IConversation {
	return {
		id: "conv",
		kind: "direct",
		title: null,
		encryption,
		created_by: ME,
		created_at: "2026-11-09T10:00:00Z",
		last_activity_at: "2026-11-09T10:00:00Z",
		members: memberIds.map((id) => ({
			principal_id: id,
			kind: id.startsWith("bot_") ? "bot" : "user",
		})) as IConversation["members"],
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
		sealSecret: async (scope, keys, value) => {
			calls.push(
				`sealSecret:${scope.bot_id}:${scope.name}:${keys.map((k) => k.key_id).join("+")}`,
			);
			return {
				v: 1 as const,
				enc: INBOX_KEY_ALGORITHM,
				ct: JSON.stringify({ to: scope.bot_id, value }),
				keys: [],
			};
		},
		open: async (principal, entries) => {
			calls.push(`open:${entries.length}`);
			return entries.map((entry) => {
				const sealed = JSON.parse((entry.envelope as { ct: string }).ct);
				return sealed.to === principal ? sealed.body : null;
			});
		},
		sealFile: async (conversationId, file) => {
			calls.push(`sealFile:${conversationId}:${file.length}`);
			return {
				key: `key-${conversationId}`,
				ciphertext: Uint8Array.from([...file].reverse()),
			};
		},
		openFile: async (conversationId, fileKey, ciphertext) => {
			calls.push(`openFile:${conversationId}`);
			if (fileKey !== `key-${conversationId}`) {
				throw new Error("the conversation file does not open with this key");
			}
			return Uint8Array.from([...ciphertext].reverse());
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
	setMembers(memberIds: string[]): void;
}

function fakeInbox(initial: "plain" | "e2e"): FakeInbox {
	let encryption = initial;
	let memberIds = [ME, BOT];
	let keys = [key(ME, "k-device"), key(BOT, "k-bot")];
	let entries: IInboxEntry[] = [];
	const sent: ISendInboxEntryRequest[] = [];
	const registered: string[] = [];
	const inbox = {
		getConversation: async () => conversation(encryption, memberIds),
		listConversations: async () => [conversation(encryption, memberIds)],
		getKeys: async (ids: string[]) =>
			keys.filter((known) => ids.includes(known.principal_id)),
		registerKey: async (request: { key_id: string }) => {
			registered.push(request.key_id);
			return key(ME, request.key_id);
		},
		listEntries: async () => ({ entries, next_before: null }),
		conversationFileDownload: async (
			_: string,
			handle: { path: string; mime: string },
		) => ({
			url: `https://files.example/${handle.path}`,
			name: SEALED_FILE_NAME,
			mime: handle.mime,
			bytes: 3,
		}),
		sendEntry: async (_: string, request: ISendInboxEntryRequest) => {
			const sealed = Boolean(request.envelopes);
			if (sealed !== (encryption === "e2e" && request.kind !== "call")) {
				throw Object.assign(new Error("mismatch"), {
					code: "INBOX_ENCRYPTION_MISMATCH",
				});
			}
			if (
				request.envelopes &&
				Object.keys(request.envelopes).sort().join() !==
					[...memberIds].sort().join()
			) {
				throw Object.assign(new Error("members changed"), {
					code: "INBOX_ENVELOPES_MISMATCH",
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
		setMembers: (value) => {
			memberIds = value;
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

describe("sealBotSecret", () => {
	test("seals to the bot's own published keys, bound to its id and the name", async () => {
		const fake = fakeInbox("plain");
		fake.setKeys([
			key(ME, "k-device"),
			key(BOT, "k-bot"),
			key(BOT, "k-bot-2"),
			{ ...key(BOT, "k-rsa"), algorithm: "rsa" },
		]);
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		const envelope = await inbox.sealBotSecret?.(BOT, "API_KEY", "sk-1");
		expect(calls).toEqual([`sealSecret:${BOT}:API_KEY:k-bot+k-bot-2`]);
		expect(envelope?.enc).toBe(INBOX_KEY_ALGORITHM);
	});

	test("a bot without a published key gets nothing sealed", async () => {
		const fake = fakeInbox("plain");
		fake.setKeys([key(ME, "k-device")]);
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		await expect(inbox.sealBotSecret?.(BOT, "API_KEY", "sk-1")).rejects.toThrow(
			"published no inbox key",
		);
		expect(calls).toEqual([]);
	});
});

const SEALED_PART = {
	kind: "attachment" as const,
	id: "a1",
	name: "draft.pdf",
	handle: {
		app_id: "conversation_conv",
		path: "file_1",
		bytes: 38,
		sha256: "c1",
		mime: "application/octet-stream",
	},
	sealed: {
		alg: SEALED_FILE_ALGORITHM,
		key: "key-conv",
		mime: "application/pdf",
		bytes: 3,
	},
};

const withFile = (id: string): ISendInboxEntryRequest => ({
	message_id: id,
	kind: "message",
	envelope: {
		v: 1,
		enc: "none",
		body: { text: "the draft", parts: [SEALED_PART] },
	},
});

async function withFetch<T>(
	bytes: number[],
	work: (fetched: string[]) => Promise<T>,
): Promise<T> {
	const original = globalThis.fetch;
	const fetched: string[] = [];
	globalThis.fetch = (async (url: string) => {
		fetched.push(url);
		return new Response(Uint8Array.from(bytes));
	}) as unknown as typeof fetch;
	try {
		return await work(fetched);
	} finally {
		globalThis.fetch = original;
	}
}

describe("files of encrypted conversations", () => {
	test("a message with a sealed file reaches the server only as envelopes", async () => {
		const fake = fakeInbox("e2e");
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		const entry = await inbox.sendEntry("conv", withFile("m1"));
		expect(Object.keys(fake.sent[0] ?? {}).sort()).toEqual([
			"envelopes",
			"kind",
			"message_id",
		]);
		expect(entry.envelope).toEqual({
			v: 1,
			enc: "none",
			body: { text: "the draft", parts: [SEALED_PART] },
		});
		const sealedBody = JSON.parse(
			(fake.sent[0]?.envelopes?.[BOT] as { ct: string }).ct,
		);
		expect(sealedBody.body.parts[0].sealed.key).toBe("key-conv");
	});

	test("a plain conversation never receives a sealed file's key or name", async () => {
		const fake = fakeInbox("plain");
		const inbox = withEndToEnd(fake.inbox, fakeCrypto([]), () => ME);
		await expect(inbox.sendEntry("conv", withFile("m1"))).rejects.toThrow(
			"isn't encrypted",
		);
		expect(fake.sent).toHaveLength(0);
	});

	test("a member removed before the send gets no envelope and so no file key", async () => {
		const fake = fakeInbox("e2e");
		fake.setMembers([ME, BOT, "user-gone"]);
		fake.setKeys([
			key(ME, "k-device"),
			key(BOT, "k-bot"),
			key("user-gone", "k-gone"),
		]);
		const inbox = withEndToEnd(fake.inbox, fakeCrypto([]), () => ME);
		await inbox.listConversations();
		fake.setMembers([ME, BOT]);
		await inbox.sendEntry("conv", withFile("m1"));
		expect(fake.sent).toHaveLength(1);
		expect(Object.keys(fake.sent[0]?.envelopes ?? {}).sort()).toEqual([
			BOT,
			ME,
		]);
	});

	test("a device that opened the message opens the file and shows its own name and type", async () => {
		const fake = fakeInbox("e2e");
		fake.setEntries([
			sealedEntry("e1", { text: "the draft", parts: [SEALED_PART] }),
		]);
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		await inbox.listEntries("conv");
		const download = await withFetch([3, 2, 1], async (fetched) => {
			const result = await inbox.conversationFileDownload("conv", {
				path: "file_1",
				mime: "application/octet-stream",
			});
			expect(fetched).toEqual(["https://files.example/file_1"]);
			return result;
		});
		expect(download.url.startsWith("blob:")).toBe(true);
		expect(download).toMatchObject({
			name: "draft.pdf",
			mime: "application/pdf",
			bytes: 3,
		});
		const opened = await (await fetch(download.url)).arrayBuffer();
		expect([...new Uint8Array(opened)]).toEqual([1, 2, 3]);
		expect(calls).toContain("openFile:conv");
	});

	test("a device without the message's key opens no file and keeps the server's link", async () => {
		const fake = fakeInbox("e2e");
		fake.setEntries([
			sealedEntry("e1", { text: "for the bot", parts: [SEALED_PART] }, BOT),
		]);
		const calls: string[] = [];
		const inbox = withEndToEnd(fake.inbox, fakeCrypto(calls), () => ME);
		const page = await inbox.listEntries("conv");
		expect(page.entries[0]?.envelope.enc).toBe(INBOX_KEY_ALGORITHM);
		const download = await inbox.conversationFileDownload("conv", {
			path: "file_1",
			mime: "application/octet-stream",
		});
		expect(download.url).toBe("https://files.example/file_1");
		expect(calls.some((call) => call.startsWith("openFile"))).toBe(false);
	});

	test("a file key from another conversation's message opens nothing", async () => {
		const fake = fakeInbox("e2e");
		const foreign = {
			...SEALED_PART,
			sealed: { ...SEALED_PART.sealed, key: "key-other" },
		};
		fake.setEntries([sealedEntry("e1", { text: "copied", parts: [foreign] })]);
		const inbox = withEndToEnd(fake.inbox, fakeCrypto([]), () => ME);
		await inbox.listEntries("conv");
		await withFetch([3, 2, 1], async () => {
			await expect(
				inbox.conversationFileDownload("conv", {
					path: "file_1",
					mime: "application/octet-stream",
				}),
			).rejects.toThrow("Could not open the encrypted file draft.pdf");
		});
	});

	test("sealing a file uploads ciphertext under a name and type that say nothing", async () => {
		const calls: string[] = [];
		const file = new File([Uint8Array.from([1, 2, 3])], "draft.pdf", {
			type: "application/pdf",
		});
		const { upload, sealed } = await sealConversationFile(
			fakeCrypto(calls),
			"conv",
			file,
		);
		expect(upload.name).toBe(SEALED_FILE_NAME);
		expect(upload.type).toBe("application/octet-stream");
		expect([...new Uint8Array(await upload.arrayBuffer())]).toEqual([3, 2, 1]);
		expect(sealed).toEqual({
			alg: SEALED_FILE_ALGORITHM,
			key: "key-conv",
			mime: "application/pdf",
			bytes: 3,
		});
		expect(calls).toEqual(["sealFile:conv:3"]);
	});

	test("a file too large once sealed is refused before anything is read", async () => {
		const calls: string[] = [];
		const huge = {
			name: "film.mov",
			type: "video/quicktime",
			size: 35 * 1024 * 1024,
			arrayBuffer: async () => {
				throw new Error("read");
			},
		} as unknown as File;
		await expect(
			sealConversationFile(fakeCrypto(calls), "conv", huge),
		).rejects.toThrow("Could not encrypt film.mov");
		expect(calls).toEqual([]);
	});

	test("the sealed size matches the Rust format", () => {
		expect(sealedFileBytes(0)).toBe(35);
		expect(sealedFileBytes(65_536)).toBe(19 + 65_536 + 16);
		expect(sealedFileBytes(65_537)).toBe(19 + 65_537 + 32);
		expect(sealedFileBytes(35 * 1024 * 1024)).toBe(
			35 * 1024 * 1024 + 19 + 560 * 16,
		);
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
