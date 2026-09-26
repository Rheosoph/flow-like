"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowDownIcon, ArrowUpIcon, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "../../ui/card";
import { formatPercent } from "./sales-format";

export function StatCard({
	title,
	value,
	change,
	icon: Icon,
	subtitle,
}: {
	title: string;
	value: string;
	change?: number | null;
	icon: LucideIcon;
	subtitle?: string;
}) {
	const { t } = useTranslation("settings");
	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium">{title}</CardTitle>
				<Icon className="h-4 w-4 text-muted-foreground" />
			</CardHeader>
			<CardContent>
				<div className="text-2xl font-bold">{value}</div>
				{change !== undefined && change !== null && (
					<p
						className={`text-xs ${change >= 0 ? "text-green-600" : "text-red-600"} flex items-center gap-1`}
					>
						{change >= 0 ? (
							<ArrowUpIcon className="h-3 w-3" />
						) : (
							<ArrowDownIcon className="h-3 w-3" />
						)}
						{t("changeFromLastPeriod", "{{change}} from last period", {
							change: formatPercent(change),
						})}
					</p>
				)}
				{subtitle && (
					<p className="text-xs text-muted-foreground mt-1">{subtitle}</p>
				)}
			</CardContent>
		</Card>
	);
}

export function SectionHeading({
	title,
	description,
	action,
}: {
	title: string;
	description?: string;
	action?: ReactNode;
}) {
	return (
		<div className="flex flex-wrap items-center justify-between gap-3">
			<div>
				<h2 className="text-xl font-semibold">{title}</h2>
				{description && (
					<p className="text-sm text-muted-foreground">{description}</p>
				)}
			</div>
			{action}
		</div>
	);
}

export function EmptyCard({
	icon: Icon,
	message,
	children,
}: {
	icon: LucideIcon;
	message: string;
	children?: ReactNode;
}) {
	return (
		<Card>
			<CardContent className="flex flex-col items-center justify-center py-12 text-center">
				<Icon className="h-12 w-12 text-muted-foreground mb-4" />
				<p className="text-muted-foreground">{message}</p>
				{children}
			</CardContent>
		</Card>
	);
}

export function CustomerCell({
	name,
	avatar,
	fallback,
}: {
	name: string | null;
	avatar: string | null;
	fallback: string;
}) {
	return (
		<div className="flex items-center gap-2">
			{avatar && <img src={avatar} alt="" className="h-6 w-6 rounded-full" />}
			<span>{name || fallback}</span>
		</div>
	);
}
