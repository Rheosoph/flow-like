import type { AssetSourceResolver } from "../../../hooks/use-asset-source";
import { signedUrlExpiry } from "../../../lib/stable-asset-url";
import type { IStorageState } from "../../../state/backend-state/storage-state";
import { isAllowedChatAssetSource } from "../../a2ui/authoring";
import type { EditorAssetResolver } from "../asset-resolver-context";
import { resolveEditorAssetUrl } from "../hooks/use-editor-asset-url";
import { isStorageUrl } from "../upload-context";

/** A failed or unexpiring link is asked for again after this long at the earliest. */
const RETRY_MS = 30_000;
const MAX_REUSE_MS = 25 * 60_000;
const EXPIRY_MARGIN_MS = 60_000;

export interface ChatAssetScope {
	/** The chat's own file resolver (a bot's `bot/…` files). */
	readonly host?: EditorAssetResolver;
	readonly storage: Pick<
		IStorageState,
		"downloadStorageItems" | "downloadStorageItemsUser"
	>;
	readonly appId?: string;
	readonly scope: "app" | "user";
}

function reuseUntil(signed: string, now: number) {
	const expiry = signedUrlExpiry(signed);
	const lifetime = expiry
		? expiry - now - EXPIRY_MARGIN_MS
		: Number.POSITIVE_INFINITY;
	return now + Math.max(RETRY_MS, Math.min(MAX_REUSE_MS, lifetime));
}

/**
 * Media of a chat reply's interface resolves the way the reply's Markdown images do: https and
 * inline images as written, the chat's own files through the host, and `storage://` links signed
 * for the reader, who can open only what they may read. Anything else is refused.
 *
 * A signed link is reused until shortly before it lapses and a failure is not retried for 30 s,
 * so an image that will not load cannot keep re-signing itself through its error handler.
 */
export function createChatAssetResolver({
	host,
	storage,
	appId,
	scope,
}: ChatAssetScope): AssetSourceResolver {
	const signed = new Map<string, { url: Promise<string>; until: number }>();
	const sign = (source: string) =>
		host?.accepts(source)
			? host.resolve(source)
			: resolveEditorAssetUrl(source, storage, appId, scope);

	return (raw) => {
		const source = raw.trim();
		if (isAllowedChatAssetSource(source)) return Promise.resolve(source);
		if (!host?.accepts(source) && !isStorageUrl(source)) {
			return Promise.reject(
				new Error(
					`A chat reply's interface cannot load "${raw.trim().slice(0, 80)}": use an https URL, an inline image or a stored file's storage:// link.`,
				),
			);
		}
		const now = Date.now();
		const cached = signed.get(source);
		if (cached && cached.until > now) return cached.url;
		const entry = { url: sign(source), until: now + RETRY_MS };
		signed.set(source, entry);
		entry.url.then(
			(url) => {
				entry.until = reuseUntil(url, Date.now());
			},
			() => undefined,
		);
		return entry.url;
	};
}
