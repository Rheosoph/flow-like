"use client";
import { Shield } from "lucide-react";
import type { IAppVisibility } from "../../lib/schema/app/app";
import { visibilityMeta } from "../settings/visibility-status/visibility-meta";

export function visibilityLabel(v: IAppVisibility) {
	return visibilityMeta(v)?.badgeLabel ?? "Unknown";
}

export function visibilityIcon(v: IAppVisibility) {
	const cl = "h-4 w-4";
	const Icon = visibilityMeta(v)?.BadgeIcon ?? Shield;
	return <Icon className={cl} />;
}
