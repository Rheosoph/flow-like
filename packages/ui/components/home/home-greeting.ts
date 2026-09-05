import {
	type UserDisplayLike,
	humanizeEmailLocalPart,
	isIdpHandle,
	userHandle,
} from "../../lib/user-display";

interface GreetingClaims {
	given_name?: string;
	name?: string;
	preferred_username?: string;
	nickname?: string;
	email?: string;
}

function readable(value?: string | null) {
	const name = value?.replace(/\s+/g, " ").trim();
	return name &&
		(!isIdpHandle(name) || /\p{L}/u.test(name.replace(/[a-z]/gi, ""))) &&
		!/^\{+\s*name\s*\}+$/i.test(name)
		? name
		: undefined;
}

/** Account edits take precedence over older identity-token claims. */
export function homeGreetingName(
	configured?: string,
	account?: UserDisplayLike | null,
	claims?: GreetingClaims | null,
): string | undefined {
	const override = readable(configured);
	if (override) return override;
	for (const candidate of [account?.name, claims?.given_name, claims?.name]) {
		const name = readable(candidate);
		if (name) return name.split(" ")[0];
	}
	return (
		readable(userHandle(account)) ??
		readable(
			userHandle({
				preferred_username: claims?.preferred_username,
				username: claims?.nickname,
			}),
		) ??
		humanizeEmailLocalPart(account?.email)?.split(" ")[0] ??
		humanizeEmailLocalPart(claims?.email)?.split(" ")[0]
	);
}

export function homeGreetingForHour(hour: number) {
	return hour < 12
		? "Good morning"
		: hour < 18
			? "Good afternoon"
			: "Good evening";
}
