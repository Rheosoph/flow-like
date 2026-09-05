import type { QueryClient } from "@tanstack/react-query";
import type { IUserInfo } from "../../state/backend-state/user-state";

export interface AccountDraft {
	username: string;
	name: string;
	description: string;
}

export function accountDraft(info?: Partial<IUserInfo>): AccountDraft {
	return {
		username: info?.preferred_username ?? "",
		name: info?.name ?? "",
		description: info?.description ?? "",
	};
}

/** Keep edits while an independent email or photo update refreshes the account. */
export function mergeAccountDraft(
	draft: AccountDraft,
	previous: AccountDraft,
	next: AccountDraft,
): AccountDraft {
	return Object.fromEntries(
		(Object.keys(next) as (keyof AccountDraft)[]).map((key) => [
			key,
			draft[key] === previous[key] ? next[key] : draft[key],
		]),
	) as unknown as AccountDraft;
}

export function accountHasChanges(draft: AccountDraft, saved: AccountDraft) {
	return (Object.keys(saved) as (keyof AccountDraft)[]).some(
		(key) => draft[key] !== saved[key],
	);
}

export async function invalidateAccountIdentity(client: QueryClient) {
	await client.invalidateQueries({
		predicate: ({ queryKey }) =>
			["getInfo", "lookupUser", "lookupUsers", "lookupUserBatched"].includes(
				String(queryKey[0]),
			),
	});
}

export function accountError(error: unknown, fallback: string): string {
	const name = error instanceof Error ? error.name : "";
	switch (name) {
		case "InvalidPasswordException":
			return "This password does not meet your account's password requirements. Try a longer password with uppercase and lowercase letters, a number, and a symbol.";
		case "PasswordHistoryPolicyViolationException":
			return "You have used this password before. Choose a different password.";
		case "NotAuthorizedException":
			return "Your session or current password could not be verified. Check your current password, or sign in again and retry.";
		case "UserUnAuthenticatedException":
		case "AccountSessionExpired":
			return "Your session has expired. Sign in again and retry.";
		case "CodeMismatchException":
			return "That confirmation code is incorrect. Check the code and try again.";
		case "ExpiredCodeException":
			return "That confirmation code has expired. Request a new code.";
		case "AliasExistsException":
			return "That email address or username is already in use. Choose another one.";
		case "LimitExceededException":
		case "TooManyRequestsException":
			return "Too many attempts. Wait a moment before trying again.";
		case "CodeDeliveryFailureException":
			return "The confirmation email could not be delivered. Check the address and try again.";
		case "NetworkError":
		case "TypeError":
			return "The request could not be completed. Check your connection and try again.";
		default:
			return fallback;
	}
}
