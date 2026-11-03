"use client";

import { BotsView } from "@flow-like/flow-like-ui/components/bots-tab";
import { Suspense } from "react";

export default function BotsPage() {
	return (
		<main className="flex flex-col flex-1 w-full min-h-0 overflow-hidden bg-background">
			<Suspense fallback={null}>
				<BotsView />
			</Suspense>
		</main>
	);
}
