/**
 * One place that decides how a user is named, handled and initialed.
 *
 * Federated identity providers put their pool-internal handle in `username`
 * (`google_110293…`, `signinwithapple_001234.9f8e…`). It is never a name, so every
 * surface routes through here instead of picking fields ad hoc. Mirrors
 * `packages/api/src/routes/user/identity.rs`.
 */

import { splitNameSegments } from "./utils";

export interface UserDisplayLike {
	readonly id?: string;
	readonly name?: string | null;
	readonly preferred_username?: string | null;
	readonly username?: string | null;
	readonly email?: string | null;
	/** `IUserLookup` (peers). */
	readonly avatar_url?: string | null;
	/** `IUserInfo` (self). */
	readonly avatar?: string | null;
}

const IDP_HANDLE_PREFIXES = [
	"google_",
	"signinwithapple_",
	"apple_",
	"facebook_",
	"loginwithamazon_",
	"amazon_",
	"microsoft_",
	"azuread_",
	"github_",
	"gitlab_",
	"twitter_",
	"linkedin_",
	"okta_",
	"auth0_",
	"keycloak_",
	"oidc_",
	"saml_",
	"cognito_",
];

const UUID_PATTERN =
	/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

// Kept in lockstep with `is_opaque_identifier` in packages/api/src/routes/user/identity.rs.
function isOpaqueIdentifier(lower: string): boolean {
	if (UUID_PATTERN.test(lower)) return true;

	const alphanumeric = lower.replace(/[^a-z0-9]/g, "").length;
	if (alphanumeric === 0) return true;

	// Google subs are long digit runs.
	if (alphanumeric >= 10 && !/[a-z]/.test(lower)) return true;

	// Apple subs look like `001234.9f8e7d6c5b4a….0123`; Cognito subs are hex blobs.
	return (
		lower.length >= 20 && /^[0-9a-f._-]+$/.test(lower) && /[0-9]/.test(lower)
	);
}

/** True when a value is a provider handle or opaque id, i.e. never showable. */
export function isIdpHandle(value?: string | null): boolean {
	const trimmed = value?.trim();
	if (!trimmed) return true;
	const lower = trimmed.toLowerCase();

	// A provider prefix only means a linked account when what follows is an id
	// rather than words — `apple_pie_lover` is a person, `apple_001234` is not.
	const prefix = IDP_HANDLE_PREFIXES.find((candidate) =>
		lower.startsWith(candidate),
	);
	if (prefix) {
		const suffix = lower.slice(prefix.length);
		return /[0-9]/.test(suffix) || isOpaqueIdentifier(suffix);
	}

	return isOpaqueIdentifier(lower);
}

export function isPrivateRelayEmail(email?: string | null): boolean {
	const lower = email?.trim().toLowerCase();
	if (!lower) return false;
	return (
		lower.endsWith("@privaterelay.appleid.com") ||
		lower.endsWith("@appleid.com")
	);
}

function clean(value?: string | null): string | undefined {
	const collapsed = value?.replace(/\s+/g, " ").trim();
	return collapsed ? collapsed : undefined;
}

/** `felix.schultz@example.com` → `Felix Schultz`. */
export function humanizeEmailLocalPart(
	email?: string | null,
): string | undefined {
	const trimmed = clean(email);
	if (!trimmed || isPrivateRelayEmail(trimmed)) return undefined;

	const local = trimmed.split("@")[0]?.split("+")[0]?.trim();
	if (!local || isOpaqueIdentifier(local.toLowerCase())) return undefined;

	const words = local
		.split(/[._-]/)
		.filter(Boolean)
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1));

	return words.length > 0 ? clean(words.join(" ")) : undefined;
}

/** The handle to render as `@handle`, or undefined when we only have an internal one. */
export function userHandle(user?: UserDisplayLike | null): string | undefined {
	if (!user) return undefined;
	const preferred = clean(user.preferred_username);
	if (preferred && !isIdpHandle(preferred)) return preferred;
	const username = clean(user.username);
	if (username && !isIdpHandle(username)) return username;
	return undefined;
}

/**
 * Best human-readable name. Never returns a provider handle or a raw id, so a
 * caller that wants "something rather than nothing" passes a `fallback`.
 */
export function userDisplayName(
	user?: UserDisplayLike | null,
	fallback = "Unknown user",
): string {
	if (!user) return fallback;
	return (
		clean(user.name) ??
		userHandle(user) ??
		humanizeEmailLocalPart(user.email) ??
		clean(user.email) ??
		fallback
	);
}

/**
 * Secondary line under the name — the handle, or the email when it adds something
 * the name does not already say. Returns undefined when it would merely repeat the
 * primary line, which is what happens for a user who has a handle but no name.
 */
export function userSecondaryLabel(
	user?: UserDisplayLike | null,
): string | undefined {
	if (!user) return undefined;
	const primary = userDisplayName(user, "");

	const handle = userHandle(user);
	if (handle) return handle === primary ? undefined : `@${handle}`;

	const email = clean(user.email);
	if (email && email !== primary) return email;
	return undefined;
}

/** Up to two uppercase letters. `"?"` when there is nothing to work with. */
export function userInitials(
	source?: UserDisplayLike | string | null,
	fallback = "?",
): string {
	const label =
		typeof source === "string"
			? clean(source)
			: clean(userDisplayName(source, ""));
	if (!label) return fallback;

	const words = label.split(/[\s._-]+/).filter(Boolean);
	if (words.length === 0) return fallback;

	const initials =
		words.length === 1
			? words[0].slice(0, 2)
			: `${words[0].charAt(0)}${words[words.length - 1].charAt(0)}`;

	return initials.toUpperCase() || fallback;
}

