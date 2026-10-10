"use client";

import { createContext, type ReactNode, useId } from "react";

export const FootnoteScopeContext = createContext("document");

export function FootnoteScope({ children }: { children: ReactNode }) {
	const scope = useId();
	return (
		<FootnoteScopeContext.Provider value={scope}>
			{children}
		</FootnoteScopeContext.Provider>
	);
}
