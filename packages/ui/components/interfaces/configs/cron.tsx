"use client";

import { Trans, i18n as i18next, useTranslation } from "@flow-like/locales";
import { CronExpressionParser } from "cron-parser";
import { useEffect, useMemo, useState } from "react";

import { CheckIcon, ChevronsUpDown, Clock } from "lucide-react";

import { cn } from "../../../lib/utils";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	Badge,
	Button,
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
	Input,
	Label,
	Popover,
	PopoverContent,
	PopoverTrigger,
	RadioGroup,
	RadioGroupItem,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
} from "../../ui";
import { Calendar } from "../../ui/calendar";
import type { IConfigInterfaceProps } from "../interfaces";
import type { SinkExecutionTarget } from "./http";

/* -----------------------------------------------------------------------------
   Lightweight cron humanizer (no deps)
----------------------------------------------------------------------------- */
const DOW = [
	"Sunday",
	"Monday",
	"Tuesday",
	"Wednesday",
	"Thursday",
	"Friday",
	"Saturday",
];
const MON = [
	"January",
	"February",
	"March",
	"April",
	"May",
	"June",
	"July",
	"August",
	"September",
	"October",
	"November",
	"December",
];
const pad2 = (n: number) => String(n).padStart(2, "0");
const ord = (n: number) => {
	const s = ["th", "st", "nd", "rd"],
		v = n % 100;
	return `${n}${s[(v - 20) % 10] || s[v] || s[0]}`;
};
const exact = (s: string) => (/^\d+$/.test(s) ? Number.parseInt(s, 10) : null);
const step = (s: string) => {
	const m = s.match(/^\*\/(\d+)$/);
	return m ? Number.parseInt(m[1], 10) : null;
};
const list = (s: string) =>
	s.includes(",")
		? s
				.split(",")
				.map((x) => Number.parseInt(x, 10))
				.filter(Number.isFinite)
		: null;
const range = (s: string) => {
	const m = s.match(/^(\d+)-(\d+)$/);
	if (!m) return null;
	const a = +m[1],
		b = +m[2];
	const out: number[] = [];
	for (let i = a; i <= b; i++) out.push(i);
	return out;
};
const joinList = (items: string[]) =>
	items.length <= 1
		? items[0] || ""
		: i18next.t("valAndVal2", "{{val}} and {{val2}}", {
				val: items.slice(0, -1).join(", "),
				val2: items[items.length - 1],
			});
const dowName = (n: number) => DOW[(n === 7 ? 0 : n) % 7] ?? String(n);
const monName = (n: number) => MON[(n - 1) % 12] ?? String(n);
const domToText = (s: string) => {
	const a = list(s) ?? range(s);
	if (a) return joinList(a.map(ord));
	const n = exact(s);
	return n !== null ? ord(n) : null;
};
const monToText = (s: string) => {
	const a = list(s) ?? range(s);
	if (a) return joinList(a.map(monName));
	const n = exact(s);
	return n !== null ? monName(n) : null;
};
const dowToText = (s: string) => {
	if (s === "1-5") return "weekdays";
	if (s === "0,6" || s === "6,0") return "weekends";
	const a = list(s) ?? range(s);
	if (a) return joinList(a.map(dowName));
	const n = exact(s);
	return n !== null ? dowName(n) : null;
};

