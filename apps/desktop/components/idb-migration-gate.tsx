"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import {
	needsSqliteIdbMigration,
	runSqliteIdbMigration,
} from "../lib/init-idb-sqlite";

/**
 * Blocks the app tree on first launch after the SQLite-IndexedDB switch
 * while native IndexedDB data is copied over. Renders children immediately
 * on every later launch (the check is a synchronous localStorage read).
 */
export function IdbMigrationGate({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const { t } = useTranslation("common");
	const [migrating, setMigrating] = useState(
		() => typeof window !== "undefined" && needsSqliteIdbMigration(),
	);

	useEffect(() => {
		if (!migrating) return;
		let active = true;
		runSqliteIdbMigration().finally(() => {
			if (active) setMigrating(false);
		});
		return () => {
			active = false;
		};
	}, [migrating]);

	if (migrating) {
		return (
			<div
				className="flex h-screen w-screen flex-col items-center justify-center gap-3 bg-background"
				suppressHydrationWarning
			>
				<Loader2 className="size-6 animate-spin text-muted-foreground" />
				<p className="text-sm text-muted-foreground">
					{t('upgradingLocalStorage', 'Upgrading local storage…')}
				</p>
			</div>
		);
	}

	return <>{children}</>;
}
