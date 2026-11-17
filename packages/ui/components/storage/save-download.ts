import { i18n } from "@flow-like/locales";
import { toast } from "sonner";
import { isIOS } from "../../lib/platform";
import { mimeTypeForName } from "./storage-attach-limits";

/** Safari cancels a download whose object URL is revoked in the same task as the click. */
const REVOKE_DELAY_MS = 60_000;

export async function fetchFileBlob(url: string, fileName: string) {
	const response = await fetch(url);
	if (!response.ok) {
		throw new Error(
			`Downloading ${fileName} failed with HTTP ${response.status}`,
		);
	}
	return response.blob();
}

/**
 * Saves through an object URL, since browsers ignore `download` on
 * cross-origin links and would navigate to the object instead.
 */
export function saveBlobAsFile(blob: Blob, fileName: string) {
	const objectUrl = URL.createObjectURL(
		new Blob([blob], { type: "application/octet-stream" }),
	);
	const anchor = document.createElement("a");
	anchor.href = objectUrl;
	anchor.download = fileName;
	document.body.appendChild(anchor);
	anchor.click();
	anchor.remove();
	setTimeout(() => URL.revokeObjectURL(objectUrl), REVOKE_DELAY_MS);
}

const isDomError = (error: unknown, name: string) =>
	error instanceof DOMException && error.name === name;

/**
 * On iOS the share sheet is how a file reaches Files ("Save to Files"). The
 * app's WKWebView has no download manager at all, so `<a download>` and
 * `window.open` do nothing there. Returns false where files cannot be shared.
 */
export async function shareFileOnIOS(
	blob: Blob,
	fileName: string,
): Promise<boolean> {
	if (!isIOS() || typeof navigator.share !== "function") return false;
	const file = new File([blob], fileName, {
		type: mimeTypeForName(fileName, blob.type),
	});
	if (!navigator.canShare?.({ files: [file] })) return false;
	try {
		await navigator.share({ files: [file] });
	} catch (error) {
		if (isDomError(error, "AbortError")) return true;
		if (!isDomError(error, "NotAllowedError")) throw error;
		// The tap that asked for the file expired while it downloaded; the
		// toast's button is a fresh tap.
		toast(
			i18n.t("common:fileReadyToSave", "{{name}} is ready", {
				name: fileName,
			}),
			{
				action: {
					label: i18n.t("common:save", "Save"),
					onClick: () => {
						navigator.share({ files: [file] }).catch((retryError) => {
							if (!isDomError(retryError, "AbortError"))
								console.warn("Sharing the file failed:", retryError);
						});
					},
				},
			},
		);
	}
	return true;
}

export async function saveUrlAsFile(url: string, fileName: string) {
	const blob = await fetchFileBlob(url, fileName);
	if (await shareFileOnIOS(blob, fileName)) return;
	saveBlobAsFile(blob, fileName);
}
