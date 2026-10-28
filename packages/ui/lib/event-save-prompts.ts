function carriesFlag(error: unknown, flag: string): boolean {
	return (
		typeof error === "object" &&
		error !== null &&
		(error as Record<string, unknown>)[flag] === true
	);
}

/** The desktop rejects a save with this flag when its consent or token prompt was dismissed. */
export function isLocalTriggerDismissal(error: unknown): boolean {
	return carriesFlag(error, "isLocalSinkConsentDismissed");
}

/**
 * Whether the user stopped a save at a prompt the desktop shows before it
 * writes anything: the trigger's consent or token, or the approval of the
 * computer automation its board runs.
 */
export function isSavePromptRefusal(error: unknown): boolean {
	return (
		isLocalTriggerDismissal(error) || carriesFlag(error, "isRpaConsentError")
	);
}
