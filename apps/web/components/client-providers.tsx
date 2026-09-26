"use client";

import dynamic from "next/dynamic";
import { usePathname } from "next/navigation";
import { isHostedFrontendPath } from "../lib/hosted-route";
import { WebNavigationProvider } from "./web-navigation-provider";

const AppProviders = dynamic(() =>
	import("./app-providers").then((module) => module.AppProviders),
);

export function ClientProviders({ children }: { children: React.ReactNode }) {
	const pathname = usePathname();
	// Hosted pages initialize only their publication-scoped runtime. Anonymous
	// visitors must not initialize the account, sidebar, or global execution APIs.
	if (isHostedFrontendPath(pathname)) return <>{children}</>;
	return (
		<WebNavigationProvider>
			<AppProviders>{children}</AppProviders>
		</WebNavigationProvider>
	);
}
