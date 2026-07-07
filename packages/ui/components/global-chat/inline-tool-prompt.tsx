"use client";

import { motion } from "framer-motion";
import { LayersIcon, ShieldQuestionIcon, SparklesIcon } from "lucide-react";
import { useState } from "react";
import {
	Button,
	Checkbox,
	Textarea,
	cn,
	useBackend,
	useInvoke,
} from "../../index";
import type {
	GlobalToolAsk,
	GlobalToolAskChoice,
	GlobalToolPrompt,
} from "../../state/global-chat/global-chat-store";

const choiceValue = (choice: GlobalToolAskChoice): unknown =>
	choice.value ?? choice.label;

/**
 * Resolves an approval's target app id to its name + icon so the card shows the app's identity
 * instead of the opaque id. The tool's approval sentence (from the backend) embeds the raw id — we
 * swap it for the resolved name so the copy reads naturally too.
 */
function ApprovalAppIdentity({
	appId,
	description,
}: {
	appId: string;
	description?: string;
}) {
	const backend = useBackend();
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		!!appId,
	);
	const name = meta.data?.name || appId;
	const icon = meta.data?.icon ?? meta.data?.thumbnail;
	const resolvedDescription = description?.replace(appId, name);

	return (
		<div className="space-y-2">
			<div className="flex items-center gap-2 rounded-lg border border-border/60 bg-muted/40 px-2.5 py-1.5">
				{icon ? (
					<img
						src={icon}
						alt=""
						className="size-5 rounded-sm object-cover shrink-0"
					/>
				) : (
					<span className="flex items-center justify-center size-5 rounded-sm bg-primary/15 text-primary shrink-0">
						<LayersIcon className="size-3" />
					</span>
				)}
				<span className="min-w-0 flex-1 truncate text-[13px] font-medium">
					{name}
				</span>
			</div>
			{resolvedDescription && (
				<p className="text-xs text-muted-foreground">{resolvedDescription}</p>
			)}
		</div>
	);
}

/** Preselect the default choice (matched by value or label), falling back to the first option. */
function initialSelected(ask: GlobalToolAsk | undefined): Set<number> {
	if (!ask || ask.mode === "freeform" || ask.choices.length === 0)
		return new Set();
	const index = ask.choices.findIndex(
		(choice) =>
			choice.value === ask.defaultValue || choice.label === ask.defaultValue,
	);
	return new Set([index >= 0 ? index : 0]);
}

/**
 * Inline replacement for the former modal tool dialogs: renders the pending approval or question
 * from the global assistant directly in the chat surface, pinned above the input. Questions support
 * freeform text, single choice, and multiple choice, mirroring the `ask_user` tool's modes.
 */
