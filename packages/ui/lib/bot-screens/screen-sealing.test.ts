import { describe, expect, test } from "bun:test";
import { type IInboxCrypto, INBOX_KEY_ALGORITHM } from "../inbox-e2e";
import { ScreenNoKeyError, createScreenSealing } from "./screen-sealing";
import { memoryTrustStore } from "./screen-trust";

function harness(
	opened: unknown = { hello: 1 },
	sub: string | undefined = "sub-1",
) {
	const sealed: unknown[] = [];
	const crypto = {
		key: async () => ({
			key_id: "ik_own",
			algorithm: INBOX_KEY_ALGORITHM,
			public_key: "pk-own",
			fingerprint: "",
		}),
		seal: async (scope: unknown, members: unknown) => {
			sealed.push({ scope, members });
			return { "sub-1": { v: 1, enc: "x", ct: "c" } };
		},
		open: async () => [opened],
	} as unknown as IInboxCrypto;
	const trust = memoryTrustStore();
	const sealing = createScreenSealing({ crypto, sub: () => sub, trust });
	return { sealing, sealed, trust };
}

const host = {
	key_id: "ik_host",
	public_key: "pk-host",
	label: "Studio Mac",
	host_peer_id: "hostpeer0123",
	paired_at: 1,
};

describe("createScreenSealing", () => {
	test("seals under the bot-screen scope to exactly the pinned key", async () => {
		const { sealing, sealed } = harness();
		await sealing.seal({ session: "s1", message: "offer" }, { a: 1 }, host);
		expect(sealed).toEqual([
			{
				scope: {
					conversation_id: "bot-screen:s1",
					message_id: "offer",
					sender_id: "sub-1",
				},
				members: [
					{
						principal_id: "sub-1",
						keys: [{ key_id: "ik_host", public_key: "pk-host" }],
					},
				],
			},
		]);
	});

	test("reports this device's own key id", async () => {
		expect(await harness().sealing.own()).toEqual({
			key_id: "ik_own",
			public_key: "pk-own",
		});
	});

	test("opens a body and reports no_key when nothing opens", async () => {
		expect(
			await harness().sealing.open(
				{ session: "s1", message: "answer" },
				{ v: 1, enc: "x", ct: "c" },
			),
		).toEqual({ hello: 1 });
		await expect(
			harness(null).sealing.open(
				{ session: "s1", message: "answer" },
				{ v: 1, enc: "x", ct: "c" },
			),
		).rejects.toBeInstanceOf(ScreenNoKeyError);
	});

	test("pins and forgets hosts per account, and signed out trusts nobody", async () => {
		const { sealing, trust } = harness();
		await sealing.pin(host);
		expect(await sealing.trusted()).toEqual([host]);
		expect(await trust.list("sub-2")).toEqual([]);
		await sealing.unpin("ik_host");
		expect(await sealing.trusted()).toEqual([]);
		expect(await harness(undefined, undefined).sealing.trusted()).toEqual([]);
	});
});
