import { describe, expect, test } from "bun:test";
import { serverLimitFact } from "./bot-limits";

describe("serverLimitFact", () => {
	test("is what the server says about the plan, without the ways out it adds", () => {
		expect(
			serverLimitFact(
				"Your FREE plan allows 3 bots on the server and you have 3; move one to a desktop or device, delete one, or upgrade your plan",
			),
		).toBe("Your FREE plan allows 3 bots on the server and you have 3");
	});

	test("drops a leading error code and a closing full stop", () => {
		expect(
			serverLimitFact(
				"[BOT_SERVER_LIMIT_REACHED; ref abc123] Your PRO plan allows 25 bots on the server.",
			),
		).toBe("Your PRO plan allows 25 bots on the server");
	});

	test("drops the words a failed move puts before the error code", () => {
		expect(
			serverLimitFact(
				"Could not move bot bot_local1: [BOT_SERVER_LIMIT_REACHED] Your FREE plan allows 3 bots on the server and you have 3; move one",
			),
		).toBe("Your FREE plan allows 3 bots on the server and you have 3");
		expect(
			serverLimitFact(
				"Could not move bot bot_local1: [BOT_SERVER_LIMIT_REACHED]",
			),
		).toBe("");
	});

	test("is empty when the server said nothing", () => {
		expect(serverLimitFact("")).toBe("");
		expect(serverLimitFact("  ")).toBe("");
		expect(serverLimitFact("[BOT_SERVER_LIMIT_REACHED]")).toBe("");
	});
});
