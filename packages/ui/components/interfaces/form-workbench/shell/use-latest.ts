"use client";

import { type RefObject, useRef } from "react";
import { useIsoLayoutEffect } from "./focus";

/** A ref that always holds the value of the last committed render, for handlers that must stay stable. */
export function useLatest<T>(value: T): RefObject<T> {
	const ref = useRef(value);
	useIsoLayoutEffect(() => {
		ref.current = value;
	});
	return ref;
}
