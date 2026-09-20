"use client";

import { useRouter } from "next/navigation";
import { createContext, useContext, useMemo } from "react";

export interface ClientNavigation {
	navigate: (
		href: string,
		replace: boolean,
		options?: { scroll?: boolean },
	) => void;
	href: (href: string) => string;
}

/** Hosts can adapt navigation while other clients retain their Next router. */
export const ClientNavigationContext = createContext<
	ClientNavigation | undefined
>(undefined);

export function useClientRouter(): ReturnType<typeof useRouter> {
	const router = useRouter();
	const navigation = useContext(ClientNavigationContext);
	return useMemo<ReturnType<typeof useRouter>>(
		() =>
			navigation
				? {
						...router,
						push: (href, options) => navigation.navigate(href, false, options),
						replace: (href, options) =>
							navigation.navigate(href, true, options),
					}
				: router,
		[router, navigation],
	);
}

const unchangedHref = (href: string) => href;

export function useClientHref(): (href: string) => string {
	return useContext(ClientNavigationContext)?.href ?? unchangedHref;
}
