import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type { Root } from "react-dom/client";
import {
	installDomGlobals,
	restoreDomGlobals,
	settle,
} from "../components/bots-tab/testing/dom-harness";
import { deriveLiveCall } from "../lib/realtime/conversation-call";
import type {
	IConversation,
	IConversationMember,
} from "../state/backend-state/inbox-state";
import type { ConversationRoom } from "./use-conversation-room";

const actual = {
	realtime: { ...(await import("../lib/realtime/webrtc")) },
	backend: { ...(await import("../state/backend-state")) },
	hub: { ...(await import("./use-hub")) },
};
const getAccess = mock(async (_id: string) => ({
	jwt: "jwt",
	encryption_key: "key",
	key_id: "epoch-1",
}));
const backend = { inboxState: { getConversationRealtimeAccess: getAccess } };
const created: {
	room: string;
	sub?: string;
	signalingServers?: string[];
	onAuthFailure?: () => void;
	dispose: ReturnType<typeof mock>;
	awareness: ConversationRoom["awareness"];
	provider: ConversationRoom["provider"];
}[] = [];
const roots: Root[] = [];
const seen: (ConversationRoom | undefined)[] = [];
let latest: ConversationRoom | undefined;
let signaling: string[] = ["wss://example.test"];
let refusal: Error | null = null;

beforeAll(() => {
	installDomGlobals();
	mock.module("../state/backend-state", () => ({
		...actual.backend,
		useBackend: () => backend,
	}));
	mock.module("./use-hub", () => ({
		...actual.hub,
		useHub: () => ({ hub: { signaling } }),
	}));
	mock.module("../lib/realtime/webrtc", () => ({
		...actual.realtime,
		createRealtimeSession: async (options: {
			room: string;
			sub?: string;
			signalingServers?: string[];
		}) => {
			if (refusal) {
				const cause = refusal;
				refusal = null;
				throw cause;
			}
			const session = {
				...options,
				dispose: mock(() => {}),
				awareness: {
					setLocalStateField: mock((_field: string, _value: unknown) => {}),
					getStates: () => new Map(),
					getLocalState: () => null,
					clientID: 1,
					on() {},
					off() {},
				},
				provider: {
					on() {},
					off() {},
					room: { peerId: "peer", webrtcConns: new Map() },
				},
				refreshAccess() {},
			};
			created.push(session);
			return session;
		},
	}));
});
afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	created.length = 0;
	seen.length = 0;
	refusal = null;
	getAccess.mockClear();
	latest = undefined;
	signaling = ["wss://example.test"];
	document.body.innerHTML = "";
});
afterAll(() => {
	mock.module("../lib/realtime/webrtc", () => actual.realtime);
	mock.module("../state/backend-state", () => actual.backend);
	mock.module("./use-hub", () => actual.hub);
	restoreDomGlobals();
});

const person = (
	principal_id: string,
	kind: "user" | "bot" = "user",
): IConversationMember => ({
	principal_id,
	kind,
	added_by: null,
	joined_at: "2026-11-05T12:00:00Z",
});
function conversation(
	members: IConversationMember[],
	local = false,
): IConversation {
	return {
		id: "conversation-1",
		kind: "direct",
		title: null,
		encryption: "plain",
		created_by: "me",
		created_at: "2026-11-05T12:00:00Z",
		last_activity_at: "2026-11-05T12:00:00Z",
		members,
		unread_count: 0,
		last_entry: null,
		local,
	};
}
async function setup() {
	const { createRoot } = await import("react-dom/client");
	const { useConversationRoom } = await import("./use-conversation-room");
	function Probe({
		item,
		me,
		enabled,
	}: { item: IConversation; me: string | null; enabled: boolean }) {
		latest = useConversationRoom(item, me, enabled);
		seen.push(latest);
		return null;
	}
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	roots.push(root);
	return async (
		item: IConversation,
		me: string | null = "me",
		enabled = true,
	) => {
		await act(async () =>
			root.render(createElement(Probe, { item, me, enabled })),
		);
		await settle();
	};
}