function humanizeCron(expr: string, opts?: { tz?: string }) {
	const tz = opts?.tz ? ` (${opts.tz})` : "";
	if (!expr?.trim()) return "";
	const parts = expr.trim().split(/\s+/);

	// Support both 5-field and 6-field (with seconds) cron
	const [sec, min, hour, dom, mon, dow] =
		parts.length === 6 ? parts : ["0", ...parts];

	const sExact = exact(sec || "*");
	const mExact = exact(min || "*");
	const hExact = exact(hour || "*");
	const mStep = step(min || "*");
	const hStep = step(hour || "*");
	const sStep = step(sec || "*");

	// Fast paths with seconds support
	if (
		sec === "*" &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everySecondtz", "Every second{{tz}}", { tz });
	if (
		sStep &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everySstepSecondstz", "Every {{sStep}} seconds{{tz}}", {
			sStep,
			tz,
		});
	if (
		sExact !== null &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"atValOfEveryMinutetz",
			"At :{{val}} of every minute{{tz}}",
			{ val: pad2(sExact), tz },
		);
	if (
		sec === "0" &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everyMinutetz", "Every minute{{tz}}", { tz });
	if (
		sec === "0" &&
		mStep &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everyMstepMinutestz", "Every {{mStep}} minutes{{tz}}", {
			mStep,
			tz,
		});
	if (
		sec === "0" &&
		mExact !== null &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"atValPastEveryHourtz",
			"At :{{val}} past every hour{{tz}}",
			{ val: pad2(mExact), tz },
		);
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"atValval2EveryDaytz",
			"At {{val}}:{{val2}} every day{{tz}}",
			{ val: pad2(hExact), val2: pad2(mExact), tz },
		);
	if (
		sec === "0" &&
		mExact !== null &&
		hStep &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"everyHstepHoursAtValtz",
			"Every {{hStep}} hours at :{{val}}{{tz}}",
			{ hStep, val: pad2(mExact), tz },
		);

	// With specific seconds
	if (
		sExact !== null &&
		mExact !== null &&
		hExact !== null &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	) {
		return i18next.t(
			"atValval2val3EveryDaytz",
			"At {{val}}:{{val2}}:{{val3}} every day{{tz}}",
			{ val: pad2(hExact), val2: pad2(mExact), val3: pad2(sExact), tz },
		);
	}

	// DOW / monthly / monthly+day
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		dom === "*" &&
		mon === "*" &&
		dow !== "*"
	) {
		const when = dowToText(dow);
		if (when)
			return i18next.t(
				"atValval2OnWhentz",
				"At {{val}}:{{val2}} on {{when}}{{tz}}",
				{ val: pad2(hExact), val2: pad2(mExact), when, tz },
			);
	}
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		dom !== "*" &&
		mon === "*" &&
		dow === "*"
	) {
		const days = domToText(dom);
		if (days)
			return i18next.t(
				"atValval2OnTheDaysOfEveryMonthtz",
				"At {{val}}:{{val2}} on the {{days}} of every month{{tz}}",
				{ val: pad2(hExact), val2: pad2(mExact), days, tz },
			);
	}
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		mon !== "*" &&
		dow === "*"
	) {
		const months = monToText(mon);
		if (months) {
			if (dom === "*")
				return i18next.t(
					"atValval2InMonthstz",
					"At {{val}}:{{val2}} in {{months}}{{tz}}",
					{ val: pad2(hExact), val2: pad2(mExact), months, tz },
				);
			const days = domToText(dom);
			if (days)
				return i18next.t(
					"atValval2OnTheDaysOfMonthstz",
					"At {{val}}:{{val2}} on the {{days}} of {{months}}{{tz}}",
					{ val: pad2(hExact), val2: pad2(mExact), days, months, tz },
				);
		}
	}
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		mon !== "*" &&
		dow !== "*"
	) {
		const months = monToText(mon);
		const when = dowToText(dow);
		if (months && when)
			return i18next.t(
				"atValval2OnWhenInMonthstz",
				"At {{val}}:{{val2}} on {{when}} in {{months}}{{tz}}",
				{ val: pad2(hExact), val2: pad2(mExact), when, months, tz },
			);
	}

	return i18next.t("cronExprtz", "Cron: {{expr}}{{tz}}", { expr, tz });
}

/* -----------------------------------------------------------------------------
   Types (serde-friendly)
----------------------------------------------------------------------------- */
export type ScheduledLocal = {
	date: string; // "YYYY-MM-DD"
	time: string; // "HH:mm"
};

export type CronSink = {
	expression?: string | null; // "0 9 * * *"
	scheduled_for?: ScheduledLocal | null; // local date+time, runtime uses timezone to compute UTC
	last_fired?: string | null; // RFC3339 (from runtime)
	timezone?: string | null; // IANA e.g. "Europe/Berlin"
	sink_execution?: SinkExecutionTarget;
};

type Mode = "one_time" | "recurring";

const QUICK_CRON_PRESETS = [
	{ label: "Every 30 seconds", value: "*/30 * * * * *" },
	{ label: "Every minute", value: "0 * * * * *" },
	{ label: "Every 5 minutes", value: "0 */5 * * * *" },
	{ label: "Hourly (:00)", value: "0 0 * * * *" },
	{ label: "Daily 09:00", value: "0 0 9 * * *" },
	{ label: "Weekdays 09:00", value: "0 0 9 * * 1-5" },
	{ label: "Sundays 18:00", value: "0 0 18 * * 0" },
];

