import { describe, expect, test } from "bun:test";
import {
	humanizeEmailLocalPart,
	isIdpHandle,
	isPrivateRelayEmail,
	looksLikeAccountId,
	looksLikeUserColumnName,
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
			userSecondaryLabel({
				name: "Felix Schultz",
				preferred_username: "felix",
			}),
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

describe("looksLikeUserColumnName", () => {
	test("claims the columns this product actually stores subs in", () => {
		for (const name of [
			"sub",
			"user_sub",
			"userSub",
			"user_id",
			"userId",
			"target_user_sub",
			"actor",
			"actor_id",
			"actor_user_id",
			"author_id",
			"author_user_id",
			"created_by",
			"createdBy",
			"last_modified_by",
			"run_sub",
			"execution_sub",
			"on_behalf_of",
			"onBehalfOf",
			"added_by_user_id",
			"approved_by_user_id",
			"requested_by_user_id",
			"acknowledged_by_user_id",
			"reviewed_by_id",
			"reviewer_id",
			"invited_by_id",
			"invitee_id",
			"granted_by",
			"revoked_by",
			"creator_user_id",
			"responsible_user_id",
			"assigned_to",
			"remote_principal_id",
			"_flow_user_sub",
			"owner",
			"feedback_reporter",
		]) {
			expect([name, looksLikeUserColumnName(name)]).toEqual([name, true]);
		}
	});

	test("leaves user-ish names that hold something else alone", () => {
		for (const name of [
			// `sub` as a prefix is a different word.
			"subject",
			"subject_alt_names",
			"submission",
			"submission_count",
			"submitted_at",
			"subscriber_count",
			"sub_traces",
			"sub_provider",
			"subtitle",
			// `user` describing something that is not the account.
			"username",
			"user_name",
			"userName",
			"preferred_username",
			"user_email",
			"user_agent",
			"user_type",
			"user_count",
			"user_scoped",
			"unique_users",
			"active_users_weekly",
			"user_password",
			"user_roles",
			"has_user_context",
			"imap_username",
			"twitter_username",
			// ids of things that are not people.
			"id",
			"app_id",
			"board_id",
			"run_id",
			"node_id",
			"event_id",
			"session_id",
			"membership_id",
			"creator_membership_id",
			"owner_role_id",
			"owner_app_id",
			"anon_id",
			"customer_id",
			"agent_id",
			"account_id",
			"account_type",
			"storage_account_name",
			"service_account_json",
			"request_identity",
			"creator_name",
			"authors",
			// SQL clauses are not people.
			"by",
			"sort_by",
			"order_by",
			"group_by",
			"groupBy",
			"filter_by",
			"partition_by",
			"split_by",
			// a trailing `to` that names a destination or a new value.
			"path_to",
			"reply_to",
			"changed_to",
			"moved_to",
			// timestamps stay timestamps.
			"created_at",
			"updated_at",
		]) {
			expect([name, looksLikeUserColumnName(name)]).toEqual([name, false]);
		}
	});

	test("ignores names with nothing to read", () => {
		expect(looksLikeUserColumnName("")).toBe(false);
		expect(looksLikeUserColumnName("___")).toBe(false);
	});
});

describe("looksLikeAccountId", () => {
	test("claims the id shapes the directory can answer for", () => {
		expect(looksLikeAccountId("42c52474-5081-70d7-2b23-4bd8c38d8fb0")).toBe(
			true,
		);
		expect(looksLikeAccountId("110293847561029384756")).toBe(true);
		expect(looksLikeAccountId("google_110293847561029384756")).toBe(true);
		expect(looksLikeAccountId("signinwithapple_001234.9f8e7d6c5b4a.0123")).toBe(
			true,
		);
		expect(looksLikeAccountId(" 42c52474-5081-70d7-2b23-4bd8c38d8fb0 ")).toBe(
			true,
		);
	});

	test("leaves readable values as the text they are", () => {
		expect(looksLikeAccountId("felix")).toBe(false);
		expect(looksLikeAccountId("felix.schultz")).toBe(false);
		expect(looksLikeAccountId("system")).toBe(false);
		expect(looksLikeAccountId("microsoft")).toBe(false);
		expect(looksLikeAccountId("info@tm9657.de")).toBe(false);
		expect(looksLikeAccountId("2026-08-22T10:30:00Z")).toBe(false);
		expect(looksLikeAccountId("Northstar Labs")).toBe(false);
		expect(looksLikeAccountId("ACC-88")).toBe(false);
	});

	test("rejects nothing and near-nothing", () => {
		expect(looksLikeAccountId("")).toBe(false);
		expect(looksLikeAccountId(null)).toBe(false);
		expect(looksLikeAccountId(undefined)).toBe(false);
		// Short opaque strings are ids of something, but never of an account.
		expect(looksLikeAccountId("ab-12")).toBe(false);
	});

	test("rejects redaction masks, which are unshowable but are not ids", () => {
		expect(looksLikeAccountId("------------")).toBe(false);
		expect(looksLikeAccountId("************")).toBe(false);
		expect(looksLikeAccountId("____________")).toBe(false);
	});
});
