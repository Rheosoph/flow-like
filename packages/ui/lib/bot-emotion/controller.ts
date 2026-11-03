import type { BotExpression } from "../../components/bots/types";
import type { EmotionClient } from "./client";
import { draftCue, explicitDraftCue } from "./draft-cues";
import {
	type EmotionInput,
	type EmotionLoadState,
	type ExpressionScores,
	MAX_CONTEXT_CHARS,
	boundEmotionInput,
} from "./protocol";
import { chooseExpression, smoothScores } from "./scoring";

export interface BotExpressionState extends EmotionLoadState {
	expression: BotExpression;
	/** Keep a submitted reaction briefly before the working face resumes. */
	lingering?: boolean;
}

export const EMOTION_THROTTLE_MS = 300;
export const EXPRESSION_HOLD_MS = 900;
export const SENT_REACTION_MS = 2_000;
const SEND_HANDOFF_MS = 600;

export class BotExpressionController {
	private owner = {};
	private input: EmotionInput = { draft: "", messages: [] };
	private active?: EmotionInput;
	private held?: EmotionInput;
	private state: BotExpressionState = { expression: "neutral", status: "idle" };
	private previous?: ExpressionScores;
	private lastInference = 0;
	private changedAt = 0;
	private inferTimer?: ReturnType<typeof setTimeout>;
	private holdTimer?: ReturnType<typeof setTimeout>;
	private warmTimer?: ReturnType<typeof setTimeout>;
	private settleTimer?: ReturnType<typeof setTimeout>;
	private draftContext: EmotionInput["messages"] = [];
	private clearedDraft?: { text: string; expression: BotExpression };
	private disposed = false;
	private unsubscribe: () => void;

	constructor(
		private client: EmotionClient,
		private onChange: (state: BotExpressionState) => void,
	) {
		this.unsubscribe = client.subscribe((state) => {
			if (this.disposed) return;
			if (state.status === "unavailable") {
				clearTimeout(this.holdTimer);
				this.held = undefined;
				this.previous = undefined;
			}
			const lingering = this.state.lingering;
			this.state = {
				...state,
				...(lingering ? { lingering } : {}),
				expression:
					!lingering &&
					(state.status === "unavailable" || state.status === "loading")
						? draftCue(this.input.draft)
						: this.state.expression,
			};
			this.onChange(this.state);
		});
		this.warmTimer = setTimeout(() => {
			if (!this.disposed) this.client.warmup();
		}, 750);
	}

	update(input: EmotionInput) {
		if (this.disposed) return;
		const bounded = boundEmotionInput(input);
		if (JSON.stringify(bounded) === JSON.stringify(this.input)) return;
		const prior = this.input;
		this.input = bounded;
		if (!bounded.draft) {
			if (prior.draft) {
				this.clearedDraft = {
					text: prior.draft.slice(-MAX_CONTEXT_CHARS),
					expression: this.state.expression,
				};
				clearTimeout(this.settleTimer);
				this.settleTimer = setTimeout(() => {
					this.clearedDraft = undefined;
				}, SEND_HANDOFF_MS);
			}
			this.client.cancel(this.owner);
			this.active = undefined;
			clearTimeout(this.holdTimer);
			this.held = undefined;
			clearTimeout(this.inferTimer);
			this.inferTimer = undefined;
			this.previous = undefined;
			if (this.state.lingering) return;
			if (this.clearedDraft && this.wasSubmitted()) {
				const { expression } = this.clearedDraft;
				this.clearedDraft = undefined;
				clearTimeout(this.settleTimer);
				if (expression !== "neutral") {
					this.setExpression(expression, true);
					this.settleTimer = setTimeout(
						() => this.setExpression("neutral"),
						SENT_REACTION_MS,
					);
					return;
				}
			}
			this.setExpression("neutral");
			this.changedAt = 0;
			return;
		}
		clearTimeout(this.settleTimer);
		this.clearedDraft = undefined;
		if (!prior.draft) {
			this.draftContext = bounded.messages;
			if (this.state.lingering) this.setExpression("neutral");
		}
		if (this.held && !this.canApply(this.held)) {
			clearTimeout(this.holdTimer);
			this.held = undefined;
		}
		if (
			this.active &&
			(this.state.status === "loading" || !this.canApply(this.active))
		) {
			this.client.cancel(this.owner);
			this.active = undefined;
		}
		const explicit = explicitDraftCue(bounded.draft);
		if (explicit) {
			if (
				explicit !== explicitDraftCue(prior.draft) ||
				this.state.status !== "ready"
			) {
				clearTimeout(this.holdTimer);
				this.held = undefined;
				this.previous = undefined;
				this.setExpression(explicit);
			}
		} else if (this.state.status !== "ready")
			this.setExpression(draftCue(bounded.draft));
		else if (explicitDraftCue(prior.draft)) this.setExpression("neutral");
		this.scheduleInference();
	}

