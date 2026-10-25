"use client";

import { createContext, useContext } from "react";

/** True inside a chat `a2ui` surface; a nested `a2ui` fence then renders as plain code. */
export const A2UIFenceContext = createContext(false);

export function useIsInsideA2UIFence(): boolean {
	return useContext(A2UIFenceContext);
}
