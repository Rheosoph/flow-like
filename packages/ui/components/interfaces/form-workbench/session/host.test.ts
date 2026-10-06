import { describe, expect, test } from "bun:test";
import {
	MAX_ATTACHMENT_BYTES,
	MAX_REQUEST_BYTES,
} from "../../../../lib/service-runtime/backend";
import {
	FORM_LIMITS,
	type HostCapabilityInput,
	type ResolveHostCapabilities,
	type ResolveViewerHabits,
} from "../contracts";
import {
	HOSTED_ROOM_BYTES,
	SERVICE_FILE_BYTES,
	SERVICE_ROOM_BYTES,
	formHostKindOf,
	resolveHostCapabilities,
	resolveViewerHabits,
	viewerInputOf,
} from "./host";

const _hosts: ResolveHostCapabilities = resolveHostCapabilities;
const _viewer: ResolveViewerHabits = resolveViewerHabits;
void _hosts;
void _viewer;

const uploadHelper: HostCapabilityInput["helperState"] = {
	fileToUrl: async () => "data:,",
	fileToTemporaryFile: async () => ({ url: "" }),
	filesToTemporaryFiles: async () => [],
};

/** The hosted link's helper: EmptyHelperState's throwing fileToTemporaryFile, no bulk upload. */
const hostedHelper: HostCapabilityInput["helperState"] = {
	fileToUrl: async () => "data:,",
	fileToTemporaryFile: () => {
		throw new Error("Not implemented");
	},
};

function input(parts: Partial<HostCapabilityInput>): HostCapabilityInput {
	return {
		kind: "app",
		presentation: "page",
		helperState: uploadHelper,
		hasToolbar: true,
		signedIn: true,
		memoryScope: "profile:local",
		...parts,
	};
}

describe("resolveHostCapabilities", () => {
	test("desktop app: temporary uploads with FlowPaths, device memory, next files, cancel", () => {
		expect(resolveHostCapabilities(input({}))).toEqual({
			kind: "app",
			presentation: "page",
			persistence: "device",
			memoryScope: "profile:local",
			uploads: "temporary",
			flowPathFiles: true,
			nextFiles: true,
			stop: "cancel",
			inlineFileLimitBytes: null,
			inlineRoomBytes: null,
			warnFileBytes: FORM_LIMITS.warnFileBytes,
			fixedTarget: null,
			hasToolbar: true,
		});
	});

	test("signed-out web: no FlowPath files, session memory, the tile presentation kept", () => {
		const host = resolveHostCapabilities(
			input({ signedIn: false, memoryScope: null, presentation: "tile" }),
		);
		expect(host.flowPathFiles).toBe(false);
		expect(host.uploads).toBe("temporary");
		expect(host.persistence).toBe("session");
		expect(host.memoryScope).toBeNull();
		expect(host.presentation).toBe("tile");
	});

	test("an app helper without bulk uploads sends files inline at dispatch", () => {
		const host = resolveHostCapabilities(
			input({ helperState: { fileToUrl: async () => "data:," } }),
		);
		expect(host.uploads).toBe("inline");
		expect(host.flowPathFiles).toBe(false);
		expect(host.nextFiles).toBe(true);
	});

	test("hosted link: decided by the kind, never by the throwing helper method", () => {
		const host = resolveHostCapabilities(
			input({
				kind: "hosted",
				helperState: hostedHelper,
				signedIn: true,
				memoryScope: "user:someone",
				hasToolbar: false,
			}),
		);
		expect(host).toEqual({
			kind: "hosted",
			presentation: "page",
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
			hasToolbar: false,
		});
		expect(HOSTED_ROOM_BYTES).toBe(
			Math.floor(((2 * 1024 * 1024 - 64 * 1024) * 3) / 4),
		);
	});

	test("service page: 3.5 MB per file, 7.1 MB per run, next files, runs on the device", () => {
		const host = resolveHostCapabilities(
			input({ kind: "service", helperState: { fileToUrl: async () => "" } }),
		);
		expect(host.uploads).toBe("inline");
		expect(host.flowPathFiles).toBe(false);
		expect(host.nextFiles).toBe(true);
		expect(host.stop).toBe("cancel");
		expect(host.fixedTarget).toBe("local");
		expect(host.persistence).toBe("session");
		expect(host.inlineFileLimitBytes).toBe(SERVICE_FILE_BYTES);
		expect(SERVICE_FILE_BYTES).toBe(MAX_ATTACHMENT_BYTES);
		expect(MAX_ATTACHMENT_BYTES).toBe(3_735_552);
		expect(SERVICE_ROOM_BYTES).toBe(
			Math.floor(((MAX_REQUEST_BYTES - 512 * 1024) * 3) / 4),
		);
		expect(host.inlineRoomBytes).toBe(SERVICE_ROOM_BYTES);
	});
});

describe("formHostKindOf", () => {
	test("the prop wins; a Devices runtime namespace is a service page; else the app", () => {
		expect(formHostKindOf("hosted", "device-runtime:abc")).toBe("hosted");
		expect(formHostKindOf(undefined, "device-runtime:abc")).toBe("service");
		expect(formHostKindOf(undefined, "app-invoice-ai")).toBe("app");
	});
});

describe("resolveViewerHabits", () => {
	test("macOS gets ⌘; day-first dates and a decimal point for en-GB", () => {
		const viewer = resolveViewerHabits({
			platform: "MacIntel",
			language: "en-GB",
		});
		expect(viewer.mac).toBe(true);
		expect(viewer.locale).toBe("en-GB");
		expect(viewer.dateLocale.order).toBe("dmy");
		expect(viewer.decimalSign).toBe(".");
	});

	test("Windows gets Ctrl; en-US reads month first", () => {
		const viewer = resolveViewerHabits({
			platform: "Win32",
			language: "en-US",
		});
		expect(viewer.mac).toBe(false);
		expect(viewer.dateLocale.order).toBe("mdy");
	});

	test("German viewers type a decimal comma; an iPad counts as Apple", () => {
		const viewer = resolveViewerHabits({ platform: "iPad", language: "de-DE" });
		expect(viewer.mac).toBe(true);
		expect(viewer.decimalSign).toBe(",");
		expect(viewer.dateLocale.sep).toBe(".");
	});

	test("an unusable language tag falls back to English", () => {
		const viewer = resolveViewerHabits({
			platform: "",
			language: "not a tag!",
		});
		expect(viewer.locale).toBe("en");
		expect(viewer.decimalSign).toBe(".");
	});

	test("viewerInputOf prefers userAgentData's platform and has defaults", () => {
		expect(
			viewerInputOf({
				platform: "MacIntel",
				language: "fr-FR",
				userAgentData: { platform: "macOS" },
			}),
		).toEqual({ platform: "macOS", language: "fr-FR" });
		expect(viewerInputOf(undefined)).toEqual({ platform: "", language: "en" });
	});
});
