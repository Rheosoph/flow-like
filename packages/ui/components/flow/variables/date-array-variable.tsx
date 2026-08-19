import { useTranslation } from "@flow-like/locales";
import { format } from "date-fns";
import { CalendarIcon, PlusCircleIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { cn } from "../../..";
import { Calendar } from "../../../components/ui/calendar";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import {
	Badge,
	Button,
	Input,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Separator,
} from "../../ui";

export function DateArrayVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (v: IVariable) => void;
}>) {
	const { t } = useTranslation("flow");
	const parsedTimes = useMemo<Date[]>(() => {
		const p = parseUint8ArrayToJson(variable.default_value);
		if (!Array.isArray(p)) return [];

		return p.map((item) => {
			if (typeof item === "string") {
				return new Date(item);
			}
			// Fallback for old SystemTime format
			if (typeof item === "object" && "secs_since_epoch" in item) {
				return new Date(
					item.secs_since_epoch * 1000 +
						(item.nanos_since_epoch || 0) / 1_000_000,
				);
			}
			return new Date();
		});
	}, [variable.default_value]);

	// state for the "new" entry
	const [newDate, setNewDate] = useState<Date>(new Date());
	const [newTime, setNewTime] = useState<string>(format(newDate, "HH:mm"));

	// keep time input in sync if calendar changes
	useEffect(() => {
		setNewTime(format(newDate, "HH:mm"));
	}, [newDate]);

	const handleAdd = useCallback(() => {
		if (disabled || !newDate || !newTime) return;
		const [hrs, mins] = newTime.split(":").map((n) => Number.parseInt(n, 10));
		const dt = new Date(
			newDate.getFullYear(),
			newDate.getMonth(),
			newDate.getDate(),
			hrs,
			mins,
		);
		const updated = [...parsedTimes, dt.toISOString()];
		onChange({
			...variable,
			default_value: convertJsonToUint8Array(updated),
		});
	}, [disabled, newDate, newTime, parsedTimes, variable, onChange]);

	const handleRemove = useCallback(
		(idx: number) => {
			if (disabled) return;
			const p = parseUint8ArrayToJson(variable.default_value);
			const arr = Array.isArray(p) ? p : [];
			const updated = arr.filter((_, i) => i !== idx);
			onChange({
				...variable,
				default_value: convertJsonToUint8Array(updated),
			});
		},
		[disabled, variable, onChange],
	);

	return (
		<div className="flex flex-col gap-3 w-full min-w-0">
			<div className="flex items-center gap-2">
				<Popover>
					<PopoverTrigger disabled={disabled} asChild>
						<Button
							disabled={disabled}
							variant="outline"
							className={cn(
								"flex-1 justify-start text-left font-normal min-w-0",
								!newDate && "text-muted-foreground",
							)}
						>
							<CalendarIcon className="mr-2 h-4 w-4 shrink-0" />
							<span className="truncate">
								{newDate ? (
									`${format(newDate, "PPP")} - ${newTime}`
								) : (
									<span>{t("pickADate", "Pick a date")}</span>
								)}
							</span>
						</Button>
					</PopoverTrigger>
					<PopoverContent className="w-auto p-2">
						<div className="flex flex-col items-center gap-2">
							<div className="flex items-center gap-2 w-full">
								<p className="text-nowrap text-sm font-medium">
									{t("time", "Time:")}
								</p>
								<Input
									disabled={disabled}
									type="time"
									value={newTime}
									onChange={(e) => setNewTime(e.target.value)}
								/>
							</div>
							<Calendar
								disabled={disabled}
								showOutsideDays
								ISOWeek
								captionLayout="dropdown"
								mode="single"
								selected={newDate}
								onSelect={(date) => {
									setNewDate(date ?? new Date());
								}}
								className="w-full"
							/>
						</div>
					</PopoverContent>
				</Popover>
				<Button
					disabled={disabled}
					size="icon"
					variant="default"
					onClick={handleAdd}
					className="shrink-0"
				>
					<PlusCircleIcon className="w-4 h-4" />
				</Button>
			</div>

			{parsedTimes.length > 0 && (
				<>
					<Separator />
					<div className="flex flex-col gap-2 rounded-md border p-3">
						{parsedTimes.map((dt, idx) => (
							<Badge
								key={`${dt.toString()}-${idx}`}
								variant="secondary"
								className="group inline-flex items-center gap-1.5 pr-1 w-full"
							>
								<span className="break-all text-xs flex-1 min-w-0">
									{format(dt, "PPP")} – {format(dt, "HH:mm")}
								</span>
								<Button
									size="icon"
									variant="ghost"
									onClick={() => handleRemove(idx)}
									disabled={disabled}
									className="h-4 w-4 shrink-0 rounded-sm hover:bg-destructive hover:text-destructive-foreground"
								>
									<XIcon className="h-3 w-3" />
								</Button>
							</Badge>
						))}
					</div>
				</>
			)}
		</div>
	);
}
