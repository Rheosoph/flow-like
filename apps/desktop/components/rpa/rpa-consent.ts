import { invoke } from "@tauri-apps/api/core";

export type RpaConsentContext = "execution" | "event_registration";
export type RpaConsentRememberScope = "none" | "board" | "event";

export type RpaConsentRequest = {
	appId: string;
	boardId: string;
	context: RpaConsentContext;
	eventId?: string;
	requestId: string;
};

export type RpaConsentResult = {
	granted: boolean;
	requestId: string;
};

type RpaPermissionStatus = {
	accessibility: boolean;
	screen_recording: boolean;
};

type RpaSystemPermissionRequest = {
	appId?: string;
	boardId?: string;
	checkError?: string;
	eventId?: string;
	permissions?: RpaPermissionStatus;
	requestId: string;
};

type RpaSystemPermissionResult = {
	granted: boolean;
	requestId: string;
};

function safeRandomId(): string {
	if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
		return crypto.randomUUID();
	}
	return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function rpaConsentKey(scope: "board" | "event", id: string): string {
	return `rpa-consent-${scope}-${id}`;
}

export function hasRpaAutomationConsent(
	boardId: string,
	eventId?: string,
): boolean {
	try {
		if (localStorage.getItem(rpaConsentKey("board", boardId)) === "1") {
			return true;
		}
		return eventId
			? localStorage.getItem(rpaConsentKey("event", eventId)) === "1"
			: false;
	} catch {
		return false;
	}
}

export function saveRpaAutomationConsent(
	scope: "board" | "event",
	id: string,
): void {
	try {
		localStorage.setItem(rpaConsentKey(scope, id), "1");
	} catch {
		// Ignore storage errors.
	}
}

export function requestRpaAutomationConsent({
	appId,
	boardId,
	context,
	eventId,
}: Omit<RpaConsentRequest, "requestId">): Promise<boolean> {
	if (hasRpaAutomationConsent(boardId, eventId)) {
		return Promise.resolve(true);
	}

	if (typeof window === "undefined") {
		return Promise.resolve(false);
	}

	const requestId = safeRandomId();

	return new Promise((resolve) => {
		const onResult = (event: Event) => {
			const result = (event as CustomEvent<RpaConsentResult>).detail;
			if (result.requestId !== requestId) return;

			window.removeEventListener("flow:rpa-consent-result", onResult);
			resolve(result.granted);
		};

		window.addEventListener("flow:rpa-consent-result", onResult);
		window.dispatchEvent(
			new CustomEvent<RpaConsentRequest>("flow:rpa-consent-required", {
				detail: {
					appId,
					boardId,
					context,
					eventId,
					requestId,
				},
			}),
		);
	});
}

export async function ensureRpaSystemPermissions(
	context: Omit<
		RpaSystemPermissionRequest,
		"checkError" | "permissions" | "requestId"
	> = {},
): Promise<boolean> {
	let status: RpaPermissionStatus | undefined;
	let checkError: string | undefined;

	try {
		status = await invoke<RpaPermissionStatus>("check_rpa_permissions");
		if (status.accessibility && status.screen_recording) return true;
	} catch (error) {
		checkError = error instanceof Error ? error.message : String(error);
	}

	if (typeof window === "undefined") return false;

	const requestId = safeRandomId();

	return new Promise((resolve) => {
		const onResult = (event: Event) => {
			const result = (event as CustomEvent<RpaSystemPermissionResult>).detail;
			if (result.requestId !== requestId) return;

			window.removeEventListener("flow:rpa-permissions-result", onResult);
			resolve(result.granted);
		};

		window.addEventListener("flow:rpa-permissions-result", onResult);
		window.dispatchEvent(
			new CustomEvent<RpaSystemPermissionRequest>(
				"flow:rpa-permissions-required",
				{
					detail: {
						...context,
						permissions: status,
						checkError,
						requestId,
					},
				},
			),
		);
	});
}
