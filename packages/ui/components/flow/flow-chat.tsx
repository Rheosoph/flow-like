"use client";
import { useTranslation } from "@flow-like/locales";
import { SendHorizontal, X } from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import {
	type PeerUserInfo,
	colorFromSub,
	truncateName,
} from "../../hooks/use-peer-users";
import type { ChatMessage } from "../../hooks/use-realtime-chat";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";

function formatTime(ts: number): string {
	const d = new Date(ts);
	return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export const FlowChat = memo(function FlowChat({
	messages,
	onSendMessage,
	onClose,
	peerUsers,
	sub,
}: {
	messages: ChatMessage[];
	onSendMessage: (text: string) => void;
	onClose: () => void;
	peerUsers: Map<string, PeerUserInfo>;
	sub?: string;
}) {
	const { t } = useTranslation("flow");
	const [input, setInput] = useState("");
	const messagesEndRef = useRef<HTMLDivElement>(null);
	const inputRef = useRef<HTMLInputElement>(null);

	// biome-ignore lint/correctness/useExhaustiveDependencies: scroll on new messages
	useEffect(() => {
		messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
	}, [messages.length]);

	useEffect(() => {
		inputRef.current?.focus();
	}, []);

	const handleSend = useCallback(() => {
		if (!input.trim()) return;
		onSendMessage(input);
		setInput("");
	}, [input, onSendMessage]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				handleSend();
			}
		},
		[handleSend],
	);

	return (
		<div className="flex flex-col w-80 h-96 rounded-xl border border-border/60 bg-background/95 backdrop-blur-lg shadow-2xl overflow-hidden">
			{/* Header */}
			<div className="flex items-center justify-between px-3 py-2 border-b border-border/40">
				<span className="text-sm font-medium">
					{t("boardChat", "Board Chat")}
				</span>
				<button
					type="button"
					onClick={onClose}
					className="h-6 w-6 flex items-center justify-center rounded-md hover:bg-muted/50 transition-colors"
				>
					<X className="h-3.5 w-3.5" />
				</button>
			</div>

			{/* Messages */}
			<div className="flex-1 overflow-y-auto px-3 py-2 space-y-2 min-h-0">
				{messages.length === 0 && (
					<div className="flex items-center justify-center h-full text-xs text-muted-foreground">
						{t("noMessagesYetSayHi", "No messages yet. Say hi!")}
					</div>
				)}
				{messages.map((msg, i) => {
					const isSelf = msg.sub === sub;
					const userInfo = peerUsers.get(msg.sub);
					const color = userInfo?.color ?? colorFromSub(msg.sub);
					const name =
						userInfo?.truncatedName ?? truncateName(msg.sub?.slice(-8));

					// Group consecutive messages from same sender
					const prevMsg = i > 0 ? messages[i - 1] : undefined;
					const showHeader =
						!prevMsg ||
						prevMsg.sub !== msg.sub ||
						msg.timestamp - prevMsg.timestamp > 60_000;

					return (
						<div
							key={msg.id}
							className={`flex flex-col ${isSelf ? "items-end" : "items-start"}`}
						>
							{showHeader && (
								<div
									className={`flex items-center gap-1.5 mb-0.5 ${isSelf ? "flex-row-reverse" : ""}`}
								>
									<Avatar className="h-4 w-4">
										{userInfo?.avatarUrl && (
											<AvatarImage
												src={userInfo.avatarUrl}
												className="object-cover"
											/>
										)}
										<AvatarFallback
											className="text-[8px] font-bold"
											style={{
												background: color,
												color: "white",
											}}
										>
											{name.charAt(0).toUpperCase()}
										</AvatarFallback>
									</Avatar>
									<span className="text-[10px] font-medium" style={{ color }}>
										{isSelf ? "You" : name}
									</span>
									<span className="text-[10px] text-muted-foreground">
										{formatTime(msg.timestamp)}
									</span>
								</div>
							)}
							<div
								className={`rounded-lg px-2.5 py-1.5 max-w-[85%] text-sm break-words ${
									isSelf ? "bg-primary text-primary-foreground" : "bg-muted"
								}`}
							>
								{msg.text}
							</div>
						</div>
					);
				})}
				<div ref={messagesEndRef} />
			</div>

			{/* Input */}
			<div className="border-t border-border/40 px-3 py-2">
				<div className="flex items-center gap-2">
					<input
						ref={inputRef}
						type="text"
						value={input}
						onChange={(e) => setInput(e.target.value)}
						onKeyDown={handleKeyDown}
						placeholder={t("typeAMessage", "Type a message...")}
						className="flex-1 bg-muted/50 rounded-lg px-3 py-1.5 text-sm outline-none placeholder:text-muted-foreground/60 focus:ring-1 focus:ring-primary/30"
						maxLength={500}
					/>
					<button
						type="button"
						onClick={handleSend}
						disabled={!input.trim()}
						className="h-7 w-7 flex items-center justify-center rounded-lg bg-primary text-primary-foreground disabled:opacity-40 hover:bg-primary/90 transition-colors"
					>
						<SendHorizontal className="h-3.5 w-3.5" />
					</button>
				</div>
			</div>
		</div>
	);
});
