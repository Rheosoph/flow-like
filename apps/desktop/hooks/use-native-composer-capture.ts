import { captureFileName } from "@flow-like/flow-like-ui/lib/capture-file-name";
import { readLocationForUserContext } from "@flow-like/flow-like-ui/lib/device-bridge";
import {
	type ComposerAttachResult,
	type ComposerAttachSource,
	useComposerCaptureStore,
} from "@flow-like/flow-like-ui/state/composer-capture";
import { i18n } from "@flow-like/locales";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { readNativeSharedFiles } from "../lib/native-shared-files";
import { isIOSDevice, isMobileDevice } from "../lib/platform";

export interface NativeCaptureCapabilities {
	readonly document?: boolean;
	/** Android without ML Kit falls back to a file picker, which the composer already offers. */
	readonly documentMode?: "scanner" | "picker";
	readonly photo?: boolean;
	/** VisionKit's live text and code scanner (iOS 16+ on supported hardware). */
	readonly liveText?: boolean;
}

interface NativeCaptureResult {
	readonly files?: string[];
	readonly text?: string;
	readonly cancelled?: boolean;
}

type NativeCaptureOperation = "document" | "photo" | "live_text";

/** Native failures cross the bridge as `"<code>: <message>"`. */
export function splitNativeError(error: unknown): {
	code: string;
	message: string;
} {
	const text = error instanceof Error ? error.message : String(error);
	const separator = text.indexOf(": ");
	return separator > 0
		? { code: text.slice(0, separator), message: text.slice(separator + 2) }
		: { code: "", message: text };
}

/** Runs a native capture; `undefined` when the person cancelled it. */
async function nativeCapture(
	scope: string,
	operation: NativeCaptureOperation,
): Promise<NativeCaptureResult | undefined> {
	try {
		const result = await invoke<NativeCaptureResult>("native_bot_request", {
			requestId: crypto.randomUUID(),
			options: { scope, operation },
		});
		return result.cancelled ? undefined : result;
	} catch (error) {
		const { code, message } = splitNativeError(error);
		if (code === "cancelled") return undefined;
		throw new Error(message);
	}
}

/** The inbox prefixes stored files with UUIDs; give the composer a readable name instead. */
function renameCaptures(files: File[], prefix: string): File[] {
	const at = new Date();
	return files.map(
		(file, index) =>
			new File(
				[file],
				captureFileName(
					prefix,
					at,
					/\.[a-z0-9]+$/i.exec(file.name)?.[0] ?? "",
					files.length > 1 ? index + 1 : undefined,
				),
				{ type: file.type },
			),
	);
}

async function captureFiles(
	scope: string,
	operation: NativeCaptureOperation,
	prefix: string,
): Promise<ComposerAttachResult> {
	const result = await nativeCapture(scope, operation);
	if (!result) return {};
	return {
		files: renameCaptures(
			await readNativeSharedFiles(result.files ?? []),
			prefix,
		),
	};
}

async function captureText(
	scope: string,
	operation: NativeCaptureOperation,
): Promise<ComposerAttachResult> {
	const result = await nativeCapture(scope, operation);
	if (!result) return {};
	const text = result.text?.trim();
	if (!text)
		throw new Error(
			i18n.t(
				"captureFoundNoText",
				"No readable text was found. Try again with better light.",
			),
		);
	return { text };
}

export function locationText(
	latitude: number,
	longitude: number,
	accuracy: number,
	apple: boolean,
): string {
	const lat = latitude.toFixed(5);
	const lon = longitude.toFixed(5);
	const link = apple
		? `https://maps.apple.com/?ll=${lat},${lon}&q=${lat},${lon}`
		: `https://www.openstreetmap.org/?mlat=${lat}&mlon=${lon}#map=17/${lat}/${lon}`;
	return `${i18n.t(
		"myLocationVal",
		"My location: {{coordinates}} (±{{accuracy}} m)",
		{
			coordinates: `${lat}, ${lon}`,
			accuracy: Math.round(accuracy),
		},
	)}\n${link}`;
}

async function captureLocation(): Promise<ComposerAttachResult> {
	const fix = await readLocationForUserContext(
		{ highAccuracy: true, maximumAgeMs: 0, timeoutMs: 20_000 },
		{ appId: "chat", deadline: Date.now() + 25_000, userInitiated: true },
	);
	return {
		text: locationText(
			fix.latitude,
			fix.longitude,
			fix.accuracy,
			isIOSDevice(),
		),
	};
}

/** The device sources a composer offers, given what the native layer reports. */
export function nativeComposerSources(
	scope: string,
	capabilities: NativeCaptureCapabilities,
	mobile: boolean,
): ComposerAttachSource[] {
	const sources: ComposerAttachSource[] = [];
	if (capabilities.document && capabilities.documentMode !== "picker") {
		sources.push(
			{
				kind: "scan_document",
				attach: () => captureFiles(scope, "document", "scan"),
			},
			{ kind: "scan_text", attach: () => captureText(scope, "document") },
		);
	}
	if (capabilities.liveText)
		sources.push({
			kind: "live_text",
			attach: () => captureText(scope, "live_text"),
		});
	if (capabilities.photo)
		sources.push({
			kind: "take_photo",
			attach: () => captureFiles(scope, "photo", "photo"),
		});
	if (mobile) sources.push({ kind: "location", attach: captureLocation });
	return sources;
}

/** Offers the device's scanner, camera and location in every chat composer for `scope`. */
export function useNativeComposerCapture(scope: string | undefined): void {
	useEffect(() => {
		if (!scope || !isTauri()) return;
		let active = true;
		let unregister: (() => void) | undefined;
		void invoke<NativeCaptureCapabilities>("native_bot_capabilities")
			.then((capabilities) => {
				if (!active) return;
				const sources = nativeComposerSources(
					scope,
					capabilities,
					isMobileDevice(),
				);
				if (sources.length > 0)
					unregister = useComposerCaptureStore.getState().register(sources);
			})
			.catch(() => {});
		return () => {
			active = false;
			unregister?.();
		};
	}, [scope]);
}
