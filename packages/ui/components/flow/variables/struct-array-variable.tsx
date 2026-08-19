"use client";

import { useTranslation } from "@flow-like/locales";
import { PlusCircleIcon, XIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { Button, Separator } from "../../ui";
import { StructVariable, buildSchemaDefaults } from "./struct-variable";

/**
 * Editor for arrays (and sets) of structs. Renders one schema-driven
 * `StructVariable` form per item plus add/remove controls. Items are keyed by
 * a generation counter so removals remount the remaining forms with their
 * current values instead of showing a neighbour's stale state.
 */
export function StructArrayVariable({
	disabled,
	variable,
	onChange,
	refs,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
	refs?: Record<string, string>;
}>) {
	const { t } = useTranslation("flow");
	const [generation, setGeneration] = useState(0);

	const values = useMemo<unknown[]>(() => {
		const parsed = parseUint8ArrayToJson(variable.default_value);
		return Array.isArray(parsed) ? parsed : [];
	}, [variable.default_value]);

	const commit = useCallback(
		(updated: unknown[]) => {
			onChange({
				...variable,
				default_value: convertJsonToUint8Array(updated),
			});
		},
		[onChange, variable],
	);

	const handleAdd = useCallback(() => {
		if (disabled) return;
		commit([...values, buildSchemaDefaults(variable.schema, refs)]);
	}, [disabled, values, commit, variable.schema, refs]);

	const handleRemove = useCallback(
		(index: number) => {
			if (disabled) return;
			commit(values.filter((_, i) => i !== index));
			setGeneration((g) => g + 1);
		},
		[disabled, values, commit],
	);

	const handleItemChange = useCallback(
		(index: number, itemVariable: IVariable) => {
			const item = parseUint8ArrayToJson(itemVariable.default_value);
			const updated = [...values];
			updated[index] = item;
			commit(updated);
		},
		[values, commit],
	);

	return (
		<div className="flex flex-col gap-3 w-full min-w-0">
			{values.length === 0 && (
				<p className="text-xs text-muted-foreground">
					{t("noItemsYet", "No items yet.")}
				</p>
			)}
			{values.map((item, index) => (
				<div
					// biome-ignore lint/suspicious/noArrayIndexKey: generation counter remounts on structural changes
					key={`${generation}-${index}`}
					className="rounded-lg border p-3 flex flex-col gap-2 min-w-0"
				>
					<div className="flex items-center justify-between">
						<span className="text-xs font-medium text-muted-foreground">
							#{index + 1}
						</span>
						<Button
							variant="ghost"
							size="icon"
							className="h-6 w-6"
							disabled={disabled}
							onClick={() => handleRemove(index)}
							title={t("removeItem", "Remove item")}
						>
							<XIcon className="w-3.5 h-3.5" />
						</Button>
					</div>
					<StructVariable
						disabled={disabled}
						variable={{
							...variable,
							id: `${variable.id}-${index}`,
							default_value: convertJsonToUint8Array(item),
						}}
						onChange={(itemVariable) => handleItemChange(index, itemVariable)}
						refs={refs}
					/>
				</div>
			))}
			<Separator />
			<Button
				variant="outline"
				size="sm"
				disabled={disabled}
				onClick={handleAdd}
				className="self-start"
			>
				<PlusCircleIcon className="w-4 h-4 mr-1.5" />
				{t("addItem", "Add item")}
			</Button>
		</div>
	);
}
