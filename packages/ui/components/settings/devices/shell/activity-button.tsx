"use client";

import { useTranslation } from "@flow-like/locales";
import { Activity, LoaderCircle } from "lucide-react";
import { DvButton } from "../primitives/dv-button";
import { cx } from "../primitives/tone";
import { useActivity } from "../workspace";
import { ACTIVITY_TRAY_ID, trayCounts, useActivityTray } from "./activity-tray";
import { CHROME_BUTTON, CHROME_LABEL, CountPill } from "./attention-button";

export interface ActivityButtonProps {
	className?: string;
}

export interface ActivityButtonViewProps {
	/** Items running or waiting. */
	inFlight: number;
	/** Something is running right now: the icon becomes a spinner. */
	active: boolean;
	open: boolean;
	onToggle: () => void;
	className?: string;
}

/** The Activity button over plain counts. */
export function ActivityButtonView({
	inFlight,
	active,
	open,
	onToggle,
	className,
}: Readonly<ActivityButtonViewProps>) {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			variant="ghost"
			icon={active ? undefined : Activity}
			aria-expanded={open}
			aria-controls={ACTIVITY_TRAY_ID}
			aria-label={t(
				"chrome.activity.name",
				"Activity: {{count, number}} in progress",
				{ count: inFlight },
			)}
			data-chrome="activity"
			data-active={active ? "" : undefined}
			className={cx(CHROME_BUTTON, className)}
			onClick={onToggle}
		>
			{active ? (
				<LoaderCircle aria-hidden className="size-4 animate-spin" />
			) : null}
			<span className={CHROME_LABEL}>
				{t("chrome.activity.label", "Activity")}
			</span>
			<CountPill count={inFlight} />
		</DvButton>
	);
}

/** SPEC §3.2 item 8: in-flight count; toggles the activity tray. */
export function ActivityButton({ className }: Readonly<ActivityButtonProps>) {
	const { open, toggle } = useActivityTray();
	const { items } = useActivity();
	const counts = trayCounts(items);
	return (
		<ActivityButtonView
			inFlight={counts.inFlight}
			active={counts.active > 0}
			open={open}
			onToggle={toggle}
			className={className}
		/>
	);
}