/* -----------------------------------------------------------------------------
   Small Intl helpers (no date libs)
----------------------------------------------------------------------------- */
function parseHHMM(s: string): { h: number; m: number } {
	const [h, m] = (s || "09:00").split(":").map((x) => Number.parseInt(x, 10));
	return { h: Number.isFinite(h) ? h : 9, m: Number.isFinite(m) ? m : 0 };
}
function nowInTZ(tz: string) {
	const d = new Date();
	const parts = new Intl.DateTimeFormat("en-CA", {
		timeZone: tz,
		year: "numeric",
		month: "2-digit",
		day: "2-digit",
		hour: "2-digit",
		minute: "2-digit",
		hour12: false,
	}).formatToParts(d);
	const get = (t: Intl.DateTimeFormatPartTypes) =>
		Number(parts.find((p) => p.type === t)?.value);
	return {
		y: get("year"),
		mo: get("month"),
		d: get("day"),
		h: get("hour"),
		mi: get("minute"),
	};
}
function isPastInTZ(sel: ScheduledLocal | null | undefined, tz: string) {
	if (!sel?.date || !sel?.time) return null;
	const [y, mo, d] = sel.date.split("-").map((n) => Number.parseInt(n, 10));
	const { h, m } = parseHHMM(sel.time);
	const now = nowInTZ(tz);
	const a = [y, mo, d, h, m],
		b = [now.y, now.mo, now.d, now.h, now.mi];
	for (let i = 0; i < a.length; i++) {
		if (a[i] < b[i]) return true;
		if (a[i] > b[i]) return false;
	}
	return false;
}
function formatLocalDisplay(
	sel: ScheduledLocal | null | undefined,
	tz: string,
) {
	if (!sel?.date || !sel?.time) return "—";
	const [y, mo, d] = sel.date.split("-").map((n) => Number.parseInt(n, 10));
	const { h, m } = parseHHMM(sel.time);
	const approx = new Date(
		Date.UTC(y, (mo || 1) - 1, d || 1, h || 0, m || 0, 0),
	);
	return new Intl.DateTimeFormat("en-GB", {
		timeZone: tz,
		year: "numeric",
		month: "short",
		day: "2-digit",
		hour: "2-digit",
		minute: "2-digit",
		hour12: false,
		timeZoneName: "short",
	}).format(approx);
}

/* -----------------------------------------------------------------------------
   IANA timezone helpers
----------------------------------------------------------------------------- */
const IANA_TIMEZONES: string[] = (() => {
	try {
		return Intl.supportedValuesOf("timeZone");
	} catch {
		return [
			"UTC",
			"America/New_York",
			"America/Chicago",
			"America/Denver",
			"America/Los_Angeles",
			"Europe/London",
			"Europe/Berlin",
			"Europe/Paris",
			"Asia/Tokyo",
			"Asia/Shanghai",
			"Asia/Kolkata",
			"Australia/Sydney",
			"Pacific/Auckland",
		];
	}
})();

function isValidTimezone(tz: string): boolean {
	if (!tz) return false;
	try {
		Intl.DateTimeFormat(undefined, { timeZone: tz });
		return true;
	} catch {
		return false;
	}
}

function getUnsupportedRemoteCronSeconds(expr: string): string | null {
	const parts = expr.trim().split(/\s+/);
	if (parts.length !== 6) return null;
	const seconds = parts[0];
	return seconds === "0" || seconds === "*" ? null : seconds;
}

function formatRelativeTime(iso: string): string {
	try {
		const d = new Date(iso);
		if (Number.isNaN(d.getTime())) return iso;
		const diff = Date.now() - d.getTime();
		const absDiff = Math.abs(diff);
		const isFuture = diff < 0;
		const prefix = isFuture ? "in " : "";
		const suffix = isFuture ? "" : " ago";
		if (absDiff < 60_000) return "just now";
		if (absDiff < 3_600_000) {
			const m = Math.floor(absDiff / 60_000);
			return `${prefix}${m}m${suffix}`;
		}
		if (absDiff < 86_400_000) {
			const h = Math.floor(absDiff / 3_600_000);
			return `${prefix}${h}h${suffix}`;
		}
		const days = Math.floor(absDiff / 86_400_000);
		return `${prefix}${days}d${suffix}`;
	} catch {
		return iso;
	}
}

