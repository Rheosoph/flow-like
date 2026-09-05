"use client";

import { ChartNoAxesColumn, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { HOME_DATA_COLORS } from "./home-data-presentation";

export function HomeDataMessage({
	title,
	children,
	icon: Icon = ChartNoAxesColumn,
	action,
}: {
	title: string;
	children?: ReactNode;
	icon?: LucideIcon;
	action?: ReactNode;
}) {
	return (
		<div className="flex min-h-0 items-center gap-3 py-3">
			<div className="flex size-9 shrink-0 items-center justify-center rounded-xl border border-border/60 bg-muted/40 text-muted-foreground">
				<Icon className="size-4" aria-hidden="true" />
			</div>
			<div className="min-w-0 flex-1">
				<p className="text-sm font-medium leading-5">{title}</p>
				{children && (
					<div className="mt-1 text-xs leading-5 text-muted-foreground">
						{children}
					</div>
				)}
				{action && <div className="mt-2">{action}</div>}
			</div>
		</div>
	);
}

export function HomeDataLegend({
	items,
}: { items: { key: string; label: string; value?: string }[] }) {
	return (
		<ul
			className="flex max-h-16 shrink-0 flex-wrap gap-x-3 gap-y-1 overflow-y-auto pt-2 text-[11px] leading-4 text-muted-foreground"
			aria-label="Chart legend"
		>
			{items.map((item, index) => (
				<li
					className="flex min-w-0 max-w-full items-center gap-1.5"
					key={item.key}
					title={item.label}
				>
					<span
						className="size-2 shrink-0 rounded-sm"
						style={{
							background: HOME_DATA_COLORS[index % HOME_DATA_COLORS.length],
						}}
					/>
					<span className="max-w-40 truncate">{item.label}</span>
					{item.value && (
						<span className="font-medium tabular-nums text-foreground">
							{item.value}
						</span>
					)}
				</li>
			))}
		</ul>
	);
}

export function HomeDataCalendar({
	data,
	format,
}: {
	data: { day: string; value: number }[];
	format: (value: unknown) => string;
}) {
	const values = new Map(data.map((item) => [item.day, item.value]));
	const months = [...new Set(data.map((item) => item.day.slice(0, 7)))].sort();
	const minimum = Math.min(...data.map((item) => item.value));
	const maximum = Math.max(...data.map((item) => item.value));
	return (
		<div className="flex h-full min-h-0 flex-col gap-2" data-home-data-calendar>
			<div className="grid min-h-0 flex-1 content-start grid-cols-[repeat(auto-fit,minmax(min(100%,240px),1fr))] gap-4 overflow-auto">
				{months.map((month) => {
					const [year, number] = month.split("-").map(Number);
					const first = new Date(Date.UTC(year, number - 1, 1));
					const offset = (first.getUTCDay() + 6) % 7;
					const days = new Date(Date.UTC(year, number, 0)).getUTCDate();
					return (
						<table
							key={month}
							className="my-0 w-full table-fixed border-separate border-spacing-0.5 text-center text-[10px]"
						>
							<caption className="mb-1 text-left text-xs font-medium">
								{first.toLocaleDateString(undefined, {
									month: "long",
									year: "numeric",
									timeZone: "UTC",
								})}
							</caption>
							<thead>
								<tr>
									{["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map(
										(day) => (
											<th
												key={day}
												className="h-4 border-0 bg-transparent p-0 text-center font-normal text-muted-foreground"
											>
												{day.slice(0, 1)}
											</th>
										),
									)}
								</tr>
							</thead>
							<tbody>
								{Array.from(
									{ length: Math.ceil((days + offset) / 7) },
									(_, week) => week * 7 - offset + 1,
								).map((weekStart) => (
									<tr key={`${month}-week-${weekStart}`}>
										{[0, 1, 2, 3, 4, 5, 6].map((dayOffset) => {
											const day = weekStart + dayOffset;
											if (day < 1 || day > days)
												return (
													<td
														key={`${month}-empty-${day}`}
														className="border-0 p-0"
													/>
												);
											const date = `${month}-${String(day).padStart(2, "0")}`;
											const value = values.get(date);
											const strength =
												value === undefined
													? 0
													: maximum === minimum
														? 72
														: 24 +
															((value - minimum) / (maximum - minimum)) * 60;
											return (
												<td
													key={date}
													title={`${date}: ${value === undefined ? "No data" : format(value)}`}
													className="h-5 rounded border-0 p-0 leading-none tabular-nums"
													style={{
														background:
															value === undefined
																? "var(--muted)"
																: `color-mix(in srgb, var(--chart-1) ${strength}%, var(--muted))`,
														color:
															value === undefined
																? "var(--muted-foreground)"
																: "var(--foreground)",
													}}
												>
													{day}
													<span className="sr-only">
														: {value === undefined ? "No data" : format(value)}
													</span>
												</td>
											);
										})}
									</tr>
								))}
							</tbody>
						</table>
					);
				})}
			</div>
			<div className="flex shrink-0 items-center justify-end gap-2 text-[10px] tabular-nums text-muted-foreground">
				<span>{format(minimum)}</span>
				<span
					className="h-1.5 w-16 rounded-full"
					style={{
						background:
							"linear-gradient(to right, color-mix(in srgb, var(--chart-1) 24%, var(--muted)), color-mix(in srgb, var(--chart-1) 84%, var(--muted)))",
					}}
				/>
				<span>{format(maximum)}</span>
			</div>
		</div>
	);
}
