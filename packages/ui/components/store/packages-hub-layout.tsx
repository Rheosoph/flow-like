"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode, Ref } from "react";

/**
 * Mine and Library share one scrolling region and the same control positions
 * as the Explore hub. A plain `div`: the app shell already renders `<main>`.
 */
export function PackagesHubLayout({
	subtitle,
	toolbar,
	filters,
	actions,
	navigation,
	children,
	scrollRef,
}: Readonly<{
	subtitle: string;
	toolbar: ReactNode;
	filters?: ReactNode;
	actions?: ReactNode;
	navigation?: ReactNode;
	children: ReactNode;
	scrollRef?: Ref<HTMLDivElement>;
}>) {
	const { t } = useTranslation();
	return (
		<div className="flex min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden">
			<div
				ref={scrollRef}
				data-packages-scroll
				className="min-h-0 flex-1 overflow-auto [scrollbar-gutter:stable]"
			>
				<div className="mx-auto w-full max-w-[1600px] px-4 pt-5 pb-12 sm:px-8 sm:pt-6">
					<div data-packages-header className="space-y-4">
						<div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1">
							<div className="flex min-h-11 min-w-0 items-center">
								<h1 className="text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
									{t("packagesHubTitle", "Packages")}
								</h1>
							</div>
							<div className="flex min-h-11 flex-wrap items-center justify-end gap-2">
								{actions}
								{navigation}
							</div>
							<p className="col-span-2 min-h-10 text-sm text-muted-foreground sm:min-h-5">
								{subtitle}
							</p>
						</div>
						<div data-packages-toolbar>{toolbar}</div>
						<div data-packages-filters className="min-h-12">
							{filters}
						</div>
					</div>
					<div
						data-packages-content
						className="mt-6 border-t border-border/50 pt-6 sm:mt-7 sm:pt-7"
					>
						{children}
					</div>
				</div>
			</div>
		</div>
	);
}
