import { ApiResponseError } from "./api-error";

export const JOIN_MAX_RETRIES = 6;
export const JOIN_BASE_DELAY_MS = 800;

export type JoinFailureKind = "invalid" | "forbidden" | "retry-exhausted";

export interface JoinAttemptResult {
	ok: boolean;
	kind?: JoinFailureKind;
	error?: unknown;
}

/**
 * 4xx responses are verdicts, not glitches — retrying them just burns ~50s of
 * backoff before showing the user a wrong "expired link" message.
 */
export function isTerminalJoinError(error: unknown): boolean {
	return (
		error instanceof ApiResponseError &&
		error.status >= 400 &&
		error.status < 500 &&
		error.status !== 401 &&
		error.status !== 408 &&
		error.status !== 429
	);
}

export function joinFailureMessage(kind: JoinFailureKind): string {
	switch (kind) {
		case "invalid":
			return "This invite link is invalid, expired, or was revoked.";
		case "forbidden":
			return "This invite link can no longer be used — it may have reached its usage limit.";
		default:
			return "Joining did not complete — please check your connection and open the invite link again.";
	}
}

export async function attemptJoinWithRetry(
	join: () => Promise<void>,
	onAttempt?: (attempt: number) => void,
): Promise<JoinAttemptResult> {
	for (let i = 0; i <= JOIN_MAX_RETRIES; i++) {
		onAttempt?.(i);
		try {
			await join();
			return { ok: true };
		} catch (error) {
			if (isTerminalJoinError(error)) {
				const status = (error as ApiResponseError).status;
				return {
					ok: false,
					kind: status === 404 ? "invalid" : "forbidden",
					error,
				};
			}
			if (i === JOIN_MAX_RETRIES) {
				return { ok: false, kind: "retry-exhausted", error };
			}
			await new Promise((resolve) =>
				setTimeout(resolve, JOIN_BASE_DELAY_MS * 2 ** i),
			);
		}
	}
	return { ok: false, kind: "retry-exhausted" };
}