	private wasSubmitted() {
		// Only newly appended user text confirms a send. A matching old message
		// must not make deleting a repeated draft look like another submission.
		const messages = this.input.messages;
		const users = (items: EmotionInput["messages"]) =>
			items.filter((message) => message.role === "user");
		if (
			JSON.stringify(users(this.draftContext)) ===
			JSON.stringify(users(messages))
		)
			return false;
		let overlap = Math.min(this.draftContext.length, messages.length);
		while (
			overlap > 0 &&
			JSON.stringify(this.draftContext.slice(-overlap)) !==
				JSON.stringify(messages.slice(0, overlap))
		)
			overlap--;
		const latestUser = messages.findLast((message) => message.role === "user");
		return messages
			.slice(overlap)
			.some(
				(message) =>
					message === latestUser && message.content === this.clearedDraft?.text,
			);
	}

	private canApply(input: EmotionInput) {
		// Keep a result while more words are appended, but never apply a replaced draft.
		return (
			this.input.draft.startsWith(input.draft) &&
			explicitDraftCue(input.draft) === explicitDraftCue(this.input.draft) &&
			JSON.stringify(this.input.messages) === JSON.stringify(input.messages)
		);
	}

	private scheduleInference() {
		if (this.disposed || this.active || this.inferTimer !== undefined) return;
		const wait = this.lastInference
			? Math.max(0, EMOTION_THROTTLE_MS - (Date.now() - this.lastInference))
			: EMOTION_THROTTLE_MS;
		this.inferTimer = setTimeout(() => {
			this.inferTimer = undefined;
			this.lastInference = Date.now();
			const input = this.input;
			this.active = input;
			void this.client.predict(this.owner, input).then((scores) => {
				if (this.disposed || this.active !== input) return;
				this.active = undefined;
				if (scores && this.canApply(input)) {
					// Smooth repeated evidence for the same face. Mixing a new emotion
					// with the preceding neutral result can otherwise erase its signal
					// until another keystroke happens to trigger another inference.
					this.previous =
						this.previous &&
						chooseExpression(this.previous) === chooseExpression(scores)
							? smoothScores(scores, this.previous)
							: scores;
					this.showExpression(chooseExpression(this.previous), input);
				}
				// Only one inference runs per avatar; continuous typing queues the latest draft.
				if (this.input !== input) this.scheduleInference();
			});
		}, wait);
	}

	private showExpression(expression: BotExpression, input: EmotionInput) {
		clearTimeout(this.holdTimer);
		this.held = undefined;
		if (expression === this.state.expression) return;
		const delay =
			this.state.expression === "neutral"
				? 0
				: Math.max(0, this.changedAt + EXPRESSION_HOLD_MS - Date.now());
		if (!delay) {
			this.setExpression(expression);
			return;
		}
		this.held = input;
		this.holdTimer = setTimeout(() => {
			this.held = undefined;
			if (!this.disposed && this.canApply(input))
				this.setExpression(expression);
		}, delay);
	}

	private setExpression(expression: BotExpression, lingering = false) {
		if (
			expression === this.state.expression &&
			Boolean(this.state.lingering) === lingering
		)
			return;
		this.changedAt = Date.now();
		const { lingering: _previousLingering, ...state } = this.state;
		this.state = { ...state, expression, ...(lingering ? { lingering } : {}) };
		this.onChange(this.state);
	}

	dispose() {
		this.disposed = true;
		clearTimeout(this.inferTimer);
		clearTimeout(this.holdTimer);
		clearTimeout(this.settleTimer);
		this.clearedDraft = undefined;
		this.client.cancel(this.owner);
		clearTimeout(this.warmTimer);
		this.unsubscribe();
	}
}
