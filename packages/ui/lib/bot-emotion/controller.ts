import type { BotExpression } from "../../components/bots/types";
import type { EmotionClient } from "./client";
import {
	type EmotionInput,
	type EmotionLoadState,
	type ExpressionScores,
	boundEmotionInput,
} from "./protocol";
import { chooseExpression, smoothScores } from "./scoring";

export interface BotExpressionState extends EmotionLoadState {
	expression: BotExpression;
}

export const EMOTION_THROTTLE_MS = 300;
export const EXPRESSION_HOLD_MS = 900;

export class BotExpressionController {
	private owner = {};
	private revision = 0;
	private input: EmotionInput = { draft: "", messages: [] };
	private state: BotExpressionState = { expression: "neutral", status: "idle" };
	private previous?: ExpressionScores;
	private lastInference = 0;
	private changedAt = 0;
	private inferTimer?: ReturnType<typeof setTimeout>;
	private holdTimer?: ReturnType<typeof setTimeout>;
	private warmTimer?: ReturnType<typeof setTimeout>;
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
				this.previous = undefined;
			}
			this.state = {
				...state,
				expression:
					state.status === "unavailable" ? "neutral" : this.state.expression,
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
		this.input = bounded;
		this.revision++;
		this.client.cancel(this.owner);
		clearTimeout(this.holdTimer);
		if (!bounded.draft) {
			clearTimeout(this.inferTimer);
			this.inferTimer = undefined;
			this.previous = undefined;
			this.setExpression("neutral");
			this.changedAt = 0;
			return;
		}
		if (this.inferTimer !== undefined) return;
		const wait = this.lastInference
			? Math.max(0, EMOTION_THROTTLE_MS - (Date.now() - this.lastInference))
			: EMOTION_THROTTLE_MS;
		this.inferTimer = setTimeout(() => {
			this.inferTimer = undefined;
			this.lastInference = Date.now();
			const revision = this.revision;
			void this.client.predict(this.owner, this.input).then((scores) => {
				if (this.disposed || revision !== this.revision || !scores) return;
				this.previous = smoothScores(scores, this.previous);
				const expression = chooseExpression(this.previous);
				if (expression === this.state.expression) return;
				const delay = Math.max(
					0,
					this.changedAt + EXPRESSION_HOLD_MS - Date.now(),
				);
				if (!delay) {
					this.setExpression(expression);
					return;
				}
				this.holdTimer = setTimeout(() => {
					if (!this.disposed && revision === this.revision)
						this.setExpression(expression);
				}, delay);
			});
		}, wait);
	}

	private setExpression(expression: BotExpression) {
		if (expression === this.state.expression) return;
		this.changedAt = Date.now();
		this.state = { ...this.state, expression };
		this.onChange(this.state);
	}

	dispose() {
		this.disposed = true;
		this.revision++;
		clearTimeout(this.inferTimer);
		clearTimeout(this.holdTimer);
		this.client.cancel(this.owner);
		clearTimeout(this.warmTimer);
		this.unsubscribe();
	}
}
