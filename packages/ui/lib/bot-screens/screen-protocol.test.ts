import { describe, expect, test } from "bun:test";
import { MAX_LABEL_CHARS, clampLabel, parseSignal } from "./screen-protocol";

describe("clampLabel", () => {
	test("counts UTF-16 units and never splits an emoji", () => {
		expect(clampLabel("x".repeat(70))).toBe("x".repeat(MAX_LABEL_CHARS));
		const clamped = clampLabel(`a${"😀".repeat(40)}`);
		expect(clamped.length).toBe(63);
		expect(clamped.endsWith("😀")).toBe(true);
		expect(clampLabel("Phone")).toBe("Phone");
	});
});

const NOW = 1_763_337_600_000;
const SEALED = { v: 1, enc: "hpke", ct: "abc", keys: [] };
const here = {
	type: "here",
	from: "hostpeer0123456789",
	to: "viewer0123456789",
	bot_id: "bot-1",
	active: true,
	at: NOW,
};
const parse = (value: object, now = NOW) =>
	parseSignal(JSON.stringify(value), now);

describe("parseSignal", () => {
	test("accepts every message of the contract", () => {
		expect(parse(here)).toEqual(here as never);
		expect(
			parse({ type: "hello", from: here.from, bot_id: "bot-1", at: NOW }),
		).not.toBeNull();
		expect(
			parse({
				type: "answer",
				from: here.from,
				to: here.to,
				session: "session01",
				sealed: SEALED,
				at: NOW,
			}),
		).not.toBeNull();
		expect(
			parse({
				type: "refused",
				from: here.from,
				to: here.to,
				session: "session01",
				code: "busy",
				message: "x",
				at: NOW,
			}),
		).not.toBeNull();
		expect(
			parse({
				type: "offer",
				from: here.from,
				to: here.to,
				bot_id: "b",
				session: "session01",
				sealed: SEALED,
				at: NOW,
			}),
		).not.toBeNull();
	});

	test("accepts the no_key refusal and drops sdp in the clear", () => {
		expect(
			parse({
				type: "refused",
				from: here.from,
				to: here.to,
				session: "session01",
				code: "no_key",
				message: "x",
				at: NOW,
			}),
		).not.toBeNull();
		expect(
			parse({
				type: "answer",
				from: here.from,
				to: here.to,
				session: "session01",
				sdp: "v=0",
				at: NOW,
			}),
		).toBeNull();
	});

	test("drops malformed ids, codes and sizes", () => {
		expect(parse({ ...here, from: "short" })).toBeNull();
		expect(parse({ ...here, bot_id: "bad id" })).toBeNull();
		expect(parse({ ...here, active: "yes" })).toBeNull();
		expect(parse({ ...here, type: "other" })).toBeNull();
		expect(
			parse({
				type: "refused",
				from: here.from,
				to: here.to,
				session: "session01",
				code: "nope",
				message: "",
				at: NOW,
			}),
		).toBeNull();
		expect(
			parse({
				type: "answer",
				from: here.from,
				to: here.to,
				session: "session01",
				sealed: { ...SEALED, ct: "a".repeat(48 * 1024) },
				at: NOW,
			}),
		).toBeNull();
		expect(parseSignal("not json", NOW)).toBeNull();
		expect(parseSignal("[]", NOW)).toBeNull();
	});

	test("drops messages more than 300 s from now", () => {
		expect(parse(here, NOW + 300_000)).not.toBeNull();
		expect(parse(here, NOW + 300_001)).toBeNull();
		expect(parse(here, NOW - 300_001)).toBeNull();
	});

	test("carries the host key id and the pairing messages", () => {
		expect(parse({ ...here, key_id: "ik_host" })).toMatchObject({
			key_id: "ik_host",
		});
		expect(parse({ ...here, key_id: "bad key!" })).not.toHaveProperty("key_id");
		expect(parse({ ...here, label: "Studio Mac" })).toMatchObject({
			label: "Studio Mac",
		});
		expect(parse({ ...here, label: "x".repeat(65) })).not.toHaveProperty(
			"label",
		);
		const pair = {
			type: "pair",
			from: here.to,
			to: here.from,
			session: "session01",
			key_id: "ik_viewer",
			public_key: "AQID",
			label: "Phone",
			mac: "AAAA",
			at: NOW,
		};
		expect(parse(pair)).toEqual(pair as never);
		expect(parse({ ...pair, label: "x".repeat(65) })).toBeNull();
		const paired = {
			type: "paired",
			from: here.from,
			to: here.to,
			session: "session01",
			mac: "AAAA",
			at: NOW,
		};
		expect(parse(paired)).toEqual(paired as never);
		expect(
			parse({
				type: "refused",
				from: here.from,
				to: here.to,
				session: "session01",
				code: "not_paired",
				message: "",
				at: NOW,
			}),
		).not.toBeNull();
	});
});
