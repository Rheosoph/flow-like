"use client";
import "@tm9657/flow-like-ui/globals.css";
import type { Viewport } from "next";

import {
	ExecutionEngineProviderComponent,
	PersistQueryClientProvider,
	QueryClient,
	ReactFlowProvider,
	ThemeProvider,
	Toaster,
	TooltipProvider,
	createIDBPersister,
} from "@tm9657/flow-like-ui";
import { Inter } from "next/font/google";
import { Suspense } from "react";

const inter = Inter({ subsets: ["latin"] });

const persister = createIDBPersister();
const queryClient = new QueryClient({
	defaultOptions: {
		queries: {
			networkMode: "offlineFirst",
			staleTime: 1000, // 10 seconds - balance between cache and freshness
			gcTime: 24 * 60 * 60 * 1000, // 24 hours - cache kept in memory
			refetchOnWindowFocus: false, // Don't refetch on focus (mobile battery optimization)
			refetchOnReconnect: "always", // Refetch when network comes back
			refetchOnMount: false, // Don't refetch on component mount if we have data
			retry: 2, // Only retry failed requests twice
			retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000), // Exponential backoff
		},
	},
});

export const viewport: Viewport = {
	width: "device-width",
	initialScale: 1,
	viewportFit: "cover",
	interactiveWidget: "overlays-content",
};

export default function RootLayout({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	return (
		<html lang="en" suppressHydrationWarning suppressContentEditableWarning>
			{/* <ReactScan /> */}
			{/* <PHProvider> */}
			<ReactFlowProvider>
				<PersistQueryClientProvider
					client={queryClient}
					persistOptions={{
						persister,
						maxAge: 24 * 60 * 60 * 1000,
					}}
				>
					<TooltipProvider>
						<Toaster />
						<body className={inter.className}>
							<Suspense
								fallback={
									<div className="flex flex-1 justify-center items-center">
										{"Loading..."}
									</div>
								}
							>
								<ThemeProvider
									attribute="class"
									defaultTheme="system"
									enableSystem
									disableTransitionOnChange
								>
									<ExecutionEngineProviderComponent>
										{children}
									</ExecutionEngineProviderComponent>
								</ThemeProvider>
							</Suspense>
						</body>
					</TooltipProvider>
				</PersistQueryClientProvider>
			</ReactFlowProvider>
			{/* </PHProvider> */}
		</html>
	);
}
