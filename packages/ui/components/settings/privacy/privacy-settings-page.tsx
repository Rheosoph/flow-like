"use client";

import { BarChart3, Bug, ShieldCheck, Timer } from "lucide-react";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Label } from "../../ui/label";
import { Separator } from "../../ui/separator";
import { Switch } from "../../ui/switch";

export interface PrivacySettingsPageProps {
	available: boolean;
	enabled: boolean | undefined;
	onChange: (enabled: boolean) => void;
	crashReportsEnabled: boolean;
	onCrashReportsChange: (enabled: boolean) => void;
}

interface PrivacyToggleProps {
	id: string;
	icon: React.ReactNode;
	title: string;
	description: React.ReactNode;
	checked: boolean;
	disabled: boolean;
	onChange: (enabled: boolean) => void;
}

function PrivacyToggle({
	id,
	icon,
	title,
	description,
	checked,
	disabled,
	onChange,
}: Readonly<PrivacyToggleProps>) {
	return (
		<div className="flex items-center justify-between gap-4">
			<div className="space-y-0.5">
				<Label htmlFor={id} className="flex items-center gap-2">
					{icon}
					{title}
				</Label>
				<p className="text-sm text-muted-foreground">{description}</p>
			</div>
			<Switch
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onChange}
			/>
		</div>
	);
}

const RETENTION_ROWS: { label: string; period: string }[] = [
	{ label: "Anonymous usage events", period: "30 days" },
	{ label: "Crash & error reports", period: "90 days" },
	{ label: "Session records", period: "90 days" },
	{ label: "Performance samples", period: "30 days" },
	{ label: "Trace spans", period: "7 days" },
	{ label: "Aggregated daily statistics", period: "up to 400 days" },
];

function RetentionNotice() {
	return (
		<div className="space-y-2">
			<Label className="flex items-center gap-2">
				<Timer className="h-4 w-4" />
				How long data is kept
			</Label>
			<ul className="space-y-1 text-sm text-muted-foreground">
				{RETENTION_ROWS.map((row) => (
					<li
						key={row.label}
						className="flex items-center justify-between gap-4"
					>
						<span>{row.label}</span>
						<span className="tabular-nums">{row.period}</span>
					</li>
				))}
			</ul>
			<p className="text-sm text-muted-foreground">
				Individual records are removed by a periodic sweep, so a record can
				outlive its window by a little. The daily statistics are per-day counts;
				one of them records that your install id was active on a given day so
				returning-install counts stay correct. Counters already derived from a
				report &mdash; how often an issue was seen, for instance &mdash; are
				kept after the underlying records are deleted. These periods are the
				defaults; the platform you are connected to may configure different
				ones.
			</p>
		</div>
	);
}

export function PrivacySettingsPage({
	available,
	enabled,
	onChange,
	crashReportsEnabled,
	onCrashReportsChange,
}: Readonly<PrivacySettingsPageProps>) {
	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<ShieldCheck className="h-5 w-5" />
					Privacy &amp; Telemetry
				</CardTitle>
				<CardDescription>
					Control what anonymous diagnostics and usage data Flow-Like may
					collect. Both switches are independent &mdash; turning one off never
					changes the other.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<PrivacyToggle
					id="crash-reports"
					icon={<Bug className="h-4 w-4" />}
					title="Crash & error reports"
					description={
						<>
							On by default. Sends anonymous diagnostics when something breaks:
							the exception type, a scrubbed stack trace and the app version.
							Never a user id, never your IP address, never message, prompt or
							board content.
						</>
					}
					checked={crashReportsEnabled}
					disabled={!available}
					onChange={onCrashReportsChange}
				/>
				<Separator />
				<PrivacyToggle
					id="anonymous-telemetry"
					icon={<BarChart3 className="h-4 w-4" />}
					title="Anonymous usage telemetry"
					description={
						<>
							Opt-in only. Shares aggregate counters and sanitized page paths
							&mdash; never prompts, board content, names, or any personal data.
							Usage events may be sampled, so only a share of page views is
							recorded.
						</>
					}
					checked={enabled === true}
					disabled={!available}
					onChange={onChange}
				/>
				<p className="text-sm text-muted-foreground">
					Both use the same random install id. It is deleted once both switches
					are off, and a fresh one is created if you turn either back on.
				</p>
				<Separator />
				<RetentionNotice />
				{!available && (
					<p className="text-sm text-muted-foreground">
						The connected platform has telemetry disabled, so nothing is
						collected regardless of these settings.
					</p>
				)}
			</CardContent>
		</Card>
	);
}
