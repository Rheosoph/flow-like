import { describe, expect, test } from "bun:test";
import { fromBase64Url } from "./base64";
import { openEnvelope, sealEnvelope } from "./screen-crypto";

const KEY = {
	id: "2026-11-17",
	key: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
};
const ROOM = "0123456789abcdef0123456789abcdef";
const IV = "oKGio6Slpqeoqaqr";
const PLAIN =
	'{"type":"hello","from":"viewer0123456789","bot_id":"bot-1","at":1763337600000}';
const CT =
	"wvNJH43QeD5dZscHneoCqHWxwnY1SlqmUTKwijc86AQD4q392eWIDQ9a4vvJQhFOfZRK5q70ivh5xq_So0IlUp11ldedGHvH6qB2xDKnTNPhf2plgt4ppT_5xq-LNA";

describe("screen crypto", () => {
	test("seals the shared vector", async () => {
		const envelope = await sealEnvelope(KEY, ROOM, PLAIN, fromBase64Url(IV));
		expect(envelope).toEqual({ v: 1, key_id: KEY.id, iv: IV, ct: CT });
	});

	test("opens the shared vector", async () => {
		const opened = await openEnvelope(
			{ v: 1, key_id: KEY.id, iv: IV, ct: CT },
			ROOM,
			[KEY],
		);
		expect(opened).toEqual({ status: "ok", text: PLAIN });
	});

	test("round-trips with a random iv", async () => {
		const envelope = await sealEnvelope(KEY, ROOM, "hi");
		expect(await openEnvelope(envelope, ROOM, [KEY])).toEqual({
			status: "ok",
			text: "hi",
		});
	});

	test("refuses another room, a tampered ciphertext and an unknown key", async () => {
		const envelope = { v: 1, key_id: KEY.id, iv: IV, ct: CT };
		expect((await openEnvelope(envelope, "f".repeat(32), [KEY])).status).toBe(
			"invalid",
		);
		expect(
			(
				await openEnvelope({ ...envelope, ct: `${CT.slice(0, -2)}AA` }, ROOM, [
					KEY,
				])
			).status,
		).toBe("invalid");
		expect(
			await openEnvelope(envelope, ROOM, [{ ...KEY, id: "2026-11-16" }]),
		).toEqual({
			status: "unknown_key",
			keyId: KEY.id,
		});
		expect((await openEnvelope("nope", ROOM, [KEY])).status).toBe("invalid");
	});
});
