"use client";

import { Tabs, TabsList, TabsTrigger } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { Download, Package, Search } from "lucide-react";
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
		<div className="flex flex-col h-full space-y-4">
			<div className="flex items-center gap-2">
				<Package className="h-6 w-6" />
				<h1 className="text-xl font-bold">
					{t("customNodes", "Custom Nodes")}
				</h1>
			</div>

			<Tabs value={isExplore ? "explore" : "installed"} className="w-full">
				<TabsList>
					<Link href="/settings/registry/installed">
						<TabsTrigger value="installed" className="gap-2">
							<Download className="h-4 w-4" />
							{t("owned", "Owned")}
						</TabsTrigger>
					</Link>
					<Link href="/settings/registry/explore">
						<TabsTrigger value="explore" className="gap-2">
							<Search className="h-4 w-4" />
							{t("explore", "Explore")}
						</TabsTrigger>
					</Link>
				</TabsList>
			</Tabs>

			<div className="flex-1 min-h-0 overflow-hidden">{children}</div>
		</div>
	);
}
