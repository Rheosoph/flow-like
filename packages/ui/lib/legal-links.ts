export const FLOW_LIKE_LEGAL_URLS = {
	notice: "https://flow-like.com/legal-notice",
	privacy: "https://flow-like.com/privacy-policy",
	eula: "https://flow-like.com/eula",
} as const;

export type LegalLinkKind = keyof typeof FLOW_LIKE_LEGAL_URLS;

/** The app publisher's links shown on its hosted pages. */
export interface IAppLegalLinks {
	legal_notice_url?: string | null;
	privacy_policy_url?: string | null;
}

/** Publisher links are owner-authored; only absolute http(s) URLs become hrefs. */
export function safeExternalHref(value: string | null | undefined) {
	if (!value) return undefined;
	try {
		const url = new URL(value);
		return url.protocol === "https:" || url.protocol === "http:"
			? url.href
			: undefined;
	} catch {
		return undefined;
	}
}
