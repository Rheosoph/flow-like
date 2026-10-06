/*
 * What the host can do (PLAN §6) and how the viewer reads dates and numbers (spec M4, S3). Decided
 * from the host kind first, never from which helper methods exist: the hosted helper inherits a
 * throwing `fileToTemporaryFile` from EmptyHelperState. Pure.
 */
import {
	MAX_ATTACHMENT_BYTES,
	MAX_REQUEST_BYTES,
} from "../../../../lib/service-runtime/backend";
import { isRuntimeNamespace } from "../../../../lib/service-runtime/session-scope";
import {
	FORM_LIMITS,
	type FormHostKind,
	type HostCapabilities,
	type HostCapabilityInput,
	type ResolveHostCapabilities,
	type ResolveViewerHabits,
	type ViewerInput,
} from "../contracts";
import { dateLocaleOf } from "../model/dates";
import { decimalSignOf } from "../model/numbers";

/** Bytes of files one inline run can carry: (request − reserve) × 3/4 (`flpRoom`). */
export const inlineRoomOf = (requestBytes: number, reserveBytes: number) =>
	Math.floor(((requestBytes - reserveBytes) * 3) / 4);

/** A hosted link: axum's 2 MB body limit on /a/{app_id}/invoke leaves 1.4 MB for files. */
export const HOSTED_ROOM_BYTES = inlineRoomOf(
	FORM_LIMITS.hostedRequestBytes,
	FORM_LIMITS.hostedReserveBytes,
);

/** The device page: (MAX_REQUEST_BYTES − 512 KiB) × 3/4, 7.1 MB per run. */
export const SERVICE_ROOM_BYTES = inlineRoomOf(
	MAX_REQUEST_BYTES,
	FORM_LIMITS.serviceReserveBytes,
);

/** The device page refuses one file above MAX_ATTACHMENT_BYTES (3.5 MB), never above the room of a run. */
export const SERVICE_FILE_BYTES = Math.min(
	MAX_ATTACHMENT_BYTES,
	SERVICE_ROOM_BYTES,
);

type Resolve = (input: HostCapabilityInput) => HostCapabilities;

const appHost: Resolve = (input) => {
	const temporary =
		typeof input.helperState.filesToTemporaryFiles === "function";
	return {
		kind: "app",
		presentation: input.presentation,
		persistence: input.memoryScope ? "device" : "session",
		memoryScope: input.memoryScope || null,
		uploads: temporary ? "temporary" : "inline",
		flowPathFiles: temporary && input.signedIn,
		nextFiles: true,
		stop: "cancel",
		inlineFileLimitBytes: null,
		inlineRoomBytes: null,
		warnFileBytes: FORM_LIMITS.warnFileBytes,
		fixedTarget: null,
		hasToolbar: input.hasToolbar,
	};
};

const hostedHost: Resolve = (input) => ({
	kind: "hosted",
	presentation: input.presentation,
	persistence: "session",
	memoryScope: null,
	uploads: "inline",
	flowPathFiles: false,
	nextFiles: false,
	stop: "detach",
	inlineFileLimitBytes: HOSTED_ROOM_BYTES,
	inlineRoomBytes: HOSTED_ROOM_BYTES,
	warnFileBytes: null,
	fixedTarget: "remote",
	hasToolbar: input.hasToolbar,
});

const serviceHost: Resolve = (input) => ({
	kind: "service",
	presentation: input.presentation,
	persistence: "session",
	memoryScope: null,
	uploads: "inline",
	flowPathFiles: false,
	nextFiles: true,
	stop: "cancel",
	inlineFileLimitBytes: SERVICE_FILE_BYTES,
	inlineRoomBytes: SERVICE_ROOM_BYTES,
	warnFileBytes: null,
	fixedTarget: "local",
	hasToolbar: input.hasToolbar,
});

const RESOLVERS: Readonly<Record<FormHostKind, Resolve>> = {
	app: appHost,
	hosted: hostedHost,
	service: serviceHost,
};

/**
 * App host: temporary uploads when the helper has `filesToTemporaryFiles` (else files go inline at
 * dispatch through `fileToUrl`), FlowPath files only when also signed in, device memory with a
 * scope. Hosted and service pages: inline files with their size limits, no FlowPath, session
 * memory, a fixed target; hosted Stop only stops listening and takes one file per run.
 */
export const resolveHostCapabilities: ResolveHostCapabilities = (input) =>
	RESOLVERS[input.kind](input);

/** `FormWorkbenchProps.host`, else `service` for a Devices runtime namespace, else `app`. */
export function formHostKindOf(
	host: FormHostKind | undefined,
	appId: string,
): FormHostKind {
	if (host) return host;
	return isRuntimeNamespace(appId) ? "service" : "app";
}

const MAC_PLATFORM = /mac|iphone|ipad|ipod/i;

/** A BCP 47 tag Intl accepts, else "en". */
function canonicalLocale(language: string) {
	try {
		return Intl.getCanonicalLocales(language.trim())[0] ?? "en";
	} catch {
		return "en";
	}
}

/** ⌘ on Apple platforms; dates, decimal sign and formatting from `navigator.language`, never the app language. */
export const resolveViewerHabits: ResolveViewerHabits = (input) => {
	const locale = canonicalLocale(input.language);
	return {
		mac: MAC_PLATFORM.test(input.platform),
		locale,
		dateLocale: dateLocaleOf(locale),
		decimalSign: decimalSignOf(locale),
	};
};

/** The fields of `navigator` the viewer's habits come from. */
export interface NavigatorLike {
	readonly platform?: string;
	readonly language?: string;
	readonly userAgentData?: { readonly platform?: string } | null;
}

/** `ViewerInput` from `navigator` (or nothing outside a browser): userAgentData's platform first. */
export function viewerInputOf(nav: NavigatorLike | undefined): ViewerInput {
	return {
		platform: nav?.userAgentData?.platform || nav?.platform || "",
		language: nav?.language || "en",
	};
}
