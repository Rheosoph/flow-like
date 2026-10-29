import { describe, expect, test } from "bun:test";
import { IEventExposure } from "../../../lib/schema/flow/event";
import { eventRunAsAudience } from "./event-run-as";

const CALLABLE = ["rest", "mcp"];
const OTHER_HUB_SINKS = [
	"cron",
	"api",
	"daemon",
	"deeplink",
	"discord",
	"telegram",
	"teams",
	"email",
	"inbound_email",
	"geolocation",
];

describe("whom the run-as chip says a bound Event runs as", () => {
	test("a PUBLIC REST or MCP Event runs public calls as its run-as user", () => {
		for (const event_type of CALLABLE) {
			for (const exposure of [IEventExposure.Public, undefined]) {
				expect(eventRunAsAudience({ event_type, exposure }, false)).toBe(
					"public_calls",
				);
			}
		}
	});

	test("an INTERNAL REST or MCP Event runs as the calling person", () => {
		for (const event_type of CALLABLE) {
			expect(
				eventRunAsAudience(
					{ event_type, exposure: IEventExposure.Internal },
					false,
				),
			).toBe("calling_person");
		}
	});

	test("a REST or MCP Event keeps its caller wording with a hosted page", () => {
		for (const event_type of CALLABLE) {
			expect(eventRunAsAudience({ event_type }, true)).toBe("public_calls");
		}
	});

	test("a hosted page lends its run-as user's access to visitors", () => {
		for (const event_type of ["simple_chat", "generic_form", "quick_action"]) {
			expect(eventRunAsAudience({ event_type }, true)).toBe("visitors");
		}
	});

	test("every other hub sink runs as its run-as user, whatever its exposure", () => {
		for (const event_type of OTHER_HUB_SINKS) {
			for (const exposure of [
				IEventExposure.Public,
				IEventExposure.Internal,
				undefined,
			]) {
				expect(eventRunAsAudience({ event_type, exposure }, false)).toBe(
					"run_as_user",
				);
			}
		}
	});
});
