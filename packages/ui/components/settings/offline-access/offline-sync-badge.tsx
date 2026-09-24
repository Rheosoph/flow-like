"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	CloudOffIcon,
	LogInIcon,
	type LucideIcon,
	RefreshCwIcon,
	TriangleAlertIcon,
} from "lucide-react";
import type { OfflineSyncState } from "../../../state/backend-state/offline-writes-state";
import { Badge } from "../../ui/badge";
import { syncStateLabel } from "./offline-access-logic";

const APPEARANCE: Record<
	OfflineSyncState,
	{ icon: LucideIcon; variant: "secondary" | "outline" | "destructive" }
> = {
	idle: { icon: CheckIcon, variant: "secondary" },
	syncing: { icon: RefreshCwIcon, variant: "outline" },
	waitingForConnection: { icon: CloudOffIcon, variant: "outline" },
	waitingForSignIn: { icon: LogInIcon, variant: "outline" },
	hubError: { icon: TriangleAlertIcon, variant: "outline" },
	blocked: { icon: TriangleAlertIcon, variant: "destructive" },
};

export function OfflineSyncBadge({
	state,
	account,
}: Readonly<{ state: OfflineSyncState; account?: string | null }>) {
	const { t } = useTranslation("settings");
	const { icon: Icon, variant } = APPEARANCE[state];
	return (
		<Badge variant={variant} className="max-w-full whitespace-normal">
			<Icon className={state === "syncing" ? "animate-spin" : undefined} />
			{syncStateLabel(t, state, account)}
		</Badge>
	);
}