describe("conversation room eligibility", () => {
	test("blocking while access is loading never opens the stale peer session", async () => {
		let release!: (access: {
			jwt: string;
			encryption_key: string;
			key_id: string;
		}) => void;
		getAccess.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					release = resolve;
				}),
		);
		const render = await setup();
		const item = conversation([person("me"), person("other")]);
		await render(item);
		await render(item, "me", false);
		await act(async () =>
			release({ jwt: "jwt", encryption_key: "key", key_id: "epoch-1" }),
		);
		expect(created).toHaveLength(0);
		expect(latest).toBeUndefined();
	});
	test("blocking disconnects an existing call room and unblocking permits a new session", async () => {
		const render = await setup();
		const item = conversation([person("me"), person("other")]);
		await render(item);
		expect(latest).toBeDefined();
		await render(item, "me", false);
		expect(latest).toBeUndefined();
		expect(created[0]?.dispose).toHaveBeenCalledTimes(1);
		expect(getAccess).toHaveBeenCalledTimes(1);
		await render(item);
		expect(created).toHaveLength(2);
		expect(latest).toBeDefined();
	});
	test("opens the shared account room for a person calling a bot", async () => {
		const render = await setup();
		await render(conversation([person("me"), person("bot-1", "bot")]));
		expect(getAccess).toHaveBeenCalledWith("conversation-1");
		expect(created).toHaveLength(1);
		expect(created[0]?.room).toBe("inbox:conversation-1:room-v1");
		expect(created[0]?.sub).toBe("me");
		expect(created[0]?.signalingServers).toEqual(["wss://example.test"]);
		expect(created[0]?.awareness.setLocalStateField).toHaveBeenCalledWith(
			"sub",
			"me",
		);
		expect(latest?.provider).toBe(created[0]?.provider);
	});

	test("continues to open calls between two account users", async () => {
		const render = await setup();
		await render(conversation([person("me"), person("other")]));
		expect(created).toHaveLength(1);
		expect(latest).toBeDefined();
	});

	for (const [name, item, me] of [
		["a person alone", conversation([person("me")]), "me"],
		["an empty room", conversation([]), "me"],
		[
			"a nonmember",
			conversation([person("other"), person("bot-1", "bot")]),
			"me",
		],
		[
			"a bot principal",
			conversation([person("other"), person("bot-1", "bot")]),
			"bot-1",
		],
		[
			"a signed-out participant",
			conversation([person("me"), person("bot-1", "bot")]),
			null,
		],
	] as const) {
		test(`does not request access for ${name}`, async () => {
			const render = await setup();
			await render(item, me);
			expect(getAccess).not.toHaveBeenCalled();
			expect(created).toHaveLength(0);
			expect(latest).toBeUndefined();
		});
	}

	test("a local bot call publishes microphone state without account signaling", async () => {
		signaling = [];
		const render = await setup();
		const item = conversation([person("me"), person("bot-1", "bot")], true);
		await render(item);
		const room = latest;
		expect(room?.provider.room?.peerId).toBeTruthy();
		expect(room?.awareness.getLocalState()?.sub).toBe("me");
		const changed = mock(() => {});
		room?.awareness.on("change", changed);
		room?.awareness.setLocalStateField("call", {
			id: "call-1",
			joined: true,
			muted: false,
			listening: true,
			peer: room.provider.room?.peerId,
			voiceBots: ["bot-1"],
		});
		expect(changed).toHaveBeenCalledTimes(1);
		expect(
			room?.awareness.getStates().get(room.awareness.clientID),
		).toMatchObject({
			sub: "me",
			call: { id: "call-1", joined: true },
		});
		if (!room) throw new Error("The local call room was not created");
		expect(
			deriveLiveCall(
				room.awareness.getStates(),
				room.awareness.clientID,
				new Set(["me"]),
			),
		).toMatchObject({
			callId: "call-1",
			participants: [
				{ sub: "me", local: true, listening: true, voiceBots: ["bot-1"] },
			],
		});
		await render({ ...item });
		expect(latest).toBe(room);
		room?.awareness.off("change", changed);
		room?.awareness.setLocalStateField("call", undefined);
		expect(changed).toHaveBeenCalledTimes(1);
		expect(getAccess).not.toHaveBeenCalled();
		expect(created).toHaveLength(0);
		await render(item, "me", false);
		expect(latest).toBeUndefined();
		await render(item);
		expect(latest).not.toBe(room);
		expect(latest?.awareness.getLocalState()?.call).toBeUndefined();
	});

	test("a local room never substitutes for calls with other people", async () => {
		const render = await setup();
		await render(
			conversation(
				[person("me"), person("other"), person("bot-1", "bot")],
				true,
			),
		);
		expect(latest).toBeUndefined();
		expect(getAccess).not.toHaveBeenCalled();
	});

	test("requires configured signaling even for a direct bot call", async () => {
		signaling = [];
		const render = await setup();
		await render(conversation([person("me"), person("bot-1", "bot")]));
		expect(getAccess).not.toHaveBeenCalled();
		expect(latest).toBeUndefined();
	});

	test("removing the bot disposes the direct call room", async () => {
		const render = await setup();
		await render(conversation([person("me"), person("bot-1", "bot")]));
		expect(latest).toBeDefined();
		await render(conversation([person("me")]));
		expect(latest).toBeUndefined();
		expect(created[0]?.dispose).toHaveBeenCalledTimes(1);
		expect(getAccess).toHaveBeenCalledTimes(1);
	});
});

