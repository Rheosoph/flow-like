import {
	type BoardFacts,
	type FactTransition,
	MAX_BAG,
	type SuggestionContext,
	type TransitionFact,
} from "./types";

function bagOf(types: string[], end: number): string[] {
	const bag: string[] = [];
	for (let index = end - 1; index >= 0 && bag.length < MAX_BAG; index--) {
		bag.push(types[index]);
	}
	return bag;
}

export function expandTransition(
	facts: BoardFacts,
	transition: FactTransition,
): TransitionFact {
	const { types } = facts;
	return {
		kind: transition.kind,
		src: types[transition.src],
		srcPin: transition.srcPin,
		dataType: transition.dataType,
		valueType: transition.valueType,
		schema: transition.schema,
		pred: transition.pred.map((index) => types[index]),
		ups: transition.ups.map((index) => types[index]),
		bag: bagOf(types, transition.bag),
		dst: types[transition.dst],
		dstPin: transition.dstPin,
	};
}

export function* iterateTransitions(
	corpus: Iterable<BoardFacts>,
): Generator<TransitionFact> {
	for (const facts of corpus) {
		for (const transition of facts.transitions) {
			yield expandTransition(facts, transition);
		}
	}
}

export function contextOf(fact: TransitionFact): SuggestionContext {
	const { dst: _dst, dstPin: _dstPin, ...context } = fact;
	return context;
}
