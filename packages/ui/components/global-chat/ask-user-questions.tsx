"use client";

import { useId } from "react";
import {
	type AskUserDraft,
	type AskUserForm,
	type AskUserQuestion,
	toggleAskUserChoice,
} from "../../lib/ask-user";
import { cn } from "../../lib/utils";
import { Checkbox } from "../ui/checkbox";
import { Textarea } from "../ui/textarea";

/**
 * Renders one `ask_user` form — a single question, or the batched BUILD intake card where every gap
 * is answered in one pass. Shared by the global chat's inline prompt and the board FlowPilot dialog
 * so both surfaces read identically; `size` only trades density for the roomier modal.
 */
export function AskUserQuestions({
	form,
	drafts,
	onDraftsChange,
	onSubmit,
	size = "compact",
	autoFocus = true,
}: {
	form: AskUserForm;
	drafts: AskUserDraft[];
	onDraftsChange: (drafts: AskUserDraft[]) => void;
	/** Enter-to-send. Only wire this for a lone freeform question — in a multi-question form
	 * Enter must insert a newline instead of submitting the questions below. */
	onSubmit?: () => void;
	size?: "compact" | "comfortable";
	autoFocus?: boolean;
}) {
	const roomy = size === "comfortable";
	const numbered = form.questions.length > 1;

	const patch = (index: number, draft: AskUserDraft) => {
		const next = drafts.slice();
		next[index] = draft;
		onDraftsChange(next);
	};

	return (
		<div className={cn("space-y-3", roomy && "space-y-4")}>
			{form.intro && (
				<p
					className={cn("text-muted-foreground", roomy ? "text-sm" : "text-xs")}
				>
					{form.intro}
				</p>
			)}
			{form.questions.map((question, index) => (
				<AskUserQuestionBlock
					key={question.id}
					question={question}
					draft={drafts[index] ?? { text: "", selected: [] }}
					onDraftChange={(draft) => patch(index, draft)}
					index={numbered ? index + 1 : undefined}
					roomy={roomy}
					autoFocus={autoFocus && index === 0}
					onSubmit={numbered ? undefined : onSubmit}
				/>
			))}
		</div>
	);
}

function AskUserQuestionBlock({
	question,
	draft,
	onDraftChange,
	index,
	roomy,
	autoFocus,
	onSubmit,
}: {
	question: AskUserQuestion;
	draft: AskUserDraft;
	onDraftChange: (draft: AskUserDraft) => void;
	index?: number;
	roomy: boolean;
	autoFocus: boolean;
	onSubmit?: () => void;
}) {
	const choiceIdPrefix = useId();
	return (
		<div className="space-y-1.5">
			{index !== undefined && (
				<p
					className={cn(
						"font-medium leading-snug",
						roomy ? "text-sm" : "text-xs",
					)}
				>
					<span className="mr-1.5 text-muted-foreground tabular-nums">
						{index}.
					</span>
					{question.question}
				</p>
			)}
			{question.mode === "freeform" ? (
				<Textarea
					autoFocus={autoFocus}
					value={draft.text}
					onChange={(event) =>
						onDraftChange({ ...draft, text: event.target.value })
					}
					placeholder={question.placeholder ?? "Your answer…"}
					className={cn(
						"resize-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0",
						roomy ? "min-h-28" : "min-h-20",
					)}
					onKeyDown={(event) => {
						if (
							onSubmit &&
							event.key === "Enter" &&
							!event.shiftKey &&
							draft.text.trim()
						) {
							event.preventDefault();
							onSubmit();
						}
					}}
				/>
			) : (
				<div className={cn("space-y-1.5", roomy && "space-y-2")}>
					{question.choices.map((choice, choiceIndex) => {
						const active = draft.selected.includes(choiceIndex);
						const choiceId = `${choiceIdPrefix}-${choiceIndex}`;
						return (
							<label
								key={`${choice.label}-${choiceIndex}`}
								htmlFor={choiceId}
								className={cn(
									"flex w-full cursor-pointer items-start gap-2.5 rounded-lg border text-left transition-colors",
									roomy ? "gap-3 p-3" : "p-2.5",
									active
										? "border-primary/50 bg-primary/10"
										: "border-border/50 bg-background/70 hover:bg-muted/40",
								)}
							>
								<Checkbox
									id={choiceId}
									checked={active}
									onCheckedChange={() =>
										onDraftChange(
											toggleAskUserChoice(question, draft, choiceIndex),
										)
									}
									className={cn(
										"mt-0.5 shrink-0",
										question.mode === "single_choice" && "rounded-full",
									)}
								/>
								<span className="min-w-0">
									<span
										className={cn(
											"block font-medium",
											roomy ? "text-sm" : "text-xs",
										)}
									>
										{choice.label}
									</span>
									{choice.description && (
										<span
											className={cn(
												"mt-0.5 block text-muted-foreground",
												roomy ? "text-xs" : "text-[11px]",
											)}
										>
											{choice.description}
										</span>
									)}
								</span>
							</label>
						);
					})}
				</div>
			)}
		</div>
	);
}