/* -----------------------------------------------------------------------------
   Component
----------------------------------------------------------------------------- */
export function CronJobConfig({
	isEditing,
	config,
	onConfigUpdate,
	hub,
	canExecuteLocally,
	section,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const browserTZ =
		typeof Intl !== "undefined"
			? Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC"
			: "UTC";

	const timezone = (config?.timezone as string) || browserTZ;

	const initialMode: Mode =
		(config?.expression && config.expression.trim().length > 0) ||
		(!config?.scheduled_for && !config?.expression)
			? "recurring"
			: "one_time";

	const [mode, setMode] = useState<Mode>(initialMode);

	const [tzOpen, setTzOpen] = useState(false);
	const [tzSearch, setTzSearch] = useState("");
	const tzValid = isValidTimezone(timezone);

	// One-time UI state
	const [dateValue, setDateValue] = useState<Date | undefined>(() => {
		if (config?.scheduled_for?.date) {
			const [y, mo, d] = config.scheduled_for.date
				.split("-")
				.map((n: string) => Number.parseInt(n, 10));
			return new Date(Date.UTC(y, (mo || 1) - 1, d || 1));
		}
		return undefined;
	});
	const [timeValue, setTimeValue] = useState<string>(
		config?.scheduled_for?.time || "09:00",
	);

	// Sync config from local UI
	useEffect(() => {
		if (mode !== "one_time") return;
		const dateStr = dateValue
			? new Intl.DateTimeFormat("en-CA", {
					timeZone: "UTC",
					year: "numeric",
					month: "2-digit",
					day: "2-digit",
				})
					.formatToParts(dateValue)
					.reduce((acc, p) => ({ ...acc, [p.type]: p.value }), {} as any)
			: null;

		const scheduled_for: ScheduledLocal | null = dateStr
			? {
					date: `${dateStr.year}-${dateStr.month}-${dateStr.day}`,
					time: timeValue || "09:00",
				}
			: null;

		if (
			JSON.stringify(config?.scheduled_for) !== JSON.stringify(scheduled_for)
		) {
			onConfigUpdate?.({
				...(config as any),
				scheduled_for,
				expression: null,
				timezone,
			});
		}
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [dateValue?.getTime?.(), timeValue, mode, timezone]);

	const expression = (config?.expression as string) || "";

	// Humanized cron (no deps)
	const cronHuman = useMemo(
		() => (expression.trim() ? humanizeCron(expression, { tz: timezone }) : ""),
		[expression, timezone],
	);

	// Next runs via cron-parser (supports 5 or 6 field expressions)
	const cronNextRuns = useMemo(() => {
		if (!expression.trim()) return [];
		try {
			const it = CronExpressionParser.parse(expression, { tz: timezone });
			const out: string[] = [];
			for (let i = 0; i < 5; i++) {
				const next = it.next();
				const iso = next.toISOString();
				if (iso) out.push(iso);
			}
			return out;
		} catch {
			return [];
		}
	}, [expression, timezone]);

	const isCronValid = useMemo(() => {
		if (!expression.trim()) return false;
		try {
			CronExpressionParser.parse(expression, { tz: timezone });
			return true;
		} catch {
			return false;
		}
	}, [expression, timezone]);

	const sinkExecution =
		(config?.sink_execution as SinkExecutionTarget) || undefined;

	const supportsRemote = hub?.domain != null;
	const supportsLocal = canExecuteLocally ?? false;
	const supportsBoth = supportsRemote && supportsLocal;

	const effectiveExecution: SinkExecutionTarget = useMemo(() => {
		if (sinkExecution) return sinkExecution;
		if (supportsBoth) return "HYBRID";
		if (supportsRemote) return "REMOTE";
		return "LOCAL";
	}, [sinkExecution, supportsBoth, supportsRemote]);
	const includesRemoteExecution = effectiveExecution !== "LOCAL";
	const remoteCronUnsupportedSeconds = useMemo(() => {
		if (!includesRemoteExecution || mode !== "recurring") return null;
		if (!expression.trim()) return null;
		return getUnsupportedRemoteCronSeconds(expression);
	}, [includesRemoteExecution, mode, expression]);

	const setValue = (k: keyof CronSink | string, v: any) =>
		onConfigUpdate?.({ ...(config as any), [k]: v });

	const handleModeChange = (m: Mode) => {
		setMode(m);
		if (m === "one_time") {
			setValue("expression", null);
			if (!config?.scheduled_for) {
				const today = new Date();
				const parts = new Intl.DateTimeFormat("en-CA", {
					timeZone: "UTC",
					year: "numeric",
					month: "2-digit",
					day: "2-digit",
				})
					.formatToParts(today)
					.reduce((acc, p) => ({ ...acc, [p.type]: p.value }), {} as any);
				setValue("scheduled_for", {
					date: `${parts.year}-${parts.month}-${parts.day}`,
					time: "09:00",
				});
			}
		} else {
			setValue("scheduled_for", null);
			if (!expression.trim()) setValue("expression", "0 0 9 * * *");
		}
	};

	const scheduledFor = config?.scheduled_for ?? null;
	const alreadyHappened = isPastInTZ(scheduledFor, timezone);

	// The events surface renders one section at a time; anywhere else (and for
	// any section this component doesn't know) it renders whole.
	const shows = (id: string) => !section || section === id;

	return (
		<div className="w-full space-y-6">
			{!section && (
				<div className="space-y-1">
					<h3 className="text-lg font-semibold">{t("cronJob", "Cron Job")}</h3>
					<p className="text-sm text-muted-foreground">
						{t(
							"onetimeScheduleOrRecurringCronStoredAsSimpleStringsYourRustRuntimeResolvesTimezoneAndUtcSafely",
							"One-time schedule or recurring cron. Stored as simple strings; your Rust runtime resolves timezone and UTC safely.",
						)}
					</p>
				</div>
			)}

			{/* Execution Target */}
			{shows("runtime") && supportsBoth && (
				<div className="space-y-2">
					<Label htmlFor="sink_execution">
						{t("executionTarget", "Execution Target")}
					</Label>
					<Select
						value={effectiveExecution}
						onValueChange={(value) => setValue("sink_execution", value)}
						disabled={!isEditing}
					>
						<SelectTrigger id="sink_execution" className="w-full">
							<SelectValue
								placeholder={t(
									"selectExecutionTarget",
									"Select execution target",
								)}
							/>
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="REMOTE">
								<div className="flex items-center gap-2">
									<Badge variant="default">{t("remote", "Remote")}</Badge>
									<span className="text-muted-foreground text-xs">
										{t(
											"serverOnlyAvailable247",
											"Server only — available 24/7",
										)}
									</span>
								</div>
							</SelectItem>
							<SelectItem value="LOCAL">
								<div className="flex items-center gap-2">
									<Badge variant="secondary">{t("local", "Local")}</Badge>
									<span className="text-muted-foreground text-xs">
										{t("desktopAppOnly", "Desktop app only")}
									</span>
								</div>
							</SelectItem>
							<SelectItem value="HYBRID">
								<div className="flex items-center gap-2">
									<Badge variant="outline">{t("both", "Both")}</Badge>
									<span className="text-muted-foreground text-xs">
										{t("serverDesktopApp", "Server + desktop app")}
									</span>
								</div>
							</SelectItem>
						</SelectContent>
					</Select>
					<p className="text-sm text-muted-foreground">
						{t(
							"whereThisCronJobShouldBeRegisteredAndExecuted",
							"Where this cron job should be registered and executed.",
						)}
					</p>
				</div>
			)}

			{shows("schedule") && (
				<>
					{/* Mode */}
					<div className="space-y-2">
						<Label>{t("schedulingMode", "Scheduling Mode")}</Label>
						<RadioGroup
							value={mode}
							onValueChange={handleModeChange}
							className="flex gap-6"
						>
							<div className="flex items-center space-x-2">
								<RadioGroupItem
									value="one_time"
									id="mode_one_time"
									disabled={!isEditing}
								/>
								<Label htmlFor="mode_one_time">
									{t("onetimeScheduled", "One-time (scheduled)")}
								</Label>
							</div>
							<div className="flex items-center space-x-2">
								<RadioGroupItem
									value="recurring"
									id="mode_recurring"
									disabled={!isEditing}
								/>
								<Label htmlFor="mode_recurring">
									{t("recurringCron", "Recurring (cron)")}
								</Label>
							</div>
						</RadioGroup>
					</div>

					{/* Timezone */}
					<div className="space-y-2">
						<Label htmlFor="cron_tz">{t("timezone", "Timezone")}</Label>
						<div className="flex gap-2">
							<Popover open={tzOpen} onOpenChange={setTzOpen}>
								<PopoverTrigger asChild>
									<Button
										type="button"
										variant="outline"
										role="combobox"
										aria-expanded={tzOpen}
										className={cn(
											"flex-1 justify-between",
											!tzValid && timezone && "border-destructive",
										)}
										disabled={!isEditing}
									>
										{timezone || t("selectTimezone", "Select timezone...")}
										<ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
									</Button>
								</PopoverTrigger>
								<PopoverContent className="w-75 p-0">
									<Command shouldFilter={false}>
										<CommandInput
											placeholder={t("searchTimezone", "Search timezone...")}
											value={tzSearch}
											onValueChange={setTzSearch}
										/>
										<CommandList>
											<CommandEmpty>
												{t("noTimezoneFound", "No timezone found.")}
											</CommandEmpty>
											<CommandGroup>
												{IANA_TIMEZONES.filter((tz) =>
													tz.toLowerCase().includes(tzSearch.toLowerCase()),
												)
													.slice(0, 50)
													.map((tz) => (
														<CommandItem
															key={tz}
															value={tz}
															onSelect={() => {
																setValue("timezone", tz);
																setTzOpen(false);
																setTzSearch("");
															}}
														>
															<CheckIcon
																className={cn(
																	"mr-2 h-4 w-4",
																	timezone === tz ? "opacity-100" : "opacity-0",
																)}
															/>
															{tz}
														</CommandItem>
													))}
											</CommandGroup>
										</CommandList>
									</Command>
								</PopoverContent>
							</Popover>
							<Button
								type="button"
								variant="secondary"
								onClick={() =>
									setValue(
										"timezone",
										Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
									)
								}
								disabled={!isEditing}
							>
								{t("useMyTimezone", "Use my timezone")}
							</Button>
						</div>
						{!tzValid && timezone && (
							<p className="text-sm text-destructive">
								{t(
									"unknownTimezonePleaseSelectAValidIanaTimezone",
									"Unknown timezone. Please select a valid IANA timezone.",
								)}
							</p>
						)}
					</div>

					<Separator />

					{/* ONE-TIME */}
					{mode === "one_time" && (
						<div className="space-y-4">
							<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
								<div className="space-y-2">
									<Label>{t("date", "Date")}</Label>
									<Popover>
										<PopoverTrigger asChild>
											<Button
												type="button"
												variant="outline"
												className="w-full justify-start"
												disabled={!isEditing}
											>
												{scheduledFor?.date ?? t("pickADate", "Pick a date")}
											</Button>
										</PopoverTrigger>
										<PopoverContent className="p-0">
											<Calendar
												mode="single"
												selected={dateValue}
												onSelect={(d: Date | undefined) => setDateValue(d)}
												initialFocus
											/>
										</PopoverContent>
									</Popover>
									<p className="text-sm text-muted-foreground">
										{t("selectTheCalendarDay", "Select the calendar day.")}
									</p>
								</div>

								<div className="space-y-2">
									<Label htmlFor="cron_time">{t("time", "Time")}</Label>
									<Input
										id="cron_time"
										type="time"
										value={timeValue}
										onChange={(e) => setTimeValue(e.target.value)}
										disabled={!isEditing}
									/>
									<p className="text-sm text-muted-foreground">
										{t("24hourFormatHhmm", "24-hour format (HH:MM).")}
									</p>
								</div>
							</div>

							{/* Preview + past/future */}
							<div className="space-y-2">
								<Label>{t("scheduledLocal", "Scheduled (local)")}</Label>
								<div className="flex items-center gap-2">
									<div className="flex-1 h-10 rounded-md border border-input bg-muted px-3 py-2 text-sm flex items-center">
										{formatLocalDisplay(scheduledFor, timezone)}
									</div>
									{alreadyHappened === true && (
										<Badge variant="destructive">
											{t("inThePast", "In the past")}
										</Badge>
									)}
									{alreadyHappened === false && (
										<Badge>{t("upcoming", "Upcoming")}</Badge>
									)}
								</div>
								{alreadyHappened === true && (
									<Alert variant="destructive" className="mt-2">
										<AlertTitle>
											{t("alreadyHappened", "Already happened")}
										</AlertTitle>
										<AlertDescription>
											<Trans
												i18nKey="thisTimeIsInThePastForTimezonePleasePickAFutureTime"
												values={{ timezone }}
												components={{ strong: <strong /> }}
											>
												This time is in the past for{" "}
												<strong>{"{{timezone}}"}</strong>. Please pick a future
												time.
											</Trans>
										</AlertDescription>
									</Alert>
								)}
							</div>
						</div>
					)}

					{/* RECURRING */}
					{mode === "recurring" && (
						<div className="space-y-4">
							<div className="space-y-2">
								<Label htmlFor="cron_expression">
									{t("cronExpression", "Cron Expression")}
								</Label>
								<Input
									id="cron_expression"
									value={expression}
									onChange={(e) => setValue("expression", e.target.value)}
									placeholder={`0 */5 * * * *`}
									disabled={!isEditing}
									aria-invalid={
										!isCronValid || remoteCronUnsupportedSeconds != null
									}
									className={cn(
										(!isCronValid || remoteCronUnsupportedSeconds != null) &&
											"border-destructive",
									)}
								/>
								<p className="text-sm text-muted-foreground">
									<Trans i18nKey="6fieldCronCodesecMinHourDomMonDowcodeOr5fieldCodeminHourDomMonDowcodeTimezoneApplied">
										6-field cron (<code>sec min hour dom mon dow</code>) or
										5-field (<code>min hour dom mon dow</code>). Timezone
										applied:
									</Trans>{" "}
									<strong>{timezone}</strong>.
								</p>
								{remoteCronUnsupportedSeconds && (
									<Alert variant="destructive">
										<AlertTitle>
											{t(
												"remoteCronIsMinuteprecisionOnly",
												"Remote cron is minute-precision only",
											)}
										</AlertTitle>
										<AlertDescription>
											{effectiveExecution === "HYBRID"
												? t(
														"hybridExecutionStillRegistersThisScheduleRemotely",
														"Hybrid execution still registers this schedule remotely.",
													)
												: t(
														"remoteExecutionUsesEventbridgeScheduler",
														"Remote execution uses EventBridge Scheduler.",
													)}
											{t(
												"use5fieldCronOrKeepTheSecondsFieldAt",
												"Use 5-field cron, or keep the seconds field at",
											)}{" "}
											<Trans i18nKey="strong0strongOrStrongstrongCurrentSecondsField">
												<strong>0</strong> or <strong>*</strong>. Current
												seconds field:
											</Trans>
											<strong>{remoteCronUnsupportedSeconds}</strong>
											{t(
												"switchToLocalExecutionIfYouNeedSubminuteSchedules",
												". Switch to local execution if you need sub-minute schedules.",
											)}
										</AlertDescription>
									</Alert>
								)}
							</div>

							<div className="space-y-2">
								<Label>{t("quickPresets", "Quick presets")}</Label>
								<div className="flex flex-wrap gap-2">
									{QUICK_CRON_PRESETS.map((p) => (
										<Button
											key={p.value}
											type="button"
											variant="secondary"
											size="sm"
											onClick={() => setValue("expression", p.value)}
											disabled={!isEditing}
										>
											{p.label}
										</Button>
									))}
								</div>
							</div>

							<GuidedCronBuilder
								disabled={!isEditing}
								value={expression}
								onChange={(expr) => setValue("expression", expr)}
							/>

							<div className="space-y-2">
								<Label>{t("readable", "Readable")}</Label>
								<div className="h-10 rounded-md border border-input bg-muted px-3 py-2 text-sm flex items-center">
									{isCronValid
										? cronHuman || "—"
										: t("invalidCronExpression", "Invalid cron expression")}
								</div>

								<div className="space-y-2">
									<Label>{t("nextRunsPreview", "Next runs (preview)")}</Label>
									{isCronValid ? (
										<ul className="text-sm text-muted-foreground list-disc pl-5">
											{cronNextRuns.map((iso, i) => {
												const d = new Date(iso);
												return (
													<li key={i}>
														{new Intl.DateTimeFormat("en-GB", {
															timeZone: timezone,
															year: "numeric",
															month: "short",
															day: "2-digit",
															hour: "2-digit",
															minute: "2-digit",
															hour12: false,
															timeZoneName: "short",
														}).format(d)}{" "}
														({iso})
													</li>
												);
											})}
										</ul>
									) : (
										<Alert variant="destructive">
											<AlertTitle>
												{t("invalidCron", "Invalid cron")}
											</AlertTitle>
											<AlertDescription>
												{t(
													"adjustTheExpressionToSeeUpcomingRuns",
													"Adjust the expression to see upcoming runs.",
												)}
											</AlertDescription>
										</Alert>
									)}
								</div>
							</div>
						</div>
					)}

					{/* Last fired */}
					{config?.last_fired && (
						<>
							<Separator />
							<div className="flex items-center gap-2 text-sm text-muted-foreground">
								<Clock className="h-4 w-4" />
								<span>
									{t("lastRan", "Last ran")}{" "}
									<strong>{formatRelativeTime(config.last_fired)}</strong>
									{` — `}
									{new Intl.DateTimeFormat("en-GB", {
										timeZone: timezone,
										year: "numeric",
										month: "short",
										day: "2-digit",
										hour: "2-digit",
										minute: "2-digit",
										second: "2-digit",
										hour12: false,
										timeZoneName: "short",
									}).format(new Date(config.last_fired))}
								</span>
							</div>
						</>
					)}
				</>
			)}
		</div>
	);
}

/* -----------------------------------------------------------------------------
   Guided cron builder (no deps)
----------------------------------------------------------------------------- */
function GuidedCronBuilder({
	value,
	onChange,
	disabled,
}: {
	value: string;
	onChange: (v: string) => void;
	disabled?: boolean;
}) {
	const { t } = useTranslation("interfaces");
	const parts = (value || "").trim().split(/\s+/);

	// Support both 5-field and 6-field cron
	const [sec, min, hour, dom, mon, dow] =
		parts.length === 6
			? parts
			: [
					"0",
					parts[0] || "*",
					parts[1] || "*",
					parts[2] || "*",
					parts[3] || "*",
					parts[4] || "*",
				];

	const setPart = (i: number, v: string) => {
		const next = [sec, min, hour, dom, mon, dow];
		next[i] = v;
		onChange(next.join(" "));
	};

	const fields: Array<[string, string, (v: string) => void, string[]]> = [
		["Second", sec, (v) => setPart(0, v), ["0", "*/10", "*/15", "*/30", "*"]],
		["Minute", min, (v) => setPart(1, v), ["0", "*/5", "*/10", "*/15", "*"]],
		["Hour", hour, (v) => setPart(2, v), ["*", "0", "8", "9", "12", "18"]],
		[
			t("dayOfMonth", "Day of Month"),
			dom,
			(v) => setPart(3, v),
			["*", "1", "15", "28"],
		],
		["Month", mon, (v) => setPart(4, v), ["*", "1", "4", "7", "10"]],
		[
			t("dayOfWeek", "Day of Week"),
			dow,
			(v) => setPart(5, v),
			["*", "1-5", "0", "6"],
		],
	];

	return (
		<div className="rounded-lg border p-4 space-y-3">
			<div className="flex items-center justify-between">
				<Label>{t("guidedBuilder", "Guided builder")}</Label>
				<Badge variant="secondary">
					{t("secMinHourDomMonDow", "sec min hour dom mon dow")}
				</Badge>
			</div>
			<div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-3">
				{fields.map(([label, val, cb, sugg], idx) => (
					<div className="space-y-2" key={idx}>
						<Label className="text-xs">{label}</Label>
						<Input
							value={val}
							onChange={(e) => cb(e.target.value)}
							disabled={disabled}
						/>
						<div className="flex flex-wrap gap-1">
							{sugg.map((s) => (
								<Button
									key={s}
									type="button"
									size="sm"
									variant="ghost"
									onClick={() => cb(s)}
									disabled={disabled}
									className="h-6 px-2 text-xs"
								>
									{s}
								</Button>
							))}
						</div>
					</div>
				))}
			</div>
			<p className="text-xs text-muted-foreground">
				<Trans i18nKey="useCodecodeForEveryRangesLikeCode15codeListsLike">
					Use <code>*</code> for “every”, ranges like <code>1-5</code>, lists
					like
				</Trans>{" "}
				<Trans i18nKey="code115codeAndStepsLikeCode10code">
					<code>1,15</code>, and steps like <code>*/10</code>.
				</Trans>
			</p>
		</div>
	);
}
