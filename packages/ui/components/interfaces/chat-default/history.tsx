"use client";

import { createId } from "@paralleldrive/cuid2";
import { useLiveQuery } from "dexie-react-hooks";
import {
	CalendarIcon,
	MessageCircleIcon,
	SearchIcon,
	SquarePenIcon,
	TrashIcon,
} from "lucide-react";
import { type RefObject, useCallback, useState } from "react";
import { Badge, Button, Input } from "../../ui";
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
	const [searchQuery, setSearchQuery] = useState("");

	const sessions = useLiveQuery(
		() =>
			chatDb.sessions
				.where("appId")
				.equals(appId)
				.reverse()
				.sortBy("updatedAt"),
		[appId],
	);

	const filteredSessions = sessions?.filter((session) =>
		session.summarization?.toLowerCase().includes(searchQuery.toLowerCase()),
	);

	const handleNewChat = useCallback(() => {
		const newSessionId = createId();
		onSessionChange(newSessionId);
		if (sidebarRef?.current?.isMobile()) {
			sidebarRef?.current?.toggleOpen();
		}
	}, [onSessionChange]);

	const handleSessionSelect = useCallback(
		(selectedSessionId: string) => {
			onSessionChange(selectedSessionId);
			if (sidebarRef?.current?.isMobile()) {
				sidebarRef?.current?.toggleOpen();
			}
		},
		[onSessionChange],
	);

	const handleDeleteSession = useCallback(
		async (sessionIdToDelete: string, e: React.MouseEvent) => {
			e.stopPropagation();
			await chatDb.messages
				.where("sessionId")
				.equals(sessionIdToDelete)
				.delete();
			await chatDb.sessions.delete(sessionIdToDelete);
			await chatDb.localStage
				.where("sessionId")
				.equals(sessionIdToDelete)
				.delete();

			if (sessionIdToDelete === sessionId) {
				handleNewChat();
			}
		},
		[sessionId, handleNewChat],
	);

	const formatRelativeTime = (timestamp: number) => {
		const now = Date.now();
		const diff = now - timestamp;
		const minutes = Math.floor(diff / (1000 * 60));
		const hours = Math.floor(diff / (1000 * 60 * 60));
		const days = Math.floor(diff / (1000 * 60 * 60 * 24));

		if (minutes < 1) return "Just now";
		if (minutes < 60) return `${minutes}m ago`;
		if (hours < 24) return `${hours}h ago`;
		if (days < 7) return `${days}d ago`;
		return new Date(timestamp).toLocaleDateString();
	};

	const groupSessionsByDate = (sessions: typeof filteredSessions) => {
		if (!sessions) return {};

		const groups: Record<string, typeof sessions> = {};
		const now = new Date();
		const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
		const yesterday = new Date(today.getTime() - 24 * 60 * 60 * 1000);
		const weekAgo = new Date(today.getTime() - 7 * 24 * 60 * 60 * 1000);

		sessions.forEach((session) => {
			const sessionDate = new Date(session.updatedAt);
			let groupKey: string;

			if (sessionDate >= today) {
				groupKey = "Today";
			} else if (sessionDate >= yesterday) {
				groupKey = "Yesterday";
			} else if (sessionDate >= weekAgo) {
				groupKey = "This Week";
			} else {
				groupKey = "Older";
			}

			if (!groups[groupKey]) groups[groupKey] = [];
			groups[groupKey].push(session);
		});

		return groups;
	};

	const sessionGroups = groupSessionsByDate(filteredSessions);

	return (
		<div className="flex flex-col h-full flex-grow overflow-hidden max-h-full">
			<div className="relative border-b border-border/20 bg-background/95 backdrop-blur-md">
				<div className="p-4">
					<div className="flex items-center justify-between mb-4">
						<div className="flex items-center gap-3">
							<div className="p-2 rounded-lg bg-primary/10 border border-primary/15">
								<MessageCircleIcon className="w-4 h-4 text-primary" />
							</div>
							<div>
								<h2 className="text-lg font-semibold text-foreground">
									Chat History
								</h2>
								<p className="text-xs text-muted-foreground">
									{sessions?.length ?? 0} conversation
									{sessions?.length !== 1 ? "s" : ""}
								</p>
							</div>
						</div>
						<Button
							variant="outline"
							size="sm"
							onClick={handleNewChat}
							className="h-8 w-8 p-0 border-border/30 hover:border-primary/30 hover:bg-primary/5"
						>
							<SquarePenIcon className="w-3.5 h-3.5" />
						</Button>
					</div>

					<div className="relative">
						<SearchIcon className="absolute left-3 top-1/2 transform -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/60" />
						<Input
							placeholder="Search conversations..."
							value={searchQuery}
							onChange={(e) => setSearchQuery(e.target.value)}
							className="pl-9 pr-3 py-2 h-8 text-sm bg-muted/30 border-border/30 focus:border-primary/40 focus:ring-2 focus:ring-primary/10 rounded-md"
						/>
					</div>
				</div>
			</div>

			<div className="flex flex-col flex-1 bg-gradient-to-b from-background/50 to-background max-h-full overflow-auto">
				<div className="p-6 space-y-8 w-full">
					{Object.entries(sessionGroups).map(([groupName, groupSessions]) => (
						<div key={groupName} className="space-y-4 w-full">
							<div className="flex items-center gap-3 px-1 w-full">
								<div className="p-1.5 rounded-md bg-muted/60 border border-border/40">
									<CalendarIcon className="w-3.5 h-3.5 text-muted-foreground" />
								</div>
								<h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">
									{groupName}
								</h3>
								<div className="flex-1 h-px bg-gradient-to-r from-border/50 to-transparent" />
								<Badge
									variant="secondary"
									className="text-xs px-2.5 py-1 bg-muted/60 text-muted-foreground border border-border/40"
								>
									{groupSessions.length}
								</Badge>
							</div>

							<div className="space-y-2">
								{groupSessions.map((session) => (
									<button
										key={session.id}
										className={`
                                        group w-full relative rounded-lg overflow-hidden transition-all duration-200 cursor-pointer
                                        ${
																					session.id === sessionId
																						? "bg-primary/8 border border-primary/20"
																						: "hover:bg-muted/40 border border-transparent"
																				}
                                    `}
										onClick={() => handleSessionSelect(session.id)}
									>
										<div className="px-4 py-3">
											<div className="flex items-center justify-start gap-4">
												<div
													className={`
                                                w-2 h-2 rounded-full transition-colors duration-200
                                                ${
																									session.id === sessionId
																										? "bg-primary"
																										: "bg-muted/80 group-hover:bg-primary"
																								}
                                            `}
												/>

												<div className="flex-1 min-w-0 w-full flex flex-col items-start">
													<p className="text-sm font-medium text-foreground line-clamp-1 mb-1 text-start">
														{session.summarization || "New conversation"}
													</p>
													<p className="text-xs text-muted-foreground">
														{formatRelativeTime(session.updatedAt)}
													</p>
												</div>

												<Button
													variant="destructive"
													size="icon"
													className="h-7 w-7 opacity-0 group-hover:opacity-100 transition-opacity"
													onClick={(e) => handleDeleteSession(session.id, e)}
												>
													<TrashIcon className="w-3.5 h-3.5" />
												</Button>
											</div>
										</div>
									</button>
								))}
							</div>
						</div>
					))}

					{filteredSessions?.length === 0 && (
						<div className="flex flex-col items-center justify-center py-20 text-center w-full">
							<div className="relative mb-6">
								<div className="absolute inset-0 bg-muted/30 rounded-full blur-xl" />
								<div className="relative p-6 rounded-full bg-gradient-to-br from-muted/60 to-muted/40 border border-border/40 shadow-lg">
									<MessageCircleIcon className="w-10 h-10 text-muted-foreground/60" />
								</div>
							</div>
							<h3 className="text-lg font-bold text-foreground mb-3">
								{searchQuery
									? "No conversations found"
									: "No conversations yet"}
							</h3>
							<p className="text-sm text-muted-foreground/80 max-w-sm leading-relaxed mb-6">
								{searchQuery
									? "Try adjusting your search terms to find what you're looking for"
									: "Start a new conversation and it will appear here for easy access"}
							</p>
							{!searchQuery && (
								<Button
									variant="default"
									size="sm"
									onClick={handleNewChat}
									className="gap-2 bg-primary hover:bg-primary/90 shadow-lg hover:shadow-xl transition-all duration-300 hover:scale-105"
								>
									<SquarePenIcon className="w-4 h-4" />
									Start Chatting
								</Button>
							)}
						</div>
					)}
				</div>
			</div>
		</div>
	);
}
