export type LocalSinkConsentRememberScope = "none" | "event" | "app";

/** "dismiss" is the dialog closed without a choice; the save is abandoned. */
export type LocalSinkConsentAnswer = "allow" | "decline" | "dismiss";

export type LocalSinkConsentRequest = {
	appId: string;
	eventId: string;
	eventName: string;
	eventType: string;
	requestId: string;
};

export type LocalSinkConsentResult = {
	answer: LocalSinkConsentAnswer;
	requestId: string;
};

/** What a remembered approval is keyed on: the project, or one event of one trigger type. */
export type LocalSinkConsentTarget = Pick<
	LocalSinkConsentRequest,
	"appId" | "eventId" | "eventType"
>;

export const LOCAL_SINK_CONSENT_REQUIRED = "flow:local-sink-consent-required";
export const LOCAL_SINK_CONSENT_RESULT = "flow:local-sink-consent-result";

function safeRandomId(): string {
	if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
		return crypto.randomUUID();
	}
	return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function consentKey(
	scope: "event" | "app",
	target: LocalSinkConsentTarget,
): string {
	return scope === "app"
		? `local-sink-consent-app-${target.appId}`
		: `local-sink-consent-event-${target.eventId}-${target.eventType}`;
}

/** Whether a remembered approval covers this trigger on this device. */
export function hasLocalSinkConsent(target: LocalSinkConsentTarget): boolean {
	try {
		return (
			localStorage.getItem(consentKey("app", target)) === "1" ||
			localStorage.getItem(consentKey("event", target)) === "1"
		);
	} catch {
		return false;
	}
}

export function saveLocalSinkConsent(
	scope: "event" | "app",
	target: LocalSinkConsentTarget,
): void {
	try {
		localStorage.setItem(consentKey(scope, target), "1");
	} catch {
		// Ignore storage errors.
	}
}

/**
 * Asks before an event starts a trigger on this device. A remembered approval
 * answers "allow" at once; without a window to show the dialog the answer is
 * "dismiss", so nothing registers unattended.
 */
export function requestLocalSinkConsent(
	request: Omit<LocalSinkConsentRequest, "requestId">,
): Promise<LocalSinkConsentAnswer> {
	if (hasLocalSinkConsent(request)) return Promise.resolve("allow");
	if (typeof window === "undefined") return Promise.resolve("dismiss");

	const requestId = safeRandomId();
	return new Promise((resolve) => {
		const onResult = (event: Event) => {
			const result = (event as CustomEvent<LocalSinkConsentResult>).detail;
			if (result.requestId !== requestId) return;
			window.removeEventListener(LOCAL_SINK_CONSENT_RESULT, onResult);
			resolve(result.answer);
		};
		window.addEventListener(LOCAL_SINK_CONSENT_RESULT, onResult);
		window.dispatchEvent(
			new CustomEvent<LocalSinkConsentRequest>(LOCAL_SINK_CONSENT_REQUIRED, {
				detail: { ...request, requestId },
			}),
		);
	});
}
