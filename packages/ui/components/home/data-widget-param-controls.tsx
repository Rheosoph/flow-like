"use client";

import { Check, ChevronsUpDown, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import type {
	SqlParameterInfo,
	SqlParameterOptions,
} from "../../state/backend-state/query-state";
import { Button } from "../ui/button";
import {
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "../ui/command";
import { Input } from "../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { Switch } from "../ui/switch";
import { selectClass } from "./data-widget-settings-fields";
import {
	HOME_VIEWER_ID,
	optionValues,
	paramOptionKey,
	parseNumberInput,
	toDateInput,
	toDateTimeInput,
} from "./home-data-params";
import type { HomeParamControl } from "./home-data-query";

export type ParamOptionsLoader = (
	info: SqlParameterInfo,
	request?: { search?: string; range?: boolean },
) => Promise<SqlParameterOptions>;

export interface ParamControlProps {
	info: SqlParameterInfo;
	control: HomeParamControl;
	value: unknown;
	onChange: (value: unknown) => void;
	loadOptions: ParamOptionsLoader;
	label: string;
	eager?: boolean;
	className?: string;
}

const VIEWER_LABEL = "Current user’s ID";
const SEARCH_DELAY_MS = 250;

function optionLabel(value: unknown): string {
	if (value === HOME_VIEWER_ID) return VIEWER_LABEL;
	if (value === null || value === undefined) return "(empty)";
	return typeof value === "string" ? value : JSON.stringify(value);
}

function useParamOptions(
	info: SqlParameterInfo,
	loadOptions: ParamOptionsLoader,
) {
	const [state, setState] = useState<{
		values: unknown[];
		truncated: boolean;
		loading: boolean;
		loaded: boolean;
		failed: boolean;
	}>({
		values: [],
		truncated: false,
		loading: false,
		loaded: false,
		failed: false,
	});
	const request = useRef(0);
	const load = useCallback(
		(search?: string) => {
			const id = ++request.current;
			setState((current) => ({ ...current, loading: true, failed: false }));
			loadOptions(info, search ? { search } : {}).then(
				(options) => {
					if (id !== request.current) return;
					const { values, truncated } = optionValues(options);
					setState({
						values,
						truncated,
						loading: false,
						loaded: true,
						failed: false,
					});
				},
				() => {
					if (id !== request.current) return;
					setState((current) => ({
						...current,
						loading: false,
						loaded: true,
						failed: true,
					}));
				},
			);
		},
		[info, loadOptions],
	);
	return { ...state, load };
}

function ComboboxControl({
	info,
	value,
	onChange,
	loadOptions,
	label,
	multiple,
	className,
}: ParamControlProps & { multiple: boolean }) {
	const options = useParamOptions(info, loadOptions);
	const [open, setOpen] = useState(false);
	const [search, setSearch] = useState("");
	const { load } = options;
	useEffect(() => {
		if (!open) return;
		const timer = setTimeout(() => load(search), search ? SEARCH_DELAY_MS : 0);
		return () => clearTimeout(timer);
	}, [open, search, load]);
	const selected = useMemo(
		() => (multiple ? (Array.isArray(value) ? value : []) : [value]),
		[multiple, value],
	);
	const isSelected = (candidate: unknown) =>
		selected.some((item) => paramOptionKey(item) === paramOptionKey(candidate));
	const choices = useMemo(() => {
		const known = options.values.map((item) => item);
		const extra = selected.filter(
			(item) =>
				item !== undefined &&
				item !== "" &&
				!known.some((other) => paramOptionKey(other) === paramOptionKey(item)),
		);
		return [HOME_VIEWER_ID, ...extra, ...known].filter(
			(item, index, all) =>
				all.findIndex(
					(other) => paramOptionKey(other) === paramOptionKey(item),
				) === index,
		);
	}, [options.values, selected]);
	const toggle = (choice: unknown) => {
		if (!multiple) {
			onChange(choice);
			setOpen(false);
			return;
		}
		const next = isSelected(choice)
			? selected.filter(
					(item) => paramOptionKey(item) !== paramOptionKey(choice),
				)
			: [...selected, choice];
		onChange(next.length ? next : undefined);
	};
	const summary =
		selected.filter((item) => item !== undefined && item !== "").length === 0
			? "Any"
			: selected.map(optionLabel).join(", ");
	return (
		<Popover open={open} onOpenChange={setOpen} modal>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="outline"
					aria-label={label}
					aria-expanded={open}
					className={cn(
						"h-9 w-full min-w-0 justify-between px-2 text-sm font-normal",
						className,
					)}
				>
					<span className="truncate">{summary}</span>
					<ChevronsUpDown className="ml-1 size-3.5 shrink-0 opacity-50" />
				</Button>
			</PopoverTrigger>
			<PopoverContent className="w-64 p-0" align="start">
				<Command shouldFilter={!options.truncated}>
					<CommandInput
						placeholder="Search"
						value={search}
						onValueChange={setSearch}
					/>
					<CommandList>
						{options.loading && (
							<div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
								<Loader2 className="size-3 animate-spin" />
								Loading
							</div>
						)}
						{options.failed && (
							<p className="px-3 py-2 text-xs text-muted-foreground">
								Values could not be loaded.
							</p>
						)}
						<CommandEmpty>No matching values.</CommandEmpty>
						<CommandGroup>
							{choices.map((choice) => (
								<CommandItem
									key={paramOptionKey(choice)}
									value={optionLabel(choice)}
									onSelect={() => toggle(choice)}
								>
									<Check
										className={cn(
											"mr-2 size-3.5",
											isSelected(choice) ? "opacity-100" : "opacity-0",
										)}
									/>
									<span className="truncate">{optionLabel(choice)}</span>
								</CommandItem>
							))}
						</CommandGroup>
						{options.truncated && (
							<p className="border-t px-3 py-1.5 text-[11px] text-muted-foreground">
								Type to narrow the list.
							</p>
						)}
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}

function SelectControl(props: ParamControlProps) {
	const { info, value, onChange, loadOptions, label, eager, className } = props;
	const options = useParamOptions(info, loadOptions);
	const { load, loaded, loading } = options;
	const requested = useRef(false);
	const ensure = useCallback(() => {
		if (requested.current) return;
		requested.current = true;
		load();
	}, [load]);
	useEffect(() => {
		if (eager) ensure();
	}, [eager, ensure]);
	if (options.truncated) return <ComboboxControl {...props} multiple={false} />;
	const choices = [HOME_VIEWER_ID, ...options.values];
	if (
		value !== undefined &&
		value !== "" &&
		!choices.some((item) => paramOptionKey(item) === paramOptionKey(value))
	)
		choices.push(value);
	return (
		<select
			aria-label={label}
			className={cn(selectClass, className)}
			value={value === undefined ? "" : paramOptionKey(value)}
			aria-busy={loading && !loaded}
			onFocus={ensure}
			onPointerDown={ensure}
			onChange={(event) => {
				const next = choices.find(
					(item) => paramOptionKey(item) === event.target.value,
				);
				onChange(next);
			}}
		>
			<option value="">Any</option>
			{choices.map((choice) => (
				<option key={paramOptionKey(choice)} value={paramOptionKey(choice)}>
					{optionLabel(choice)}
				</option>
			))}
		</select>
	);
}

export function ParamControl(props: ParamControlProps) {
	const { control, value, onChange, label, className } = props;
	switch (control) {
		case "toggle":
			return (
				<Switch
					aria-label={label}
					checked={value === true}
					onCheckedChange={onChange}
				/>
			);
		case "number":
			return (
				<Input
					type="number"
					aria-label={label}
					className={cn("h-9", className)}
					value={typeof value === "number" ? String(value) : ""}
					onChange={(event) => onChange(parseNumberInput(event.target.value))}
				/>
			);
		case "date":
			return (
				<Input
					type="date"
					aria-label={label}
					className={cn("h-9", className)}
					value={toDateInput(value)}
					onChange={(event) => onChange(event.target.value || undefined)}
				/>
			);
		case "datetime":
			return (
				<Input
					type="datetime-local"
					aria-label={label}
					className={cn("h-9", className)}
					value={toDateTimeInput(value)}
					onChange={(event) => onChange(event.target.value || undefined)}
				/>
			);
		case "select":
			return <SelectControl {...props} />;
		case "combobox":
			return <ComboboxControl {...props} multiple={false} />;
		case "multi-select":
			return <ComboboxControl {...props} multiple />;
		default:
			return (
				<Input
					aria-label={label}
					className={cn("h-9", className)}
					value={value === undefined ? "" : String(value)}
					onChange={(event) => onChange(event.target.value || undefined)}
				/>
			);
	}
}
