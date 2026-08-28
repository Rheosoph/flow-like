import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import type { IChannelHandle } from "../../schema/channel";
import {
	cancelChannel,
	replyToChannel,
	resetChannelClientCaches,
} from "../index";

const SIGN_IN_URL =
	"https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key=api-key";

function firebaseHandle(requestId: string | null = "req-1"): IChannelHandle {
	return {
		channel_id: "run-1",
		request_id: requestId,
		expires_at: 4_102_444_800,
		transport: {
			type: "gcp_firebase_rtdb",
			database_url: "https://db.example.firebaseio.com/",
			api_key: "api-key",
			project_id: "proj",
			custom_token: "custom-token",
			inbox_path: "channels/run-1/inbox",
			inbound_path: "/channels/run-1/inbound/",
			expires_at: 4_102_444_800,
		},
	};
}

type FetchCall = { url: string; init: RequestInit };
let calls: FetchCall[] = [];
let responder: (call: FetchCall) => Response = () =>
	new Response(null, { status: 200 });
const originalFetch = globalThis.fetch;

beforeEach(() => {
	calls = [];
	resetChannelClientCaches();
	responder = (call) =>
		call.url.startsWith(SIGN_IN_URL)
			? Response.json({ idToken: "id-token-1", expiresIn: "3600" })
			: new Response(null, { status: 200 });
	globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
		const call = { url: String(input), init: init ?? {} };
		calls.push(call);
		return responder(call);
	}) as typeof fetch;
});

afterEach(() => {
	globalThis.fetch = originalFetch;
	resetChannelClientCaches();
});

describe("gcp_firebase_rtdb channel push", () => {
	it("exchanges the custom token once and PUTs replies under the inbox", async () => {
		await replyToChannel(firebaseHandle(), { ok: true });
		await replyToChannel(firebaseHandle("req-2"), { ok: false });

		expect(calls.map((call) => call.url)).toEqual([
			SIGN_IN_URL,
			"https://db.example.firebaseio.com/channels/run-1/inbox/req-1.json?auth=id-token-1",
			"https://db.example.firebaseio.com/channels/run-1/inbox/req-2.json?auth=id-token-1",
		]);
		expect(calls[0].init.method).toBe("POST");
		expect(JSON.parse(String(calls[0].init.body))).toEqual({
			token: "custom-token",
			returnSecureToken: true,
		});
		expect(calls[1].init.method).toBe("PUT");
		const body = JSON.parse(String(calls[1].init.body)) as {
			payload: string;
		};
		expect(Object.keys(body)).toEqual(["payload"]);
		expect(JSON.parse(body.payload)).toEqual({
			channel_id: "run-1",
			request_id: "req-1",
			kind: "reply",
			value: { ok: true },
		});
	});

	it("POSTs unsolicited messages under the inbound path", async () => {
		await cancelChannel(firebaseHandle(null));

		expect(calls[1].url).toBe(
			"https://db.example.firebaseio.com/channels/run-1/inbound.json?auth=id-token-1",
		);
		expect(calls[1].init.method).toBe("POST");
		expect(JSON.parse(JSON.parse(String(calls[1].init.body)).payload)).toEqual({
			channel_id: "run-1",
			kind: "cancel",
			value: null,
		});
	});

	it("re-exchanges after the cache is reset and names rule denials", async () => {
		await replyToChannel(firebaseHandle(), 1);
		resetChannelClientCaches();
		responder = (call) =>
			call.url.startsWith(SIGN_IN_URL)
				? Response.json({ idToken: "id-token-2", expiresIn: "3600" })
				: new Response("Permission denied", { status: 403 });

		await expect(replyToChannel(firebaseHandle(), 2)).rejects.toThrow(
			/\(403\) \(rules denied\): Permission denied/,
		);
		expect(
			calls.filter((call) => call.url.startsWith(SIGN_IN_URL)),
		).toHaveLength(2);
		expect(calls.at(-1)?.url).toContain("auth=id-token-2");
	});

	it("fails clearly when the sign-in is rejected", async () => {
		responder = () => new Response("INVALID_CUSTOM_TOKEN", { status: 400 });

		await expect(replyToChannel(firebaseHandle(), 1)).rejects.toThrow(
			/sign-in for channel 'run-1' failed \(400\): INVALID_CUSTOM_TOKEN/,
		);
	});
});
