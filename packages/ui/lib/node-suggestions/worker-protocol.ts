/**
 * Messages of the node-suggestion worker and the logic behind them. The worker is compute only:
 * it extracts facts, trains and serves models; persistence and the backend stay on the main thread.
 * Corpora travel as the facts JSON strings the store holds, so the main thread never parses them
 * and `postMessage` clones flat strings instead of deep object graphs.
 */
import type { IBoard } from "../schema/flow/board";
import { suggest } from "./ensemble";
import { extractBoardFacts } from "./extract";
import { NeuralModel, type NeuralTrainProgress } from "./neural/model";
import { NgramModel } from "./ngram";
import type {
	BoardFacts,
	CatalogEntry,
	SuggestionContext,
	SuggestionResult,
} from "./types";

export type NodeSuggestionRequest =
	| { type: "load"; id: number; ngram?: string; neural?: string }
	| { type: "extract"; id: number; board: IBoard }
	| { type: "trainNgram"; id: number; corpus: string[] }
	| {
			type: "trainNeural";
			id: number;
			corpus: string[];
			catalog: CatalogEntry[];
			/** Continue from the neural model currently loaded, if any. */
			warmStart: boolean;
	  }
	| { type: "cancelTraining"; id: number }
	| {
			type: "suggest";
			id: number;
			context: SuggestionContext;
			limit?: number;
	  };

export type NodeSuggestionResponse =
	| { type: "loaded"; id: number; ngram: boolean; neural: boolean }
	| { type: "facts"; id: number; facts: BoardFacts }
	| {
			type: "trained";
			id: number;
			kind: "ngram" | "neural";
			payload: string;
			boards: number;
			transitions: number;
	  }
	| { type: "progress"; id: number; progress: NeuralTrainProgress }
	| { type: "suggestion"; id: number; result: SuggestionResult | undefined }
	| { type: "cancelled"; id: number }
	| { type: "error"; id: number; message: string };

function parseCorpus(corpus: readonly string[]): BoardFacts[] {
	const boards: BoardFacts[] = [];
	for (const json of corpus) {
		try {
			boards.push(JSON.parse(json) as BoardFacts);
		} catch {
			// A corrupted row costs one board, not the model.
		}
	}
	return boards;
}

const WARM_START_EPOCHS = 6;

const countTransitions = (corpus: readonly BoardFacts[]) =>
	corpus.reduce((sum, facts) => sum + facts.transitions.length, 0);

const messageOf = (error: unknown) =>
	error instanceof Error ? error.message : String(error);

export class NodeSuggestionWorkerCore {
	private ngram: NgramModel | undefined;
	private neural: NeuralModel | undefined;
	private training: AbortController | undefined;

	constructor(
		private readonly post: (response: NodeSuggestionResponse) => void,
	) {}

	async handle(request: NodeSuggestionRequest): Promise<void> {
		try {
			switch (request.type) {
				case "load":
					return this.load(request);
				case "extract":
					return this.post({
						type: "facts",
						id: request.id,
						facts: extractBoardFacts(request.board),
					});
				case "trainNgram":
					return this.trainNgram(request.id, request.corpus);
				case "trainNeural":
					return await this.trainNeural(request);
				case "cancelTraining":
					this.training?.abort();
					return;
				case "suggest":
					return this.post({
						type: "suggestion",
						id: request.id,
						result: this.ngram
							? suggest(
									{ ngram: this.ngram, neural: this.neural },
									request.context,
									request.limit,
								)
							: undefined,
					});
			}
		} catch (error) {
			this.post({ type: "error", id: request.id, message: messageOf(error) });
		}
	}

	private load(request: Extract<NodeSuggestionRequest, { type: "load" }>) {
		if (request.ngram !== undefined) {
			try {
				this.ngram = NgramModel.deserialize(request.ngram);
			} catch (error) {
				console.warn(
					"[node-suggestions] cached n-gram model unreadable:",
					error,
				);
			}
		}
		if (request.neural !== undefined) {
			try {
				this.neural = NeuralModel.deserialize(request.neural);
			} catch (error) {
				console.warn(
					"[node-suggestions] cached neural model unreadable:",
					error,
				);
			}
		}
		this.post({
			type: "loaded",
			id: request.id,
			ngram: this.ngram !== undefined,
			neural: this.neural !== undefined,
		});
	}

	private trainNgram(id: number, corpus: readonly string[]) {
		const boards = parseCorpus(corpus);
		const model = NgramModel.train(boards);
		this.ngram = model;
		this.post({
			type: "trained",
			id,
			kind: "ngram",
			payload: model.serialize(),
			boards: boards.length,
			transitions: countTransitions(boards),
		});
	}

	private async trainNeural(
		request: Extract<NodeSuggestionRequest, { type: "trainNeural" }>,
	) {
		this.training?.abort();
		const controller = new AbortController();
		this.training = controller;
		const boards = parseCorpus(request.corpus);
		const warmStart = request.warmStart ? this.neural : undefined;
		try {
			const model = await NeuralModel.train(boards, request.catalog, {
				signal: controller.signal,
				onProgress: (progress: NeuralTrainProgress) =>
					this.post({ type: "progress", id: request.id, progress }),
				warmStart,
				// Measured: a warm start converges to fresh-run quality in half the epochs.
				epochs: warmStart ? WARM_START_EPOCHS : undefined,
			});
			if (controller.signal.aborted) {
				this.post({ type: "cancelled", id: request.id });
				return;
			}
			this.neural = model;
			this.post({
				type: "trained",
				id: request.id,
				kind: "neural",
				payload: model.serialize(),
				boards: boards.length,
				transitions: countTransitions(boards),
			});
		} catch (error) {
			if (!controller.signal.aborted) throw error;
			this.post({ type: "cancelled", id: request.id });
		} finally {
			if (this.training === controller) this.training = undefined;
		}
	}
}
