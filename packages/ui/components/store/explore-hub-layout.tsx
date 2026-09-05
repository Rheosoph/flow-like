"use client";

import type { ReactNode, Ref } from "react";
import { ExploreHubHeader, type ExploreHubTab } from "./explore-hub-header";

/** Both catalogs share one scrolling region and the same control positions. */
export function ExploreHubLayout({
	active,
	subtitle,
	toolbar,
	filters,
	children,
	scrollRef,
}: Readonly<{
	active: ExploreHubTab;
	subtitle: string;
	toolbar: ReactNode;
	filters?: ReactNode;
	children: ReactNode;
	scrollRef?: Ref<HTMLDivElement>;
}>) {
	return (
		<main className="flex min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden">
			<div
				ref={scrollRef}
				data-explore-scroll
				className="min-h-0 flex-1 overflow-auto [scrollbar-gutter:stable]"
			>
				<div className="mx-auto w-full max-w-[1600px] px-4 pt-5 pb-12 sm:px-8 sm:pt-6">
					<div data-explore-header className="space-y-4">
						<ExploreHubHeader active={active} subtitle={subtitle} />
						<div data-explore-toolbar>{toolbar}</div>
						<div data-explore-filters className="min-h-12">
							{filters}
						</div>
					</div>
					<div
						data-explore-content
						className="mt-6 border-t border-border/50 pt-6 sm:mt-7 sm:pt-7"
					>
						{children}
					</div>
				</div>
			</div>
		</main>
	);
}