describe("a conversation room on a new key", () => {
	const nextKey = { jwt: "jwt", encryption_key: "key-2", key_id: "epoch-2" };

	/** The access went stale, as a refused socket or a newer key elsewhere reports it. */
	async function staleAccess() {
		await act(async () => created[0]?.onAuthFailure?.());
		await settle();
	}

	test("the room is swapped in place, so a call in it carries over", async () => {
		const render = await setup();
		await render(conversation([person("me"), person("other")]));
		const first = latest;
		expect(first?.provider).toBe(created[0]?.provider);

		getAccess.mockImplementationOnce(async () => nextKey);
		await staleAccess();
		expect(created).toHaveLength(2);
		expect(created[0]?.dispose).toHaveBeenCalledTimes(1);
		expect(latest?.provider).toBe(created[1]?.provider);
		expect(seen.slice(seen.indexOf(first))).not.toContain(undefined);
	});

	test("the same key keeps the room and only refreshes its access", async () => {
		const render = await setup();
		await render(conversation([person("me"), person("other")]));
		const first = latest;

		await staleAccess();
		expect(getAccess).toHaveBeenCalledTimes(2);
		expect(created).toHaveLength(1);
		expect(created[0]?.dispose).not.toHaveBeenCalled();
		expect(latest).toBe(first);
	});

	test("a rebuild that fails closes the room instead of leaving a dead one", async () => {
		const warning = spyOn(console, "warn").mockImplementation(() => undefined);
		const render = await setup();
		await render(conversation([person("me"), person("other")]));
		expect(latest).toBeDefined();

		getAccess.mockImplementationOnce(async () => nextKey);
		refusal = new Error("signaling refused");
		await staleAccess();
		expect(created).toHaveLength(1);
		expect(created[0]?.dispose).toHaveBeenCalledTimes(1);
		expect(latest).toBeUndefined();
		warning.mockRestore();
	});

	test("someone joining swaps the room in place instead of closing it", async () => {
		const render = await setup();
		await render(conversation([person("me"), person("other")]));
		const first = latest;

		getAccess.mockImplementationOnce(async () => nextKey);
		await render(
			conversation([person("me"), person("other"), person("third")]),
		);
		expect(getAccess).toHaveBeenCalledTimes(2);
		expect(created).toHaveLength(2);
		expect(latest?.provider).toBe(created[1]?.provider);
		expect(seen.slice(seen.indexOf(first))).not.toContain(undefined);
	});

	test("the same members in another order ask for nothing", async () => {
		const render = await setup();
		await render(conversation([person("me"), person("other")]));
		await render(conversation([person("other"), person("me")]));
		expect(getAccess).toHaveBeenCalledTimes(1);
		expect(created).toHaveLength(1);
	});

	test("someone joining while the room still opens is answered once it stands", async () => {
		let answer!: (access: typeof nextKey) => void;
		getAccess.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					answer = resolve;
				}),
		);
		const render = await setup();
		await render(conversation([person("me"), person("other")]));
		await render(
			conversation([person("me"), person("other"), person("third")]),
		);
		expect(created).toHaveLength(0);

		getAccess.mockImplementationOnce(async () => nextKey);
		await act(async () =>
			answer({ jwt: "jwt", encryption_key: "key", key_id: "epoch-1" }),
		);
		await settle();
		expect(getAccess).toHaveBeenCalledTimes(2);
		expect(created).toHaveLength(2);
		expect(latest?.provider).toBe(created[1]?.provider);
	});
});

