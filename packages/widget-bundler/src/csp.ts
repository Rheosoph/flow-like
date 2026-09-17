import type { WidgetCapabilities } from "@flow-like/widget-sdk";
import { escapeHtmlAttr, insertAtHeadStart } from "./html";

/**
 * Build the pack-time widget-document CSP: `default-src 'none'` baseline,
 * inline script/style allowed (self-contained documents), chunk loads
 * restricted to the bundle asset origins, and browser features from the
 * contract `capabilities`. Contract `csp` sources are never included: hosts
 * add them to the header they serve, and only after the viewer approves.
 */
export function buildCsp(
	servingPrefix: string | null,
	capabilities: WidgetCapabilities = {},
): string {
	// The bundle hash is a hash of the finished archive, so pack cannot embed a
	// hash-specific serving URL without making the archive self-referential.
	// Allow only the origins used by the supported web and Tauri asset servers;
	// callers may additionally narrow/extend this with a deployment prefix.
	if (
		servingPrefix &&
		(/[\s;"'<>]/.test(servingPrefix) ||
			!/^(?:https?:\/\/|flow-widget:)/.test(servingPrefix))
	) {
		throw new Error("Invalid widget CSP source");
	}
	const assetSources = [
		"'self'",
		"flow-widget:",
		"http://flow-widget.localhost",
		...(servingPrefix ? [servingPrefix] : []),
	].join(" ");
	const connect = capabilities.workers ? `${assetSources} blob:` : "'none'";
	return [
		"default-src 'none'",
		`script-src 'unsafe-inline' ${capabilities.wasm ? "'wasm-unsafe-eval' " : ""}${assetSources}`,
		`style-src 'unsafe-inline' ${assetSources}`,
		`img-src data: blob: ${assetSources}`,
		`font-src data: ${assetSources}`,
		`connect-src ${connect}`,
		`worker-src ${capabilities.workers ? `blob: ${assetSources}` : "'none'"}`,
		`media-src ${capabilities.media ? `blob: ${assetSources}` : "'none'"}`,
	].join("; ");
}

export function injectCspMeta(html: string, csp: string): string {
	const meta = `<meta http-equiv="Content-Security-Policy" content="${escapeHtmlAttr(csp)}" />`;
	return insertAtHeadStart(html, meta);
}
