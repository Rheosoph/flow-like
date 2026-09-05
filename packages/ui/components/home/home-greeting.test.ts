import { describe, expect, it } from "bun:test";
import { homeGreetingForHour, homeGreetingName } from "./home-greeting";

describe("home greeting identity", () => {
	it("uses the edited account name when the token has no name or an old one", () => {
		expect(homeGreetingName(undefined, { name: "Felix Schultz" }, {})).toBe(
			"Felix",
		);
		expect(
			homeGreetingName(
				undefined,
				{ name: "Ada Lovelace" },
				{ given_name: "Old name" },
			),
		).toBe("Ada");
	});
	it("keeps custom profile greetings and normalizes whitespace", () => {
		expect(homeGreetingName("  Team   Atlas  ", { name: "Felix" })).toBe(
			"Team Atlas",
		);
		expect(homeGreetingName(" ", { name: "  Felix  Schultz " })).toBe("Felix");
		expect(homeGreetingName("{name}", { name: "Felix" })).toBe("Felix");
	});
	it("uses cached claims or human handles while offline", () => {
		expect(
			homeGreetingName(undefined, undefined, { given_name: "Renée" }),
		).toBe("Renée");
		expect(homeGreetingName(undefined, { preferred_username: "river" })).toBe(
			"river",
		);
		expect(
			homeGreetingName(undefined, undefined, {
				email: "felix.schultz@example.com",
			}),
		).toBe("Felix");
	});
	it("does not greet anonymous users with account identifiers or relay addresses", () => {
		expect(homeGreetingName()).toBeUndefined();
		expect(
			homeGreetingName(undefined, {
				name: "google_1234567890123456789",
				username: "google_1234567890123456789",
			}),
		).toBeUndefined();
		expect(
			homeGreetingName(undefined, undefined, {
				email: "abcdef@privaterelay.appleid.com",
			}),
		).toBeUndefined();
	});
	it("preserves names in non-Latin scripts", () => {
		expect(homeGreetingName(undefined, { name: "李小龍" })).toBe("李小龍");
		expect(homeGreetingName(undefined, undefined, { given_name: "ليلى" })).toBe(
			"ليلى",
		);
	});
	it("uses local clock boundaries for each greeting", () => {
		expect(homeGreetingForHour(0)).toBe("Good morning");
		expect(homeGreetingForHour(11)).toBe("Good morning");
		expect(homeGreetingForHour(12)).toBe("Good afternoon");
		expect(homeGreetingForHour(17)).toBe("Good afternoon");
		expect(homeGreetingForHour(18)).toBe("Good evening");
		expect(homeGreetingForHour(23)).toBe("Good evening");
	});
});
