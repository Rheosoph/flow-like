import { describe, expect, test } from "bun:test";
import {
	humanizeEmailLocalPart,
	isIdpHandle,
	isPrivateRelayEmail,
	userAvatarUrl,
	userDisplayName,
	userHandle,
	userInitials,
	userSecondaryLabel,
} from "./user-display";

describe("isIdpHandle", () => {
	test("flags federated pool handles", () => {
		expect(isIdpHandle("google_110293847561029384756")).toBe(true);
		expect(isIdpHandle("signinwithapple_001234.9f8e7d6c5b4a.0123")).toBe(true);
		expect(isIdpHandle("Google_1102938")).toBe(true);
	});

	test("flags opaque identifiers", () => {
		expect(isIdpHandle("42c52474-5081-70d7-2b23-4bd8c38d8fb0")).toBe(true);
		expect(isIdpHandle("110293847561029384756")).toBe(true);
		expect(isIdpHandle("")).toBe(true);
		expect(isIdpHandle(undefined)).toBe(true);
	});

	test("keeps real handles", () => {
		expect(isIdpHandle("felix")).toBe(false);
		expect(isIdpHandle("felix.schultz")).toBe(false);
		expect(isIdpHandle("googler")).toBe(false);
		expect(isIdpHandle("apple_pie_lover")).toBe(false);
	});
});

describe("userDisplayName", () => {
	test("prefers the real name", () => {
		expect(
			userDisplayName({ name: "Felix Schultz", preferred_username: "felix" }),
		).toBe("Felix Schultz");
	});

	test("falls back to the handle, then the email", () => {
		expect(userDisplayName({ preferred_username: "felix" })).toBe("felix");
		expect(userDisplayName({ email: "felix.schultz@example.com" })).toBe(
			"Felix Schultz",
		);
	});

	test("never surfaces a federated handle", () => {
		expect(
			userDisplayName({
				username: "google_110293847561029384756",
				preferred_username: "google_110293847561029384756",
			}),
		).toBe("Unknown user");
	});

	test("never surfaces a raw sub", () => {
		expect(
			userDisplayName({ id: "42c52474-5081-70d7-2b23-4bd8c38d8fb0" }),
		).toBe("Unknown user");
	});

	test("honours a caller fallback", () => {
		expect(userDisplayName(undefined, "You")).toBe("You");
		expect(userDisplayName({}, "Offline")).toBe("Offline");
	});

	test("keeps a relay address out of the name", () => {
		expect(
			userDisplayName({ email: "kj2h3g4jh2g@privaterelay.appleid.com" }),
		).toBe("kj2h3g4jh2g@privaterelay.appleid.com");
	});
});

describe("userHandle", () => {
	test("prefers preferred_username and skips federated handles", () => {
		expect(userHandle({ preferred_username: "felix", username: "x" })).toBe(
			"felix",
		);
		expect(
			userHandle({ username: "google_110293847561029384756" }),
		).toBeUndefined();
	});
});

describe("userSecondaryLabel", () => {
	test("renders the handle with an @ beneath a real name", () => {
		expect(
			userSecondaryLabel({ name: "Felix Schultz", preferred_username: "felix" }),
		).toBe("@felix");
	});

	test("falls back to the email when it adds information", () => {
		expect(
			userSecondaryLabel({ name: "Felix", email: "felix@example.com" }),
		).toBe("felix@example.com");
	});

	test("adds the email even when the name was derived from it", () => {
		expect(userSecondaryLabel({ email: "ops@example.com" })).toBe(
			"ops@example.com",
		);
	});

	// A handle-only user would otherwise render "felix" over "@felix".
	test("stays quiet when the handle is already the primary line", () => {
		expect(userSecondaryLabel({ preferred_username: "felix" })).toBeUndefined();
		expect(userSecondaryLabel({ username: "felix" })).toBeUndefined();
	});

	test("stays quiet when the email is verbatim the displayed name", () => {
		expect(
			userSecondaryLabel({ email: "kj2h3g4@privaterelay.appleid.com" }),
		).toBeUndefined();
	});
});

describe("userInitials", () => {
	test("uses first and last word", () => {
		expect(userInitials({ name: "Felix Schultz" })).toBe("FS");
		expect(userInitials({ name: "Jean Baptiste Poquelin" })).toBe("JP");
	});

	test("uses two letters for a single word", () => {
		expect(userInitials({ name: "Felix" })).toBe("FE");
	});

	test("accepts a bare string", () => {
		expect(userInitials("felix schultz")).toBe("FS");
	});

	test("falls back when there is nothing to work with", () => {
		expect(userInitials({})).toBe("?");
		expect(userInitials(undefined)).toBe("?");
		expect(userInitials("", "U")).toBe("U");
	});
});

describe("userAvatarUrl", () => {
	test("reads both the peer and self field names", () => {
		expect(userAvatarUrl({ avatar_url: "https://a" })).toBe("https://a");
		expect(userAvatarUrl({ avatar: "https://b" })).toBe("https://b");
		expect(userAvatarUrl({})).toBeUndefined();
	});
});

describe("humanizeEmailLocalPart", () => {
	test("splits and title-cases the local part", () => {
		expect(humanizeEmailLocalPart("felix.schultz@example.com")).toBe(
			"Felix Schultz",
		);
		expect(humanizeEmailLocalPart("felix+news@example.com")).toBe("Felix");
	});

	test("refuses relay and opaque local parts", () => {
		expect(
			humanizeEmailLocalPart("abc@privaterelay.appleid.com"),
		).toBeUndefined();
		expect(
			humanizeEmailLocalPart("110293847561029384756@example.com"),
		).toBeUndefined();
	});
});

describe("isPrivateRelayEmail", () => {
	test("detects Apple relay addresses", () => {
		expect(isPrivateRelayEmail("x@privaterelay.appleid.com")).toBe(true);
		expect(isPrivateRelayEmail("x@example.com")).toBe(false);
	});
});
