/**
 * Which page elements a board reads, computed from the board alone: literal element
 * selectors on read pins (see the element materializer for the selector grammar).
 * `dynamic` is true when some read pin is wired instead of literal, so the run must
 * still be able to fetch elements on demand. `signature` identifies the prerun
 * manifest the demand came from.
 */
export interface IElementDemand {
	selectors: string[];
	dynamic: boolean;
	signature: string;
}
