"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useData } from "../DataContext";
import type { BoundValue } from "../types";

export interface BoundInputOptions<T> {
	/** The component's value-write counter, from `valueRevisionOf`. */
	revision?: number;
	/** Called when an external write replaces what the input was showing. */
	onExternalValue?: (value: T) => void;
}

/**
 * The counter the surface reducer advances on every flow-driven value write. It is
 * runtime machinery rather than an authorable prop, so it is deliberately absent from
 * the component interfaces and read back through here.
 */
export function valueRevisionOf(component: unknown): number | undefined {
	const revision = (component as { valueRevision?: unknown } | null)
		?.valueRevision;
	return typeof revision === "number" ? revision : undefined;
}

function comparable(value: unknown): unknown {
	return typeof value === "object" && value !== null
		? JSON.stringify(value)
		: value;
}

/**
 * The value an input renders, plus the setter that records a user edit.
 *
 * A path binding lives in the data context, so an edit writes straight back there and
 * the context stays authoritative. A literal binding has nowhere to write — only a flow
 * can replace it — so the input keeps the edit locally and goes on rendering it. Without
 * this, every literal-bound input was inert: the typed text or the picked option lasted
 * only as long as the component's own render, because the bound value it displayed had
 * never changed.
 *
 * An external write still wins. It is recognised either by moving the resolved value or
 * by advancing `revision` — the second case is a flow writing the value the field already
 * had declared, such as clearing a composer back to its empty literal.
 */
export function useBoundInputValue<T>(
	bound: BoundValue | undefined,
	fallback: T,
	options: BoundInputOptions<T> = {},
): [T, (next: T) => void] {
	const { resolve, setByPath } = useData();
	const { revision, onExternalValue } = options;

	// A binding can arrive as a raw primitive rather than a BoundValue — `setChecked`
	// and `setProgress` both write one, and `resolve` accepts them — so neither `in`
	// nor a truthiness check is safe on its own.
	const missing = bound === undefined || bound === null;
	const path =
		!missing && typeof bound === "object" && "path" in bound
			? bound.path
			: undefined;
	const external =
		(missing ? undefined : (resolve(bound) as T | undefined)) ?? fallback;

	// `resolve` re-parses a literalJson binding into a fresh object on every render, so
	// the sync has to compare content. Comparing references would read as an unbroken
	// stream of external writes and never settle.
	const externalKey = comparable(external);

	const [local, setLocal] = useState<T>(external);
	const latestRef = useRef<unknown>(externalKey);
	const revisionRef = useRef(revision);

	// The effect keys off the content, so it needs the last committed value handed to it
	// rather than captured — a literalJson binding resolves to a different object each render.
	const externalRef = useRef(external);
	externalRef.current = external;

	useEffect(() => {
		const rewritten = revisionRef.current !== revision;
		revisionRef.current = revision;
		if (!rewritten && externalKey === latestRef.current) return;
		latestRef.current = externalKey;
		setLocal(externalRef.current);
		onExternalValue?.(externalRef.current);
	}, [externalKey, revision, onExternalValue]);

	const setValue = useCallback(
		(next: T) => {
			latestRef.current = comparable(next);
			setLocal(next);
			if (path !== undefined) setByPath(path, next);
		},
		[path, setByPath],
	);

	return [path !== undefined ? external : local, setValue];
}
