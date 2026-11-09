import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
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
	dispose: ReturnType<typeof mock>;
	awareness: ConversationRoom["awareness"];
	provider: ConversationRoom["provider"];
}[] = [];
const roots: Root[] = [];
let latest: ConversationRoom | undefined;
let signaling: string[] = ["wss://example.test"];

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