export function InlineToolPrompt({ prompt }: { prompt: GlobalToolPrompt }) {
	const [remember, setRemember] = useState(false);
	const [answer, setAnswer] = useState(() =>
		typeof prompt.ask?.defaultValue === "string" ? prompt.ask.defaultValue : "",
	);
	const [selected, setSelected] = useState(() => initialSelected(prompt.ask));
	const isApproval = prompt.kind === "approval";

	const mode = prompt.ask?.mode ?? "freeform";
	const choices = prompt.ask?.choices ?? [];
	const isChoice = mode === "single_choice" || mode === "multiple_choice";
	const canSend = isChoice ? selected.size > 0 : answer.trim().length > 0;

	const toggle = (index: number) => {
		setSelected((prev) => {
			if (mode === "single_choice") return new Set([index]);
			const next = new Set(prev);
			if (next.has(index)) next.delete(index);
			else next.add(index);
			return next;
		});
	};

	const submit = () => {
		if (!canSend) return;
		if (!isChoice) {
			prompt.respond({ answer });
			return;
		}
		const ordered = Array.from(selected)
			.sort((a, b) => a - b)
			.map((index) => choices[index])
			.filter(Boolean)
			.map(choiceValue);
		prompt.respond({
			answer: mode === "single_choice" ? (ordered[0] ?? null) : ordered,
		});
	};

	return (
		<motion.div
			initial={{ opacity: 0, y: 8, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="w-full rounded-xl border border-amber-500/40 bg-background/80 backdrop-blur-xl shadow-lg overflow-hidden"
		>
			<div className="flex items-center gap-2 px-3 py-2 bg-amber-500/5 border-b border-amber-500/15">
				<span
					className={`flex items-center justify-center size-6 rounded-md shrink-0 ${isApproval ? "bg-amber-500/15 text-amber-600 dark:text-amber-400" : "bg-primary/15 text-primary"}`}
				>
					{isApproval ? (
						<ShieldQuestionIcon className="size-3.5" />
					) : (
						<SparklesIcon className="size-3.5" />
					)}
				</span>
				<span className="min-w-0 flex-1 truncate text-[13px] font-semibold">
					{prompt.title}
				</span>
				<span
					className={`px-1.5 py-0.5 rounded-full text-[10px] font-semibold uppercase tracking-wide shrink-0 ${isApproval ? "bg-amber-500/10 text-amber-600 dark:text-amber-400" : "bg-primary/10 text-primary"}`}
				>
					{isApproval ? "Approval" : "Question"}
				</span>
			</div>
			<div className="px-3 py-2.5 space-y-2.5">
				{prompt.appId ? (
					<ApprovalAppIdentity
						appId={prompt.appId}
						description={prompt.description}
					/>
				) : (
					prompt.description && (
						<p className="text-xs text-muted-foreground">
							{prompt.description}
						</p>
					)
				)}
				{prompt.kind === "ask" ? (
					<>
						{isChoice ? (
							<div className="space-y-1.5">
								{choices.map((choice, index) => {
									const active = selected.has(index);
									return (
										<button
											key={`${choice.label}-${index}`}
											type="button"
											className={cn(
												"flex w-full items-start gap-2.5 rounded-lg border p-2.5 text-left transition-colors",
												active
													? "border-primary/50 bg-primary/10"
													: "border-border/50 bg-background/70 hover:bg-muted/40",
											)}
											onClick={() => toggle(index)}
										>
											<Checkbox
												checked={active}
												className={cn(
													"mt-0.5 pointer-events-none shrink-0",
													mode === "single_choice" && "rounded-full",
												)}
											/>
											<div className="min-w-0">
												<div className="text-xs font-medium">
													{choice.label}
												</div>
												{choice.description && (
													<div className="mt-0.5 text-[11px] text-muted-foreground">
														{choice.description}
													</div>
												)}
											</div>
										</button>
									);
								})}
							</div>
						) : (
							<Textarea
								autoFocus
								value={answer}
								onChange={(e) => setAnswer(e.target.value)}
								placeholder={prompt.ask?.placeholder ?? "Your answer…"}
								className="min-h-20 resize-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
								onKeyDown={(e) => {
									if (e.key === "Enter" && !e.shiftKey && answer.trim()) {
										e.preventDefault();
										submit();
									}
								}}
							/>
						)}
						<div className="flex items-center justify-end gap-2">
							<Button
								variant="ghost"
								size="sm"
								className="outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
								onClick={() => prompt.respond(null)}
							>
								Cancel
							</Button>
							<Button
								size="sm"
								disabled={!canSend}
								className="outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
								onClick={submit}
							>
								Send
							</Button>
						</div>
					</>
				) : (
					<div className="flex flex-wrap items-center justify-between gap-2">
						<label
							htmlFor="inline-tool-prompt-remember"
							className="flex items-center gap-2 text-xs text-muted-foreground"
						>
							<Checkbox
								id="inline-tool-prompt-remember"
								checked={remember}
								onCheckedChange={(checked) => setRemember(checked === true)}
							/>
							Don&apos;t ask again this session
						</label>
						<div className="flex items-center gap-2">
							<Button
								variant="ghost"
								size="sm"
								className="outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
								onClick={() =>
									prompt.respond({ approved: false, remember: false })
								}
							>
								Deny
							</Button>
							<Button
								size="sm"
								className="outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
								onClick={() => prompt.respond({ approved: true, remember })}
							>
								Approve
							</Button>
						</div>
					</div>
				)}
			</div>
		</motion.div>
	);
}
