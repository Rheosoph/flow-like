import { useDebounce } from "@uidotdev/usehooks";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { EMPTY_FILTER, type ILogFilter, scopeToNode } from "./filter-model";
import { type INodeRef, parseQueryInput } from "./query-tokens";
import { applyParsed } from "./suggestions";

const INPUT_DEBOUNCE_MS = 250;

/**
 * Committed chips plus the text still being typed. A node scope handed in from
 * the board badge becomes an include chip; removing that chip hands the scope
 * back through `onClearNodeIdFilter`.
 */
export function useLogFilter({
	nodes,
	nodeIdFilter,
	onClearNodeIdFilter,
}: {
	nodes: readonly INodeRef[];
	nodeIdFilter?: string;
	onClearNodeIdFilter?: () => void;
}) {
	const [filter, setFilterState] = useState<ILogFilter>(() =>
		nodeIdFilter ? scopeToNode(EMPTY_FILTER, nodeIdFilter) : EMPTY_FILTER,
	);
	const filterRef = useRef(filter);
	filterRef.current = filter;
	const scope = useRef({ nodeIdFilter, onClearNodeIdFilter });
	scope.current = { nodeIdFilter, onClearNodeIdFilter };

	const setFilter = useCallback((next: ILogFilter) => {
		setFilterState(next);
		const { nodeIdFilter: scoped, onClearNodeIdFilter: clear } = scope.current;
		const kept = next.chips.some(
			(chip) => chip.kind === "node" && !chip.negated && chip.value === scoped,
		);
		if (scoped && !kept) clear?.();
	}, []);

	useEffect(() => {
		if (nodeIdFilter) {
			setFilterState((current) => scopeToNode(current, nodeIdFilter));
		}
	}, [nodeIdFilter]);

	const [input, setInput] = useState("");
	const debouncedInput = useDebounce(input, INPUT_DEBOUNCE_MS);
	const effective = useMemo(
		() =>
			debouncedInput.trim()
				? applyParsed(
						filter,
						parseQueryInput(debouncedInput, nodes, { commit: false }),
					)
				: filter,
		[filter, debouncedInput, nodes],
	);

	const clearFilters = useCallback(() => {
		setFilter(EMPTY_FILTER);
		setInput("");
	}, [setFilter]);

	return {
		filter,
		filterRef,
		setFilter,
		input,
		setInput,
		effective,
		clearFilters,
	};
}
