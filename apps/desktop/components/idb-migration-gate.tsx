"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import {
	needsSqliteIdbMigration,
	runSqliteIdbMigration,
} from "../lib/init-idb-sqlite";

/**
 * Keeps data consumers unmounted until the browser checks whether native
 * IndexedDB data needs copying into SQLite and any required migration settles.
 */
export function IdbMigrationGate({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const { t } = useTranslation("common");
	const [phase, setPhase] = useState<"checking" | "migrating" | "ready">(
		"checking",
	);

	useEffect(() => {
		let active = true;
		const finish = () => {
			if (active) setPhase("ready");
		};
		if (needsSqliteIdbMigration()) {
			setPhase("migrating");
			void runSqliteIdbMigration().then(finish, finish);
		} else {
			finish();
		}
		return () => {
			active = false;
		};
	}, []);

	if (phase !== "ready") {
		return (
			<div className="flex h-screen w-screen flex-col items-center justify-center gap-3 bg-background">
				<Loader2 className="size-6 animate-spin text-muted-foreground" />
				{phase === "migrating" && (
					<p className="text-sm text-muted-foreground">
						{t("upgradingLocalStorage", "Upgrading local storage…")}
					</p>
				)}
			</div>
		);
	}

	return <>{children}</>;
}
