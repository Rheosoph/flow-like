"use client";

import { RotateCcw } from "lucide-react";
import { useMemo } from "react";
import type { SqlParameterInfo } from "../../state/backend-state/query-state";
import { Button } from "../ui/button";
import {
	ParamControl,
	type ParamOptionsLoader,
} from "./data-widget-param-controls";
import {
	effectiveParamControl,
	groupParamEntries,
	interactiveParamNames,
	paramLabel,
} from "./home-data-params";
import type { HomeDataConfig } from "./home-data-query";
import {
	useHomeParamOptionsLoader,
	useHomeSqlDescription,
} from "./use-home-sql-description";

function fallbackInfo(name: string, current: unknown): SqlParameterInfo {
	const type =
		typeof current === "number"
			? "Float64"
			: typeof current === "boolean"
				? "Boolean"
				: null;
	return { name, data_type: type, bindings: [] };
}

function FilterField({
	label,
	children,
}: { label: string; children: React.ReactNode }) {
	return (
		<div className="flex min-w-28 max-w-48 flex-1 flex-col gap-0.5">
			<span className="truncate text-[10px] font-medium text-muted-foreground">
				{label}
			</span>
			{children}
		</div>
	);
}

/** Controls for the parameters an author marked as changeable by viewers. */
export function HomeDataFilterBar({
	config,
	overrides,
	onChange,
	onReset,
}: {
	config: HomeDataConfig;
	overrides: Record<string, unknown> | undefined;
	onChange: (name: string, value: unknown) => void;
	onReset: () => void;
}) {
	const names = useMemo(() => interactiveParamNames(config), [config]);
	const { description } = useHomeSqlDescription(config, {
		enabled: names.length > 0,
	});
	const loadOptions: ParamOptionsLoader = useHomeParamOptionsLoader(config);
	const entries = useMemo(() => {
		const described = description?.parameters ?? [];
		const infos = names.map(
			(name) =>
				described.find((info) => info.name === name) ??
				fallbackInfo(name, config.queryParams[name]),
		);
		return groupParamEntries(infos);
	}, [description, names, config.queryParams]);
	if (!names.length) return null;
	const value = (name: string) => overrides?.[name] ?? config.queryParams[name];
	const changed = names.some((name) => overrides?.[name] !== undefined);
	const render = (info: SqlParameterInfo, label?: string) => {
		const override = config.paramControls[info.name];
		const text = label ?? paramLabel(info.name, config.paramControls);
		return (
			<ParamControl
				info={info}
				control={effectiveParamControl(info, override)}
				value={value(info.name)}
				onChange={(next) => onChange(info.name, next)}
				loadOptions={loadOptions}
				label={text}
				className="h-8 text-xs"
			/>
		);
	};
	return (
		<div
			className="flex shrink-0 flex-wrap items-end gap-2"
			data-home-data-filters=""
		>
			{entries.map((entry) =>
				entry.type === "range" ? (
					<FilterField
						key={`${entry.from.name}:${entry.to.name}`}
						label={`${paramLabel(entry.from.name, config.paramControls)} – ${paramLabel(entry.to.name, config.paramControls)}`}
					>
						<div className="flex items-center gap-1">
							{render(entry.from)}
							{render(entry.to)}
						</div>
					</FilterField>
				) : (
					<FilterField
						key={entry.info.name}
						label={paramLabel(entry.info.name, config.paramControls)}
					>
						{render(entry.info)}
					</FilterField>
				),
			)}
			{changed && (
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="h-8 px-2 text-xs"
					onClick={onReset}
				>
					<RotateCcw className="mr-1 size-3" />
					Reset
				</Button>
			)}
		</div>
	);
}
