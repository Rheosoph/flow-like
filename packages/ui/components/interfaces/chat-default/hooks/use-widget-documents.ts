"use client";

import { useQueries } from "@tanstack/react-query";
import { appWidgetDocumentOf } from "../../../../lib/inbox-ui-part";
import { useBackend, useBackendReady } from "../../../../state/backend-state";
import type {
	IInboxFileHandle,
	IInboxState,
} from "../../../../state/backend-state/inbox-state";
import type { IStorageState } from "../../../../state/backend-state/storage-state";
import type { IChatWidget } from "../chat-db";
import { storedFileLink } from "./use-resolved-attachments";

const reported = new Set<string>();

const documentName = (handle: IInboxFileHandle) =>
	`${handle.app_id}/${handle.path}`;

function reportOnce(handle: IInboxFileHandle, error: unknown) {
	const name = documentName(handle);
	if (reported.has(name)) return;
	reported.add(name);
	console.warn(
		`[chat] The stored app widget ${name} could not be shown`,
		error,
	);
}

/** Null for a document that is no app widget, so it is not fetched again; a failed load throws. */
async function loadWidgetDocument(
	handle: IInboxFileHandle,
	storageState: IStorageState,
	inboxState: IInboxState | undefined,
): Promise<IChatWidget | null> {
	const name = documentName(handle);
	try {
		const link = await storedFileLink(
			{ appId: handle.app_id, path: handle.path },
			storageState,
			inboxState,
		);
		if (!link.resolved)
			throw new Error(`Could not sign a link to the widget document ${name}`);
		const response = await fetch(link.url);
		if (!response.ok)
			throw new Error(
				`Loading the widget document ${name} failed (${response.status})`,
			);
		const widget = appWidgetDocumentOf(
			await response.json().catch(() => undefined),
		);
		if (!widget)
			reportOnce(handle, new Error(`${name} is not an app_widget document`));
		return widget;
	} catch (error) {
		reportOnce(handle, error);
		throw error;
	}
}

const loadedWidgets = (results: readonly { data?: IChatWidget | null }[]) =>
	results.flatMap((result) => (result.data ? [result.data] : []));

/**
 * The app widgets stored as documents (a bot workspace file or a conversation file), in order,
 * each as soon as it loads. A document is read once per path and content hash; one that fails to
 * load or is no app widget is left out.
 */
export function useWidgetDocuments(
	documents: readonly IInboxFileHandle[],
): IChatWidget[] {
	const { storageState, inboxState } = useBackend();
	const ready = useBackendReady();
	return useQueries({
		queries: documents.map((handle) => ({
			queryKey: [
				"chat",
				"app-widget-document",
				handle.app_id,
				handle.path,
				handle.sha256 ?? "",
			],
			queryFn: () => loadWidgetDocument(handle, storageState, inboxState),
			enabled: ready,
			staleTime: Number.POSITIVE_INFINITY,
			retry: 1,
			meta: { persist: false },
		})),
		combine: loadedWidgets,
	});
}
