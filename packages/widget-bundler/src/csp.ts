import { escapeHtmlAttr, insertAtHeadStart } from "./html";

/**
 * Build the widget-document CSP from the design (§4.2): `default-src 'none'`
 * baseline, inline script/style allowed (self-contained documents), chunk
 * loads restricted to the serving prefix, network restricted to the hosts
 * granted by the package manifest.
 */
export function buildCsp(
	servingPrefix: string | null,
	connectHosts: string[],
): string {
	// The bundle hash is a hash of the finished archive, so pack cannot embed a
	// hash-specific serving URL without making the archive self-referential.
	// Allow only the origins used by the supported web and Tauri asset servers;
	// callers may additionally narrow/extend this with a deployment prefix.
	const assetSources = [
		"'self'",
		"flow-widget:",
		"http://flow-widget.localhost",
		...(servingPrefix ? [servingPrefix] : []),
	].join(" ");
	const connect = connectHosts.length > 0 ? connectHosts.join(" ") : "'none'";
	return [
		"default-src 'none'",
		`script-src 'unsafe-inline' ${assetSources}`,
		`style-src 'unsafe-inline' ${assetSources}`,
		`img-src data: blob: ${assetSources}`,
		`font-src data: ${assetSources}`,
		`connect-src ${connect}`,
	].join("; ");
}

export function injectCspMeta(html: string, csp: string): string {
	const meta = `<meta http-equiv="Content-Security-Policy" content="${escapeHtmlAttr(csp)}" />`;
	return insertAtHeadStart(html, meta);
}
