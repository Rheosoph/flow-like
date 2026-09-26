import { invoke } from "@tauri-apps/api/core";

export type RpaCapability =
	| "browser"
	| "clipboard"
	| "application_launch"
	| "input_control"
	| "input_monitoring"
	| "screen_capture"
	| "accessibility"
	| "window_management";
export type RpaCapabilityState =
	| "granted"
	| "not_required"
	| "denied"
	| "unavailable"
	| "unsupported";
export type RpaPermissionStatus = {
	accessibility: boolean;
	screen_recording: boolean;
	input_monitoring: boolean;
	executable_path?: string | null;
	platform: string;
	all_granted: boolean;
	required: RpaCapability[];
	capabilities: Array<{
		capability: RpaCapability;
		state: RpaCapabilityState;
		detail: string;
		can_request: boolean;
	}>;
};
export type RpaConsentContext = "execution" | "event_registration";
export type RpaConsentRememberScope = "none" | "board" | "event";
export type RpaConsentRequest = {
	appId: string;
	boardId: string;
	version?: [number, number, number];
	context: RpaConsentContext;
	eventId?: string;
	requestId: string;
	revision: string;
	identity: string;
	required: RpaCapability[];
};
export type RpaConsentResult = { granted: boolean; requestId: string };
export type RpaSystemPermissionRequest = {
	appId?: string;
	boardId?: string;
	eventId?: string;
	required: RpaCapability[];
	requestId: string;
};
type Requirements = {
	required: RpaCapability[];
	revision: string;
	identity: string;
	approved: boolean;
};

function safeRandomId(): string {
	return (
		globalThis.crypto?.randomUUID?.() ??
		`${Date.now()}-${Math.random().toString(36).slice(2)}`
	);
}

function requestDialog<T extends { requestId: string }>(
	name: string,
	request: T,
): Promise<boolean> {
	if (typeof window === "undefined") return Promise.resolve(false);
	return new Promise((resolve) => {
		let settled = false;
		const finish = (granted: boolean) => {
			if (settled) return;
			settled = true;
			window.removeEventListener(`${name}-result`, onResult);
			window.removeEventListener("pagehide", onUnload);
			resolve(granted);
		};
		const onResult = (event: Event) => {
			const result = (event as CustomEvent<RpaConsentResult>).detail;
			if (result?.requestId === request.requestId)
				finish(result.granted === true);
		};
		const onUnload = () => finish(false);
		window.addEventListener(`${name}-result`, onResult);
		window.addEventListener("pagehide", onUnload);
		window.dispatchEvent(
			new CustomEvent(`${name}-required`, { detail: request }),
		);
	});
}

export async function saveRpaAutomationConsent(
	request: RpaConsentRequest,
	rememberFor: RpaConsentRememberScope,
): Promise<void> {
	await invoke("grant_rpa_automation", {
		appId: request.appId,
		boardId: request.boardId,
		version: request.version,
		eventId: request.eventId,
		expectedRevision: request.revision,
		expectedIdentity: request.identity,
		scope: rememberFor === "none" ? "once" : rememberFor,
	});
}

export async function hasRpaAutomationConsent(
	request: Pick<RpaConsentRequest, "appId" | "boardId" | "version" | "eventId">,
): Promise<boolean> {
	return (await invoke<Requirements>("get_rpa_requirements", request)).approved;
}

export async function requestRpaAutomationConsent(
	request: Omit<
		RpaConsentRequest,
		"requestId" | "revision" | "identity" | "required"
	>,
): Promise<boolean> {
	const requirements = await invoke<Requirements>("get_rpa_requirements", {
		appId: request.appId,
		boardId: request.boardId,
		version: request.version,
		eventId: request.eventId,
	});
	if (requirements.required.length === 0) return true;
	if (!requirements.approved) {
		const granted = await requestDialog("flow:rpa-consent", {
			...request,
			revision: requirements.revision,
			identity: requirements.identity,
			required: requirements.required,
			requestId: safeRandomId(),
		});
		if (!granted) return false;
	}
	return ensureRpaSystemPermissions({
		...request,
		required: requirements.required,
	});
}

export async function ensureRpaSystemPermissions(
	context: Omit<RpaSystemPermissionRequest, "requestId" | "required"> & {
		required?: RpaCapability[];
	} = {},
): Promise<boolean> {
	const required = context.required ?? ["input_control", "screen_capture"];
	if (required.length === 0) return true;
	try {
		const status = await invoke<RpaPermissionStatus>("check_rpa_permissions", {
			required,
		});
		if (status.all_granted === true) return true;
	} catch {
		// The dialog presents the check error and allows retry or cancellation.
	}
	return requestDialog("flow:rpa-permissions", {
		...context,
		required,
		requestId: safeRandomId(),
	});
}
