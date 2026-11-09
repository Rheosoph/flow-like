"use client";
import { useTranslation } from "@flow-like/locales";
import { TimerIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useUserAvailability } from "../../hooks/use-user-availability";
import {
	AVAILABILITY_STATUSES,
	type AvailabilityStatus,
} from "../../lib/availability";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import { AvailabilityBadge } from "./availability-badge";
import { UserCalendarLinks } from "./user-calendar-links";

export function AvailabilitySettings() {
	const { t } = useTranslation("bots");
	const availability = useUserAvailability();
	const deviceAvailability = useUserAvailability({ local: true });
	const [status, setStatus] = useState<AvailabilityStatus>("focus");
	const [duration, setDuration] = useState(60);
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string>();
	const currentScope = useRef(availability.scope);
	currentScope.current = availability.scope;
	// biome-ignore lint/correctness/useExhaustiveDependencies: Account switches reset pending feedback.
	useEffect(() => {
		setError(undefined);
		setPending(false);
	}, [availability.scope]);
	async function run(action: () => Promise<unknown>) {
		if (pending) return;
		const scope = availability.scope;
		setPending(true);
		setError(undefined);
		try {
			await action();
		} catch (cause) {
			if (scope !== currentScope.current) return;
			setError(
				cause instanceof Error
					? cause.message
					: t(
							"availability.failed",
							"Could not update availability. Try again.",
						),
			);
		} finally {
			if (scope === currentScope.current) setPending(false);
		}
	}
	const selectClass = "h-9 rounded-md border bg-background px-2 text-sm";
	return (
		<Card id="availability">
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<TimerIcon className="size-5 text-muted-foreground" />
					{t("availability.title", "Availability and focus")}
				</CardTitle>
				<CardDescription>
					{t(
						"availability.description",
						"Choose the status other people see. Focus pauses routine notifications until the timer ends; Busy is informational.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-5">
				{!availability.isLocal &&
					deviceAvailability.data &&
					["manual", "bot"].includes(deviceAvailability.data.source) && (
						<div className="space-y-2 rounded-md border p-3">
							<div className="flex flex-wrap items-center gap-2">
								<span className="text-sm font-medium">
									{t("availability.deviceStatus", "This device's status")}
								</span>
								<AvailabilityBadge value={deviceAvailability.data} />
							</div>
							<p className="text-xs text-muted-foreground">
								{t(
									"availability.deviceHelp",
									"This app session can pause foreground alerts. It does not change your shared status or push notifications on other devices.",
								)}
							</p>
							<Button
								variant="outline"
								size="sm"
								disabled={pending}
								onClick={() => void run(deviceAvailability.clear)}
							>
								{t("availability.clearDevice", "Clear device status")}
							</Button>
						</div>
					)}
				{availability.data && (
					<div className="flex flex-wrap items-center gap-2 text-sm">
						<AvailabilityBadge value={availability.data} />
						{availability.data.until && (
							<span className="text-muted-foreground">
								{t("availability.until", "Until {{time}}", {
									time: new Date(availability.data.until).toLocaleTimeString(
										[],
										{ hour: "2-digit", minute: "2-digit" },
									),
								})}
							</span>
						)}
						{availability.data.source === "calendar" && (
							<span className="text-muted-foreground">
								{t("availability.fromCalendar", "From your calendar")}
							</span>
						)}
					</div>
				)}
				<div className="flex flex-wrap items-end gap-2">
					<label className="grid gap-1 text-sm">
						<span>{t("availability.statusLabel", "Status")}</span>
						<select
							className={selectClass}
							value={status}
							disabled={pending}
							onChange={(e) => setStatus(e.target.value as AvailabilityStatus)}
						>
							{AVAILABILITY_STATUSES.map((value) => (
								<option key={value} value={value}>
									{t(`availability.status.${value}`, value)}
								</option>
							))}
						</select>
					</label>
					<label className="grid gap-1 text-sm">
						<span>{t("availability.duration", "Duration")}</span>
						<select
							className={selectClass}
							value={duration}
							disabled={pending}
							onChange={(e) => setDuration(Number(e.target.value))}
						>
							{[15, 30, 60, 120, 240].map((value) => (
								<option key={value} value={value}>
									{t("availability.minutes", "{{count}} minutes", {
										count: value,
									})}
								</option>
							))}
						</select>
					</label>
					<Button
						disabled={pending || availability.isPending || availability.isError}
						onClick={() =>
							void run(() =>
								availability.update({ status, duration_minutes: duration }),
							)
						}
					>
						{t("availability.set", "Set status")}
					</Button>
					{availability.data &&
						["manual", "bot"].includes(availability.data.source) && (
							<Button
								variant="outline"
								disabled={pending}
								onClick={() => void run(availability.clear)}
							>
								{t("availability.clear", "Clear override")}
							</Button>
						)}
				</div>
				<p className="text-xs text-muted-foreground">
					{availability.isLocal
						? t(
								"availability.local",
								"This status applies only in this app session on this device.",
							)
						: t(
								"availability.override",
								"A manual status overrides your calendar for up to four hours. It returns to calendar availability when the timer ends.",
							)}
				</p>
				{availability.data && (
					<fieldset disabled={pending} className="space-y-2 border-t pt-4">
						<legend className="text-sm font-medium">
							{t("availability.exceptions", "Allow during Focus")}
						</legend>
						{(["allow_mentions", "allow_calls"] as const).map((key) => (
							<label key={key} className="flex items-center gap-2 text-sm">
								<input
									type="checkbox"
									checked={availability.data?.preferences[key] ?? true}
									onChange={(e) =>
										void run(() =>
											availability.update({
												preferences: {
													...(availability.data?.preferences ?? {
														allow_mentions: true,
														allow_calls: true,
													}),
													[key]: e.target.checked,
												},
											}),
										)
									}
								/>
								{key === "allow_mentions"
									? t("availability.mentions", "Direct mentions")
									: t("availability.calls", "Incoming calls")}
							</label>
						))}
					</fieldset>
				)}
				{(error || availability.isError) && (
					<p role="alert" className="text-sm text-destructive">
						{error ??
							t(
								"availability.loadFailed",
								"Could not load your availability. Try again.",
							)}{" "}
						<Button
							variant="link"
							size="sm"
							onClick={() => void availability.refetch()}
						>
							{t("availability.retry", "Retry")}
						</Button>
					</p>
				)}
				<div className="border-t pt-4">
					<UserCalendarLinks />
				</div>
			</CardContent>
		</Card>
	);
}
