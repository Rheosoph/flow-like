/**
 * One place that decides how a user is named, handled and initialed.
 *
 * Federated identity providers put their pool-internal handle in `username`
 * (`google_110293…`, `signinwithapple_001234.9f8e…`). It is never a name, so every
 * surface routes through here instead of picking fields ad hoc. Mirrors
 * `packages/api/src/routes/user/identity.rs`.
 */

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

function isOpaqueIdentifier(lower: string): boolean {
	if (UUID_PATTERN.test(lower)) return true;
	if (!/[a-z0-9]/.test(lower)) return true;
	if (lower.length >= 10 && !/[a-z]/.test(lower)) return true;
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
 * the name does not already say.
 */
export function userSecondaryLabel(
	user?: UserDisplayLike | null,
): string | undefined {
	if (!user) return undefined;
	const handle = userHandle(user);
	if (handle) return `@${handle}`;

	const email = clean(user.email);
	if (email && email !== userDisplayName(user, "")) return email;
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