describe("when a room's access is fetched again", () => {
	const MINUTE = 60_000;
	const HOUR = 60 * MINUTE;
	const midnight = Date.UTC(2026, 10, 19);
	const idle = { tokenExpiresAt: null, iceExpiresAt: null, keyRollsAt: null };
	type Hook = typeof import("./use-conversation-room");
	let roomKeyRollsAt: Hook["roomKeyRollsAt"];
	let roomAccessClock: Hook["roomAccessClock"];
	let roomAccessDue: Hook["roomAccessDue"];
	let roomAccessRefreshAt: Hook["roomAccessRefreshAt"];
	beforeAll(async () => {
		({ roomKeyRollsAt, roomAccessClock, roomAccessDue, roomAccessRefreshAt } =
			await import("./use-conversation-room"));
	});

	test("a room key rolls at the end of the UTC day in its id", () => {
		expect(roomKeyRollsAt("2026-11-18.0123456789abcdef")).toBe(midnight);
		expect(roomKeyRollsAt("2026-12-31.0123456789abcdef")).toBe(
			Date.UTC(2027, 0, 1),
		);
		expect(roomKeyRollsAt("epoch-1")).toBeNull();
		expect(roomKeyRollsAt("")).toBeNull();
	});

	test("an access names what runs out in it", () => {
		const payload = btoa(JSON.stringify({ exp: 1_800_000_000 }));
		expect(
			roomAccessClock({
				jwt: `header.${payload}.signature`,
				encryption_key: "key",
				key_id: "2026-11-18.0123456789abcdef",
				ice_expires_at: 1_800_003_600,
			}),
		).toEqual({
			tokenExpiresAt: 1_800_000_000_000,
			iceExpiresAt: 1_800_003_600_000,
			keyRollsAt: midnight,
		});
	});

	test("the earliest of token, relay credentials and key decides", () => {
		const now = midnight - 10 * HOUR;
		const clock = {
			tokenExpiresAt: now + 3 * HOUR,
			iceExpiresAt: now + 4 * HOUR,
			keyRollsAt: midnight,
		};
		expect(roomAccessRefreshAt(clock, now)).toBe(now + 3 * HOUR - 5 * MINUTE);
		expect(
			roomAccessRefreshAt({ ...clock, tokenExpiresAt: now + 12 * HOUR }, now),
		).toBe(now + 4 * HOUR - 2 * MINUTE);
		expect(roomAccessRefreshAt(idle, now)).toBeNull();
	});

	test("the key is fetched again as its day turns", () => {
		const evening = midnight - 30 * MINUTE;
		const clock = {
			tokenExpiresAt: evening + 3 * HOUR,
			iceExpiresAt: evening + 4 * HOUR,
			keyRollsAt: midnight,
		};
		expect(roomAccessRefreshAt(clock, evening, 2_000)).toBe(midnight + 2_000);
		expect(roomAccessDue(clock, midnight - 1)).toBe(false);
		expect(roomAccessDue(clock, midnight)).toBe(true);
	});

	test("a refresh is never planned sooner than a minute ahead", () => {
		const now = midnight - 10_000;
		const clock = { ...idle, tokenExpiresAt: now + HOUR, keyRollsAt: midnight };
		expect(roomAccessRefreshAt(clock, now)).toBe(now + MINUTE);
	});

	test("a hub still on the old day is asked again, but only for ten minutes", () => {
		const clock = {
			tokenExpiresAt: midnight + 3 * HOUR,
			iceExpiresAt: null,
			keyRollsAt: midnight,
		};
		const justAfter = midnight + 5_000;
		expect(roomAccessDue(clock, justAfter)).toBe(true);
		expect(roomAccessRefreshAt(clock, justAfter)).toBe(justAfter + MINUTE);

		const later = midnight + 10 * MINUTE;
		expect(roomAccessDue(clock, later)).toBe(false);
		expect(roomAccessRefreshAt(clock, later)).toBe(
			midnight + 3 * HOUR - 5 * MINUTE,
		);
	});

	test("an access without an expiring token is due at once", () => {
		expect(roomAccessDue(idle, midnight)).toBe(true);
	});
});
