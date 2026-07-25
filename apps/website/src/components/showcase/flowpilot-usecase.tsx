"use client";

import type {
	IMessage,
	IPlanStep,
	PlanStepStatus,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import {
	ChatBox,
	type ChatBoxRef,
	type ISendMessageFunction,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chatbox";
import { MessageComponent } from "@flow-like/flow-like-ui/components/interfaces/chat-default/message";
import { FlowPilotBubbleOrb } from "@flow-like/flow-like-ui/components/global-chat/flowpilot-bubble-orb";
import { Badge } from "@flow-like/flow-like-ui/components/ui/badge";
import { IRole } from "@flow-like/flow-like-ui/lib/schema/llm/history";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

let counter = 0;
function mkMsg(
	role: IRole,
	content: string,
	extra: Partial<IMessage> = {},
): IMessage {
	counter += 1;
	return {
		id: `fp-usecase-${counter}`,
		appId: "showcase",
		sessionId: "demo",
		inner: { role, content },
		files: [],
		timestamp: Date.now(),
		...extra,
	};
}

// The governed reasoning trace FlowPilot runs for the request. Statuses animate
// planned -> progress -> done so the real PlanSteps widget renders live, then the
// list settles and collapses to a one-line summary.
const PLAN: Omit<IPlanStep, "status">[] = [
	{
		id: "s1",
		title: "Read the delay notice",
		description: "supplier-delay-notice.pdf",
		reasoning:
			"Parsed the attached notice: the shipment slips 5 days, and PO 4482 is the only order feeding next week's production line.",
	},
	{
		id: "s2",
		title: "Query Supply Control",
		description: "company ontology",
		reasoning:
			"Resolved the supplier across the ontology — three open POs depend on it, one of them time-critical.",
	},
	{
		id: "s3",
		title: "Model reroute options",
		description: "Porto vs. Warsaw",
		reasoning:
			"Compared alternative suppliers on lead time, landed cost, and risk. Porto keeps the run on schedule; Warsaw slips two days.",
	},
	{
		id: "s4",
		title: "Score production impact",
		description: "run protected",
		reasoning:
			"The Porto reroute protects next week's production for €1,240 in extra freight — inside the approved variance.",
	},
	{
		id: "s5",
		title: "Draft approvals",
		description: "Finance + Ops",
		reasoning:
			"Prepared the governed approval with Finance and Ops as approvers. Nothing is sent until you confirm.",
	},
];

const ANSWER =
	"**Recommendation:** reroute PO 4482 through the Porto supplier. It protects next week's production for **+€1,240** and needs a single Finance + Ops approval. Nothing is applied yet — want me to send the approvals, or open the reroute in the board first?";

const LIVE_REPLY =
	"You're using the real Flow-Like chat UI — this embedded preview has no model attached, so I can't run your workflow here. In the app I'd execute the flow, stream the plan you see above, and show every proposed change for review before anything is applied.";

const USER_FILE = {
	url: "/demo/supplier-delay-notice.pdf",
	name: "supplier-delay-notice.pdf",
	type: "application/pdf",
	size: 41216,
};

const withStatus = (done: number, active: number): IPlanStep[] =>
	PLAN.map((s, i) => ({
		...s,
		status: (i < done
			? "done"
			: i === active
				? "progress"
				: "planned") as PlanStepStatus,
	}));

export default function FlowPilotUseCase({
	className,
}: Readonly<{ className?: string }>) {
	const rootRef = useRef<HTMLDivElement | null>(null);
	const scrollRef = useRef<HTMLDivElement | null>(null);
	const chatBoxRef = useRef<ChatBoxRef | null>(null);
	const startedRef = useRef(false);
	const cancelledRef = useRef(false);

	const userMsg = useMemo(
		() =>
			mkMsg(
				IRole.User,
				"Resolve the supplier delay without impacting next week's production — details attached.",
				{ files: [USER_FILE] },
			),
		[],
	);

	const [messages, setMessages] = useState<IMessage[]>([userMsg]);
	const [streaming, setStreaming] = useState<IMessage | null>(null);

	useEffect(() => {
		const el = scrollRef.current;
		if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
	}, [messages, streaming]);

	useEffect(() => {
		const root = rootRef.current;
		if (!root) return;
		cancelledRef.current = false;
		const sleep = (ms: number) =>
			new Promise<void>((r) => setTimeout(r, ms));
		const reduced = window.matchMedia(
			"(prefers-reduced-motion: reduce)",
		).matches;

		const finalMsg = () =>
			mkMsg(IRole.Assistant, ANSWER, {
				plan_steps: withStatus(PLAN.length, -1),
			});

		const run = async () => {
			if (startedRef.current) return;
			startedRef.current = true;

			if (reduced) {
				setMessages((m) => [...m, finalMsg()]);
				return;
			}

			const base = mkMsg(IRole.Assistant, "");
			setStreaming({
				...base,
				plan_steps: withStatus(0, 0),
				current_step_id: PLAN[0].id,
			});
			await sleep(500);

			for (let i = 0; i < PLAN.length; i++) {
				if (cancelledRef.current) return;
				setStreaming({
					...base,
					plan_steps: withStatus(i, i),
					current_step_id: PLAN[i].id,
				});
				await sleep(560 + i * 70);
				if (cancelledRef.current) return;
				const nextActive = i + 1 < PLAN.length ? i + 1 : -1;
				setStreaming({
					...base,
					plan_steps: withStatus(i + 1, nextActive),
					current_step_id: nextActive >= 0 ? PLAN[nextActive].id : undefined,
				});
				await sleep(180);
			}

			// Stream the recommendation under the completed plan.
			const donePlan = withStatus(PLAN.length, -1);
			const tokens = ANSWER.match(/\S+\s*/g) ?? [ANSWER];
			let shown = "";
			for (const tok of tokens) {
				if (cancelledRef.current) return;
				shown += tok;
				setStreaming({
					...base,
					inner: { ...base.inner, content: shown },
					plan_steps: donePlan,
				});
				await sleep(26);
			}
			if (cancelledRef.current) return;
			setStreaming(null);
			setMessages((m) => [...m, finalMsg()]);
		};

		const io = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) {
					io.disconnect();
					void run();
				}
			},
			{ rootMargin: "0px 0px -20% 0px", threshold: 0.15 },
		);
		io.observe(root);

		return () => {
			cancelledRef.current = true;
			io.disconnect();
		};
	}, []);

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
			await new Promise((r) => setTimeout(r, 26));
		}
		setStreaming(null);
		setMessages((m) => [...m, mkMsg(IRole.Assistant, text)]);
	}, []);

	const handleSend: ISendMessageFunction = useCallback(
		async (content) => {
			const text = content.trim();
			if (!text) return;
			setMessages((m) => [...m, mkMsg(IRole.User, text)]);
			await streamAssistant(LIVE_REPLY);
		},
		[streamAssistant],
	);

	return (
		<div
			ref={rootRef}
			className={cn(
				"flex w-full min-w-0 flex-col overflow-hidden rounded-xl border border-primary/20 bg-background/95 shadow-2xl shadow-black/40",
				className,
			)}
		>
			<div className="flex items-center gap-2 border-b border-border/60 bg-muted/40 px-3 py-2">
				<FlowPilotBubbleOrb className="-my-1 size-9" title="FlowPilot" />
				<strong className="text-xs">FlowPilot</strong>
				<Badge variant="outline" className="ml-auto text-[9px]">
					Supplier Ops · governed
				</Badge>
			</div>

			<div
				ref={scrollRef}
				className="flex-1 min-w-0 space-y-1 overflow-y-auto [scrollbar-gutter:stable] break-words px-3 py-4 sm:px-4"
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
					availableTools={["Reason", "Search", "Files"]}
					defaultActiveTools={["Reason"]}
				/>
			</div>
		</div>
	);
}
