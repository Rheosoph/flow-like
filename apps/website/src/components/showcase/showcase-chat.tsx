import type { IMessage } from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import {
	ChatBox,
	type ChatBoxRef,
	type ISendMessageFunction,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chatbox";
import { MessageComponent } from "@flow-like/flow-like-ui/components/interfaces/chat-default/message";
import { IRole } from "@flow-like/flow-like-ui/lib/schema/llm/history";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type ShowcaseDriver,
	type TimelineStep,
	useAutoplay,
} from "./use-autoplay";

let messageCounter = 0;
function mkMsg(role: IRole, content: string): IMessage {
	messageCounter += 1;
	return {
		id: `showcase-${messageCounter}`,
		appId: "showcase",
		sessionId: "demo",
		inner: { role, content },
		files: [],
		timestamp: Date.now(),
	};
}

export interface ShowcaseChatProps {
	timeline?: TimelineStep[];
	/** Seed messages shown before autoplay starts (e.g. an assistant greeting). */
	intro?: { role: "user" | "assistant"; content: string }[];
	/** Honest reply streamed when a visitor sends their own message. */
	replyText?: string;
	tools?: string[];
	className?: string;
}

const DEFAULT_REPLY =
	"You're chatting with the real Flow-Like chat UI — this embedded preview has no model attached, so I can't run your workflow here. In the live app I'd execute your flow and answer with real data. Explore the other live demos on this page to see the actual canvas, runs and catalog.";

export default function ShowcaseChat({
	timeline,
	intro,
	replyText = DEFAULT_REPLY,
	tools = ["Reason", "Search"],
	className,
}: Readonly<ShowcaseChatProps>) {
	const rootRef = useRef<HTMLDivElement | null>(null);
	const scrollRef = useRef<HTMLDivElement | null>(null);
	const chatBoxRef = useRef<ChatBoxRef | null>(null);

	const buildIntro = useCallback(
		() =>
			(intro ?? []).map((m) =>
				mkMsg(m.role === "user" ? IRole.User : IRole.Assistant, m.content),
			),
		[intro],
	);

	const [messages, setMessages] = useState<IMessage[]>(buildIntro);
	const [streaming, setStreaming] = useState<IMessage | null>(null);

	useEffect(() => {
		const el = scrollRef.current;
		if (!el || (messages.length === 0 && !streaming)) return;
		el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
	}, [messages, streaming]);

	const streamAssistant = useCallback(async (text: string) => {
		setStreaming(mkMsg(IRole.Assistant, ""));
		const tokens = text.match(/\S+\s*/g) ?? [text];
		let shown = "";
		for (const tok of tokens) {
			shown += tok;
			const current = shown;
			setStreaming((s) =>
				s ? { ...s, inner: { ...s.inner, content: current } } : s,
			);
			await new Promise((r) => setTimeout(r, 42));
		}
		setStreaming(null);
		setMessages((m) => [...m, mkMsg(IRole.Assistant, text)]);
	}, []);

	const handleSend: ISendMessageFunction = useCallback(
		async (content) => {
			const text = content.trim();
			if (!text) return;
			setMessages((m) => [...m, mkMsg(IRole.User, text)]);
			await streamAssistant(replyText);
		},
		[replyText, streamAssistant],
	);

	const driver = useMemo<ShowcaseDriver>(
		() => ({
			chat: {
				setInput: (t) => chatBoxRef.current?.setInput(t),
				focus: () => chatBoxRef.current?.focusInput?.(),
				send: () => {
					const text = chatBoxRef.current?.getInput().trim() ?? "";
					chatBoxRef.current?.clearInput?.();
					if (text) setMessages((m) => [...m, mkMsg(IRole.User, text)]);
				},
				reset: () => {
					setStreaming(null);
					setMessages(buildIntro());
					chatBoxRef.current?.clearInput?.();
				},
				beginStream: () => setStreaming(mkMsg(IRole.Assistant, "")),
				pushStreamChunk: (full) =>
					setStreaming((s) =>
						s
							? { ...s, inner: { ...s.inner, content: full } }
							: mkMsg(IRole.Assistant, full),
					),
				endStream: (full) => {
					setStreaming(null);
					setMessages((m) => [...m, mkMsg(IRole.Assistant, full)]);
				},
			},
		}),
		[buildIntro],
	);

	useAutoplay(rootRef, timeline, driver);

	return (
		<div
			ref={rootRef}
			className={cn(
				"flex h-full w-full flex-col overflow-hidden bg-background text-foreground",
				className,
			)}
		>
			<div
				ref={scrollRef}
				className="flex-1 space-y-1 overflow-y-auto px-3 py-4 sm:px-5"
			>
				{messages.map((m) => (
					<MessageComponent key={m.id} message={m} />
				))}
				{streaming && (
					<MessageComponent key="streaming" message={streaming} loading />
				)}
			</div>
			<div className="border-t border-border/60 bg-background/80 p-2 backdrop-blur sm:p-3">
				<ChatBox
					ref={chatBoxRef}
					onSendMessage={handleSend}
					fileUpload
					audioInput={false}
					availableTools={tools}
					defaultActiveTools={[tools[0] ?? "Reason"]}
				/>
			</div>
		</div>
	);
}
