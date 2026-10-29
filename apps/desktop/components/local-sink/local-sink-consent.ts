export type LocalSinkConsentRememberScope = "none" | "event" | "app";

/** "dismiss" is the dialog closed without a choice; the save is abandoned. */
export type LocalSinkConsentAnswer = "allow" | "decline" | "dismiss";

/** The dialog a request opens with: the approval, or the token the trigger signs in with. */
export type LocalSinkConsentStep = "consent" | "token";

export type LocalSinkConsentRequest = {
	appId: string;
	eventId: string;
	eventName: string;
	eventType: string;
	requestId: string;
	/** The trigger starts on this device anew, so the user approves it first. */
	approve: boolean;
	/** The trigger signs in to the hub and holds no token of the signed-in account. */
	needsToken: boolean;
	/** The app whose trigger for this event runs here now; approving moves it. */
	movesFromApp?: string;
	/** "Add token" on a trigger registered here: no save follows, so only a chosen token or Cancel answers it. */
	repair?: boolean;
};

export type LocalSinkConsentResult = {
	answer: LocalSinkConsentAnswer;
	requestId: string;
	/** The token chosen in the token step, checked against the signed-in account. */
	pat?: string;
};

export type LocalSinkConsentOutcome =
	| { answer: "allow"; pat?: string }
	| { answer: "decline" | "dismiss" };

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
 * Where asking starts, or `null` when nothing needs asking. A remembered
 * approval skips only the approval, and never for a trigger taken over from
 * another app; a trigger without a token still asks for one.
 */
export function firstConsentStep(
	request: Pick<
		LocalSinkConsentRequest,
		"approve" | "needsToken" | "movesFromApp"
	>,
	remembered: boolean,
): LocalSinkConsentStep | null {
	if (request.approve && (!remembered || request.movesFromApp !== undefined)) {
		return "consent";
	}
	return request.needsToken ? "token" : null;
}

/** The code `verify_trigger_token` starts its refusal of another account's token with. */
export const TRIGGER_TOKEN_WRONG_ACCOUNT = "TRIGGER_TOKEN_WRONG_ACCOUNT:";

/** Whether `verify_trigger_token` refused a token of another account. */
export function isWrongAccountRefusal(reason: string): boolean {
	return reason.startsWith(TRIGGER_TOKEN_WRONG_ACCOUNT);
}

function outcomeOf(result: LocalSinkConsentResult): LocalSinkConsentOutcome {
	if (result.answer !== "allow") return { answer: result.answer };
	return result.pat
		? { answer: "allow", pat: result.pat }
		: { answer: "allow" };
}

/**
 * Asks before an event starts a trigger on this device, and for the token it
 * signs in with. Nothing to ask answers "allow" at once; without a window to
 * show the dialog the answer is "dismiss", so nothing registers unattended.
 */
export function requestLocalSinkConsent(
	request: Omit<LocalSinkConsentRequest, "requestId">,
): Promise<LocalSinkConsentOutcome> {
	const step = firstConsentStep(request, hasLocalSinkConsent(request));
	if (!step) return Promise.resolve({ answer: "allow" });
	if (typeof window === "undefined")
		return Promise.resolve({ answer: "dismiss" });

	const requestId = safeRandomId();
	return new Promise((resolve) => {
		const onResult = (event: Event) => {
			const result = (event as CustomEvent<LocalSinkConsentResult>).detail;
			if (result.requestId !== requestId) return;
			window.removeEventListener(LOCAL_SINK_CONSENT_RESULT, onResult);
			resolve(outcomeOf(result));
		};
		window.addEventListener(LOCAL_SINK_CONSENT_RESULT, onResult);
		window.dispatchEvent(
			new CustomEvent<LocalSinkConsentRequest>(LOCAL_SINK_CONSENT_REQUIRED, {
				detail: { ...request, approve: step === "consent", requestId },
			}),
		);
	});
}
