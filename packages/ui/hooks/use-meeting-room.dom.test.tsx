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
import type {
	MeetingClient,
	MeetingRealtimeAccess,
} from "../lib/meetings/types";
import type { ConversationRoom } from "./use-conversation-room";

const realtime = { ...(await import("../lib/realtime/webrtc")) };
const created: {
	room: string;
	dispose: ReturnType<typeof mock>;
	awareness: object;
	provider: object;
}[] = [];
const roots: Root[] = [];
let latest: ConversationRoom | undefined;

beforeAll(() => {
	installDomGlobals();
	mock.module("../lib/realtime/webrtc", () => ({
		...realtime,
		createRealtimeSession: async (options: { room: string }) => {
			const session = {
				room: options.room,
				dispose: mock(() => {}),
				awareness: {
					setLocalStateField() {},
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
	latest = undefined;
	document.body.innerHTML = "";
});
afterAll(() => {
	mock.module("../lib/realtime/webrtc", () => realtime);
	restoreDomGlobals();
});

function access(epoch: number): MeetingRealtimeAccess {
	return {
		jwt: "jwt",
		encryption_key: "key",
		key_id: `epoch-${epoch}`,
		room_id: `meeting-id-${epoch}`,
		expires_at: 1,
		expires_in: 60,
		server_time: "2026-11-04T10:00:00Z",
		signaling_servers: ["wss://example.test"],
	};
}

async function setup(client: MeetingClient) {
	const { createRoot } = await import("react-dom/client");
	const { useMeetingRoom } = await import("./use-meeting-room");
	function Probe({ epoch, active }: { epoch: number; active: boolean }) {
		latest = useMeetingRoom({
			client,
			meetingId: "id",
			token: "session",
			participantId: "person",
			roomEpoch: epoch,
			active,
		}).room;
		return null;
	}
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	roots.push(root);
	return async (epoch: number, active = true) => {
		await act(async () => root.render(createElement(Probe, { epoch, active })));
		await settle();
	};
}

describe("isolated meeting realtime lifecycle", () => {
	test("uses the authorized inbox topic and server-relative credential lifetime", async () => {
		const client = {
			realtime: async () => access(0),
		} as unknown as MeetingClient;
		const render = await setup(client);
		await render(0);
		expect(created[0].room).toBe("inbox:meeting-id-0:room-v1");
		expect(latest).toBeDefined();
		await render(0, false);
		expect(latest).toBeUndefined();
		expect(created[0].dispose).toHaveBeenCalledTimes(1);
	});
	test("queues an epoch change during an in-flight refresh and disposes the old network", async () => {
		let resolveFirst: (value: MeetingRealtimeAccess) => void = () => {};
		let calls = 0;
		const client = {
			realtime: () => {
				calls++;
				return calls === 1
					? new Promise<MeetingRealtimeAccess>((resolve) => {
							resolveFirst = resolve;
						})
					: Promise.resolve(access(1));
			},
		} as unknown as MeetingClient;
		const render = await setup(client);
		await render(0);
		await render(1);
		await act(async () => resolveFirst(access(0)));
		await settle();
		expect(calls).toBeGreaterThanOrEqual(2);
		expect(created.map((session) => session.room)).toEqual([
			"inbox:meeting-id-0:room-v1",
			"inbox:meeting-id-1:room-v1",
		]);
		expect(created[0].dispose).toHaveBeenCalledTimes(1);
		expect(latest?.provider).toBe(created[1].provider);
	});
	test("an authorization refresh failure immediately disposes media transport", async () => {
		let rejected = false;
		const client = {
			realtime: async () => {
				if (rejected) throw new Error("Removed");
				return access(0);
			},
		} as unknown as MeetingClient;
		const render = await setup(client);
		await render(0);
		expect(latest).toBeDefined();
		rejected = true;
		await render(1);
		expect(latest).toBeUndefined();
		expect(created[0].dispose).toHaveBeenCalledTimes(1);
	});
});
