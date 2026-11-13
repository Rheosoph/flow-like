"use client";

import { useEffect, useMemo, useState } from "react";
import {
	type ResolvedAssetUrl,
	resolveAssetUrl,
} from "../../../../lib/asset-url-cache";
import { conversationOfApp } from "../../../../lib/conversation-files";
import { parseWorkspaceFileUrl } from "../../../../lib/workspace-file-url";
import { useBackend, useBackendReady } from "../../../../state/backend-state";
import type { IInboxState } from "../../../../state/backend-state/inbox-state";
import { mimeTypeForName } from "../../../storage/storage-attach-limits";
import type { ProcessedAttachment } from "../attachment";

const MIN_REFRESH_DELAY_MS = 1000;
const RETRY_DELAY_MS = 30_000;
/** Signs a conversation file again this long before its link lapses. */
const LINK_REFRESH_MARGIN_MS = 60_000;
/** How long a signed conversation-file link is trusted when the hub names no expiry. */
const UNDATED_LINK_MS = 4 * 60_000;
const FAILED: ResolvedAssetUrl = {
	url: "",
	expiresAt: Number.POSITIVE_INFINITY,
	usableUntil: Number.POSITIVE_INFINITY,
	resolved: false,
};

type Resolution = Map<string, ResolvedAssetUrl>;

/** A link to a file of a conversation, from the inbox that holds it. */
async function conversationFileLink(
	inboxState: IInboxState | undefined,
	conversationId: string,
	path: string,
): Promise<ResolvedAssetUrl> {
	if (!inboxState) return FAILED;
	const download = await inboxState.conversationFileDownload(conversationId, {
		path,
		mime: mimeTypeForName(path),
	});
	const now = Date.now();
	const dated = download.expires_at
		? Date.parse(download.expires_at)
		: Number.NaN;
	const expiresAt = Number.isFinite(dated)
		? dated
		: download.url.startsWith("blob:")
			? Number.POSITIVE_INFINITY
			: now + UNDATED_LINK_MS;
	return {
		url: download.url,
		expiresAt,
		usableUntil: Number.isFinite(expiresAt)
			? Math.max(expiresAt - LINK_REFRESH_MARGIN_MS, now + RETRY_DELAY_MS)
			: expiresAt,
		resolved: true,
	};
}

/**
 * Signs the `storage://` files of a strip and keeps the links alive. `url` carries the signed link
 * so every consumer works unchanged; `sourceUrl` keeps the stored address as the file's identity.
 * A conversation file (`storage://apps/conversation_<id>/app/<file>`) is signed by the inbox.
 */
export function useResolvedAttachments(
	files: readonly ProcessedAttachment[],
): ProcessedAttachment[] {
	const { storageState, inboxState } = useBackend();
	const ready = useBackendReady();
	const [resolved, setResolved] = useState<Resolution>(new Map());
	const [attempt, setAttempt] = useState(0);

	const sources = useMemo(
		() =>
			[
				...new Set(
					files
						.map((file) => file.url)
						.filter((url) => parseWorkspaceFileUrl(url)),
				),
			].sort(),
		[files],
	);
	const sourcesKey = sources.join("\n");

	// biome-ignore lint/correctness/useExhaustiveDependencies: `sourcesKey` stands for `sources`; `attempt` re-signs before a link lapses.
	useEffect(() => {
		if (!ready || sources.length === 0) return;
		let cancelled = false;
		let timer: ReturnType<typeof setTimeout> | undefined;
		void Promise.all(
			sources.map(async (source) => {
				const target = parseWorkspaceFileUrl(source);
				if (!target) return [source, FAILED] as const;
				const conversationId = conversationOfApp(target.appId);
				try {
					return [
						source,
						conversationId
							? await conversationFileLink(
									inboxState,
									conversationId,
									target.path,
								)
							: await resolveAssetUrl(target.appId, target.path, storageState),
					] as const;
				} catch {
					return [source, FAILED] as const;
				}
			}),
		).then((entries) => {
			if (cancelled) return;
			const next: Resolution = new Map();
			let usableUntil = Number.POSITIVE_INFINITY;
			for (const [source, entry] of entries) {
				next.set(source, entry);
				usableUntil = Math.min(
					usableUntil,
					entry.resolved ? entry.usableUntil : Date.now() + RETRY_DELAY_MS,
				);
			}
			setResolved(next);
			const delay = usableUntil - Date.now();
			if (Number.isFinite(delay))
				timer = setTimeout(
					() => setAttempt((value) => value + 1),
					Math.max(delay, MIN_REFRESH_DELAY_MS),
				);
		});
		return () => {
			cancelled = true;
			if (timer !== undefined) clearTimeout(timer);
		};
	}, [ready, sourcesKey, storageState, inboxState, attempt]);

	return useMemo(
		() =>
			files.map((file) => {
				if (!parseWorkspaceFileUrl(file.url)) return file;
				const entry = resolved.get(file.url);
				if (entry?.resolved)
					return { ...file, url: entry.url, sourceUrl: file.url };
				return {
					...file,
					type: "other" as const,
					sourceUrl: file.url,
					access: entry ? ("unavailable" as const) : ("pending" as const),
				};
			}),
		[files, resolved],
	);
}