/** Handles both the peer (`avatar_url`) and self (`avatar`) field names. */
export function userAvatarUrl(
	user?: UserDisplayLike | null,
): string | undefined {
	return clean(user?.avatar_url) ?? clean(user?.avatar);
}

/**
 * Words that name a person in a column: `owner`, `feedback_reporter`, `author_id`.
 *
 * Deliberately excludes words that only sometimes mean a person. `subject` is an
 * email header far more often than a principal, `agent` would drag `user_agent`
 * in, `account` is a tenant or a billing account in most of this product, and
 * `profile` is a settings profile. Singular only: `users` and `assignees` are
 * counts and lists, not one person.
 */
const USER_NOUNS = new Set([
	"user",
	"owner",
	"author",
	"creator",
	"assignee",
	"assigner",
	"reporter",
	"reviewer",
	"requester",
	"requestor",
	"approver",
	"submitter",
	"sender",
	"recipient",
	"editor",
	"uploader",
	"publisher",
	"member",
	"actor",
	"principal",
	"invitee",
	"inviter",
	"moderator",
	"maintainer",
	"contributor",
	"collaborator",
	"participant",
	"commenter",
	"executor",
	"caller",
]);

/**
 * Trailing words that only point at whatever came before them, and so are peeled
 * off before the name is judged: `owner_id`, `reviewed_by_id`, `user_sub`.
 */
const REFERENCE_SUFFIXES = new Set([
	"id",
	"uid",
	"uuid",
	"guid",
	"sub",
	"ref",
	"key",
	"identifier",
]);

/**
 * Words that make a trailing `by` the actor of something rather than a clause.
 * Without this gate every SQL-ish `sort_by`, `group_by` and `partition_by`
 * column would read as a person.
 */
const ACTOR_VERBS = new Set([
	"created",
	"updated",
	"modified",
	"changed",
	"edited",
	"deleted",
	"removed",
	"added",
	"approved",
	"rejected",
	"reviewed",
	"requested",
	"acknowledged",
	"submitted",
	"published",
	"uploaded",
	"invited",
	"granted",
	"revoked",
	"assigned",
	"delegated",
	"owned",
	"opened",
	"closed",
	"resolved",
	"claimed",
	"managed",
	"started",
	"triggered",
	"executed",
	"imported",
	"exported",
	"verified",
	"signed",
	"sent",
	"reported",
	"escalated",
	"transferred",
	"allocated",
	"authored",
	"accepted",
	"declined",
	"cancelled",
	"canceled",
	"archived",
	"restored",
	"locked",
	"unlocked",
	"shared",
	"viewed",
	"seen",
	"run",
	"last",
]);

/**
 * Words that make a trailing `to` a person rather than a destination or a new
 * value, so `assigned_to` resolves while `path_to` and `changed_to` do not.
 */
const HANDOFF_VERBS = new Set([
	"assigned",
	"granted",
	"delegated",
	"allocated",
	"escalated",
	"transferred",
	"reported",
	"addressed",
]);

/**
 * Whether a column name promises an account id.
 *
 * Anchored to the trailing words, like `looksLikeTemporalName`. A trailing `sub`
 * names an account outright; otherwise reference words are peeled off so
 * `owner_id` is judged as `owner` and `reviewed_by_id` as `reviewed_by`, and what
 * is left has to name a person — either directly, or as the actor of a verb.
 * Everything else is left alone, so `username`, `user_email` and `user_agent`
 * stay text, `group_by` and `changed_to` stay clauses, and `app_id`, `board_id`
 * and `session_id` stay ids of things that are not people.
 */
export function looksLikeUserColumnName(name: string): boolean {
	const segments = splitNameSegments(name);
	if (segments.length === 0) return false;

	// `sub` names the claim itself, so it is read before it can be peeled off as
	// a mere reference word: `sub`, `user_sub`, `run_sub`, `target_user_sub`.
	if (segments[segments.length - 1] === "sub") return true;

	while (
		segments.length > 1 &&
		REFERENCE_SUFFIXES.has(segments[segments.length - 1])
	) {
		segments.pop();
	}

	const last = segments[segments.length - 1];
	const previous = segments[segments.length - 2];

	// `created_by`, `last_modified_by`, `reviewed_by_id` — but not `group_by`.
	if (last === "by") return Boolean(previous && ACTOR_VERBS.has(previous));

	// `assigned_to`, `escalated_to`.
	if (last === "to") return Boolean(previous && HANDOFF_VERBS.has(previous));

	// `on_behalf_of`, the sub a principal names as the initiator of a run.
	if (last === "of") return previous === "behalf";

	// `owner`, `assignee`, `feedback_reporter`, `actor_user_id`.
	return USER_NOUNS.has(last);
}

/** Shortest real sub in the wild is a 21-digit Google id; 12 leaves room to spare. */
const MIN_ACCOUNT_ID_LENGTH = 12;

/**
 * Whether a stored value is shaped like an account id and therefore worth
 * resolving against the directory.
 *
 * A sub is by definition unreadable — the same property `isIdpHandle` tests — so
 * anything a human could read (a name, a handle, a repository owner, an email)
 * is left as the text it is rather than spent on a lookup that would 404.
 */
export function looksLikeAccountId(value?: string | null): boolean {
	const trimmed = value?.trim();
	if (!trimmed || trimmed.length < MIN_ACCOUNT_ID_LENGTH) return false;
	// `isIdpHandle` answers "is this unshowable?", for which a run of punctuation
	// qualifies. A redaction mask is not an id, so it is required to carry one.
	if (!/[a-z0-9]/i.test(trimmed)) return false;
	return isIdpHandle(trimmed);
}
