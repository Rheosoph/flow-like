"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { useLiveQuery } from "dexie-react-hooks";
import { type RefObject, useCallback, useMemo, useState } from "react";
import { useIsMobile } from "../../../hooks/use-mobile";
import { ChatHistoryList } from "../../chat-history/chat-history-list";
import type { IHistoryEntry } from "../../chat-history/chat-history-types";
import { buildSearchCorpus } from "../../chat-history/use-history-search";
import type { ISidebarActions } from "../interfaces";
import { chatDb } from "./chat-db";

interface IChatHistory {
	appId: string;
	sessionId: string;
	onSessionChange: (sessionId: string) => void;
	sidebarRef?: RefObject<ISidebarActions | null>;
}

export function ChatHistory({
	appId,
	sessionId,
	onSessionChange,
	sidebarRef,
}: Readonly<IChatHistory>) {
	const { t } = useTranslation("chat");
	const isMobile = useIsMobile();

	const sessions = useLiveQuery(
		() =>
			chatDb.sessions
				.where("appId")
				.equals(appId)
				.reverse()
				.sortBy("updatedAt"),
		[appId],
	);

	const sessionIds = useMemo(
		() => (sessions ?? []).map((session) => session.id),
		[sessions],
	);
	const sessionIdKey = sessionIds.join(",");

	// One indexed range scan across every listed conversation rather than a query per row, and only
	// once the user is actually searching — this sidebar is mounted for the whole session, so an
	// unconditional read would pull the app's entire message history on every mount.
	const [searching, setSearching] = useState(false);
	const messages = useLiveQuery(
		() =>
			searching && sessionIds.length > 0
				? chatDb.messages.where("sessionId").anyOf(sessionIds).toArray()
				: [],
		[searching, sessionIdKey],
	);

	const entries = useMemo<IHistoryEntry[] | undefined>(() => {
		if (!sessions) return undefined;
		const corpus = buildSearchCorpus(messages);
		return sessions.map((session) => ({
			id: session.id,
			title: session.summarization || "New conversation",
			updatedAt: session.updatedAt,
			pinnedAt: session.pinnedAt,
			searchBody: corpus.get(session.id) ?? "",
		}));
	}, [sessions, messages]);

	const closeMobileSidebar = useCallback(() => {
		if (sidebarRef?.current?.isMobile()) sidebarRef.current.toggleOpen();
	}, [sidebarRef]);

	const handleNewChat = useCallback(() => {
		onSessionChange(createId());
		closeMobileSidebar();
	}, [onSessionChange, closeMobileSidebar]);

	const handleSelect = useCallback(
		(selectedSessionId: string) => {
			onSessionChange(selectedSessionId);
			closeMobileSidebar();
		},
		[onSessionChange, closeMobileSidebar],
	);

	const handleTogglePin = useCallback(
		// A partial update, never a whole-row put: pinning must not bump `updatedAt` or the
		// conversation jumps into the "Today" group just for being pinned.
		async (id: string, pinned: boolean) => {
			await chatDb.sessions.update(id, {
				pinnedAt: pinned ? Date.now() : undefined,
			});
		},
		[],
	);

	const handleRename = useCallback(async (id: string, title: string) => {
		const next = title.trim().slice(0, 80);
		if (!next) return;
		await chatDb.sessions.update(id, { summarization: next });
	}, []);

	const handleDelete = useCallback(
		async (idToDelete: string) => {
			await chatDb.messages.where("sessionId").equals(idToDelete).delete();
			await chatDb.sessions.delete(idToDelete);
			await chatDb.localStage.where("sessionId").equals(idToDelete).delete();
			if (idToDelete === sessionId) handleNewChat();
		},
		[sessionId, handleNewChat],
	);

	return (
		<ChatHistoryList
			className="h-full max-h-full grow"
			entries={entries}
			activeId={sessionId}
			onSelect={handleSelect}
			onNew={handleNewChat}
			onTogglePin={handleTogglePin}
			onRename={handleRename}
			onDelete={handleDelete}
			density={isMobile ? "comfortable" : "compact"}
			onSearchActiveChange={setSearching}
			header={
				entries
					? t('countConversations', '{{count}} conversation', { count: entries.length })
					: undefined
			}
			emptyDescription="Start a conversation and it will appear here."
		/>
	);
}
