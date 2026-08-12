import { FlowPilotInput } from "@flow-like/flow-like-ui/components/flowpilot/FlowPilotInput";
import { FlowPilotBubbleOrb } from "@flow-like/flow-like-ui/components/global-chat/flowpilot-bubble-orb";
import { Badge } from "@flow-like/flow-like-ui/components/ui/badge";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { BotIcon, UserIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";

interface InlineFlowPilotProps {
	prompt: string;
	reply: string;
	className?: string;
}

interface DemoMessage {
	id: number;
	role: "user" | "assistant";
	content: string;
}

export default function InlineFlowPilot({
	prompt,
	reply,
	className,
}: Readonly<InlineFlowPilotProps>) {
	const nextId = useRef(2);
	const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const [thinking, setThinking] = useState(false);
	const [messages, setMessages] = useState<DemoMessage[]>([
		{ id: 0, role: "user", content: prompt },
		{ id: 1, role: "assistant", content: reply },
	]);
	const resetDemo = () => {
		if (timer.current) clearTimeout(timer.current);
		setThinking(false);
		setMessages([
			{ id: nextId.current++, role: "user", content: prompt },
			{ id: nextId.current++, role: "assistant", content: reply },
		]);
	};

	useEffect(
		() => () => {
			if (timer.current) clearTimeout(timer.current);
		},
		[],
	);

	const answer = (message: string) => {
		setMessages((current) => [
			...current.slice(-2),
			{ id: nextId.current++, role: "user", content: message },
		]);
		setThinking(true);
		if (timer.current) clearTimeout(timer.current);
		const reduced = window.matchMedia(
			"(prefers-reduced-motion: reduce)",
		).matches;
		timer.current = setTimeout(
			() => {
				setThinking(false);
				setMessages((current) => [
					...current,
					{
						id: nextId.current++,
						role: "assistant",
						content:
							"This preview uses the real FlowPilot composer. In Flow-Like, I would now update the board and show every proposed change for review.",
					},
				]);
			},
			reduced ? 0 : 520,
		);
	};

	return (
		<div
			className={cn(
				"flex min-h-48 w-full flex-col overflow-hidden rounded-xl border border-primary/20 bg-background/95 shadow-lg",
				className,
			)}
		>
			<div className="flex items-center gap-2 border-b border-border/60 bg-muted/45 px-3 py-2">
				<FlowPilotBubbleOrb
					className="-my-1 size-9"
					onClick={resetDemo}
					title="Reset the FlowPilot demo"
				/>
				<strong className="text-xs">FlowPilot</strong>
				<Badge variant="outline" className="ml-auto text-[9px]">
					local
				</Badge>
			</div>

			<div className="flex max-h-32 min-h-20 flex-1 flex-col gap-2 overflow-y-auto px-3 py-2 text-[11px] leading-relaxed">
				{messages.slice(-3).map((message) => (
					<div
						key={message.id}
						className={cn(
							"flex max-w-[88%] items-start gap-1.5 rounded-lg px-2 py-1.5",
							message.role === "user"
								? "ml-auto bg-muted text-muted-foreground"
								: "border border-primary/15 bg-primary/[0.035] text-foreground",
						)}
					>
						{message.role === "assistant" ? (
							<BotIcon className="mt-0.5 size-3 shrink-0 text-primary" />
						) : (
							<UserIcon className="mt-0.5 size-3 shrink-0" />
						)}
						<span>{message.content}</span>
					</div>
				))}
				{thinking && (
					<output className="flex items-center gap-1.5 text-muted-foreground">
						<BotIcon className="size-3 text-primary" />
						<span className="motion-safe:animate-pulse">
							Reviewing the board…
						</span>
					</output>
				)}
			</div>

			<div className="border-t border-border/50 bg-background p-2">
				<FlowPilotInput
					mode="chat"
					onSubmit={answer}
					isGenerating={thinking}
					placeholder="Ask FlowPilot to change this flow…"
					className="[&_textarea]:min-h-9 [&_textarea]:text-xs"
				/>
			</div>
		</div>
	);
}
