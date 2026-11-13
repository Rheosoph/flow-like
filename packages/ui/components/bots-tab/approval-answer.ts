"use client";

import { useState } from "react";
import { useAnswerBotApproval } from "../../hooks/use-bots";
import type { IBotApprovalItemDecision } from "../../state/backend-state/bot-state";
import {
	APPROVAL_EXPIRED,
	type ChoiceOf,
	TURN_NOT_WAITING,
	isPastWait,
	refusalCode,
} from "./approval-batch-model";
import type { ApprovalDecision } from "./approval-entry";
import type { ConversationApproval } from "./fold-entries";

export type ApprovalClosure = "expired" | "cancelled";

export interface ApprovalAnswerState {
	/** The top-level answer, from the conversation or sent from this card. */
	answer?: ApprovalDecision;
	/** The answer each call got, once there is one. */
	choiceOf: ChoiceOf;
	/** The card can no longer be answered and has no answer. */
	closure?: ApprovalClosure;
	/** Another device answered first; its answer has not arrived yet. */
	elsewhere: boolean;
	failed: boolean;
	pending: boolean;
	send: (
		decision: ApprovalDecision,
		items?: readonly IBotApprovalItemDecision[],
		onSent?: () => void,
	) => void;
	sendAsync: (decision: ApprovalDecision) => Promise<void>;
}

interface Sent {
	decision: ApprovalDecision;
	items: readonly IBotApprovalItemDecision[];
}

/** The answer state one approval card shares between its single and its batch layout. */
export function useApprovalAnswer(
	approval: ConversationApproval,
	botId: string,
): ApprovalAnswerState {
	const { mutate, mutateAsync, isPending, isError, error, variables } =
		useAnswerBotApproval();
	const [sent, setSent] = useState<Sent>();
	const request = (
		decision: ApprovalDecision,
		items: readonly IBotApprovalItemDecision[] = [],
	) => ({
		botId,
		turnId: approval.turnId,
		approvalId: approval.id,
		decision,
		items,
	});
	const code = isError && !isPending ? refusalCode(error) : undefined;
	const itemAnswers = approval.answer
		? approval.itemAnswers
		: sent &&
			Object.fromEntries(
				sent.items.map((item) => [item.args_hash, item.decision]),
			);
	const answer = approval.answer ?? sent?.decision;
	const closure: ApprovalClosure | undefined = answer
		? undefined
		: approval.cancelled
			? "cancelled"
			: code === APPROVAL_EXPIRED || isPastWait(approval.postedAt, Date.now())
				? "expired"
				: undefined;
	return {
		answer,
		choiceOf: (item) => itemAnswers?.[item.argsHash] ?? answer,
		closure,
		elsewhere: !answer && !closure && code === TURN_NOT_WAITING,
		failed:
			isError &&
			!isPending &&
			variables !== undefined &&
			code !== TURN_NOT_WAITING &&
			code !== APPROVAL_EXPIRED,
		pending: isPending,
		send: (decision, items, onSent) =>
			mutate(request(decision, items), {
				onSuccess: () => {
					setSent({ decision, items: items ?? [] });
					onSent?.();
				},
			}),
		sendAsync: async (decision) => {
			await mutateAsync(request(decision));
			setSent({ decision, items: [] });
		},
	};
}
