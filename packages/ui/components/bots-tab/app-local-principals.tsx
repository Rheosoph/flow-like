"use client";

import type { ReactNode } from "react";
import { useBots } from "../../hooks/use-bots";
import { LocalPrincipalsProvider } from "../../hooks/use-user-lookup";
import { useLocalPrincipals } from "./local-principals";

const NONE_LOCAL = () => false;

/** Resolves this device's bots for every identity surface below, not only the Bots tab. */
export function AppLocalPrincipals({
	children,
}: Readonly<{ children: ReactNode }>) {
	const bots = useBots();
	const resolve = useLocalPrincipals(bots.data, NONE_LOCAL);
	return (
		<LocalPrincipalsProvider value={resolve}>
			{children}
		</LocalPrincipalsProvider>
	);
}
