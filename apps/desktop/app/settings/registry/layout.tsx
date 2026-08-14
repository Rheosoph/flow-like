"use client";

import { useTranslation } from "@flow-like/locales";
import { Download, Search } from "lucide-react";
import Link from "next/link";
import { usePathname } from "next/navigation";

export default function RegistryLayout({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	const { t } = useTranslation("common");
	const pathname = usePathname();
	const isExplore = pathname === "/settings/registry/explore";

	return (
		<div className="flex flex-col h-full">
			<div className="px-4 sm:px-8 pt-5 space-y-4">
				<div>
					<h1 className="text-2xl font-semibold tracking-tight">
						{t('customNodes', 'Custom Nodes')}
					</h1>
					<p className="text-sm text-muted-foreground/70">
						{t('browseAndManageNodePackages', 'Browse and manage node packages')}
					</p>
				</div>

				<div className="flex items-center gap-1.5">
					<Link href="/settings/registry/installed">
						<div
							className={`flex items-center gap-1.5 px-4 py-1.5 rounded-full text-sm transition-all ${
								!isExplore
									? "bg-muted/40 text-foreground font-medium"
									: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/20"
							}`}
						>
							<Download className="h-3.5 w-3.5" />
							{t('owned', 'Owned')}
						</div>
					</Link>
					<Link href="/settings/registry/explore">
						<div
							className={`flex items-center gap-1.5 px-4 py-1.5 rounded-full text-sm transition-all ${
								isExplore
									? "bg-muted/40 text-foreground font-medium"
									: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/20"
							}`}
						>
							<Search className="h-3.5 w-3.5" />
							{t('explore', 'Explore')}
						</div>
					</Link>
				</div>
			</div>

			<div className="flex-1 min-h-0 overflow-hidden px-4 sm:px-8 pt-4">
				{children}
			</div>
		</div>
	);
}
