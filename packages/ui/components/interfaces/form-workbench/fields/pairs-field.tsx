"use client";

import { useTranslation } from "@flow-like/locales";
import { Plus } from "lucide-react";
import { type KeyboardEvent, useLayoutEffect, useRef } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { PairRow } from "../contracts";
import { valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import { lineControl } from "./control-style";
import { handleEnter } from "./enter";
import { RemoveButton } from "./file-row";
import { isComposing } from "./keys";

let rowSeq = 0;

/** New rows get ids of their own; rows read back from storage are `saved-<n>`. */
export const newRowId = () => `row-${++rowSeq}`;

function rowsOf(value: unknown): readonly PairRow[] {
	return Array.isArray(value) ? (value as readonly PairRow[]) : [];
}

interface RowProps {
	readonly props: ControlProps;
	readonly row: PairRow;
	readonly index: number;
	readonly onEdit: (next: PairRow) => void;
	readonly onRemove: () => void;
}

/** A name and a value: ↵ in the name goes to the value, ↵ in the value moves on. */
function Row({ props, row, index, onEdit, onRemove }: Readonly<RowProps>) {
	const { t } = useTranslation("interfaces");
	const { field, actions, bind, disabled } = props;
	const n = index + 1;
	const value = useRef<HTMLInputElement>(null);
	const first = index === 0;
	const style = lineControl({ touch: bind.touch });
	const onNameKey = (event: KeyboardEvent<HTMLInputElement>) => {
		if (isComposing(event) || event.key !== "Enter") return;
		event.preventDefault();
		value.current?.focus();
	};
	return (
		<div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-1.5">
			<input
				data-pair-name=""
				id={first ? bind.ids.control : undefined}
				type="text"
				autoComplete="off"
				value={row.key}
				disabled={disabled}
				placeholder={t("workbench.field.pairs.name", "Name")}
				aria-label={t("workbench.field.pairs.nameAria", "Name of row {{n}}", {
					n,
				})}
				{...(first ? bind.focus : {})}
				onChange={(event) => onEdit({ ...row, key: event.target.value })}
				onKeyDown={onNameKey}
				onBlur={() => actions.blurField(field.key)}
				className={style}
			/>
			<input
				ref={value}
				type="text"
				autoComplete="off"
				value={row.value}
				disabled={disabled}
				placeholder={t("workbench.field.pairs.value", "Value")}
				aria-label={t("workbench.field.pairs.valueAria", "Value of row {{n}}", {
					n,
				})}
				onChange={(event) => onEdit({ ...row, value: event.target.value })}
				onKeyDown={(event) => handleEnter(event, field.key, actions)}
				onBlur={() => actions.blurField(field.key)}
				className={style}
			/>
			<RemoveButton
				label={t("workbench.field.pairs.removeRow", "Remove row {{n}}", { n })}
				touch={bind.touch}
				onRemove={onRemove}
			/>
		</div>
	);
}

/** The cursor goes to the name of a row that was just added. */
function useFocusAdded(count: number) {
	const box = useRef<HTMLFieldSetElement>(null);
	const added = useRef(false);
	useLayoutEffect(() => {
		if (!added.current || count === 0) return;
		added.current = false;
		const names = box.current?.querySelectorAll<HTMLInputElement>(
			"input[data-pair-name]",
		);
		names?.[names.length - 1]?.focus();
	}, [count]);
	return {
		box,
		markAdded() {
			added.current = true;
		},
	};
}

/**
 * Name and value rows with "Add row" (SURFACE §4): a map of scalars, or a free object without a usable schema.
 * Values are typed as the map's data type says; a row without a name is a problem at press.
 */
export function PairsField(props: ControlProps) {
	const { t } = useTranslation("interfaces");
	const { field, rail, actions, bind, disabled } = props;
	const rows = rowsOf(valueAt(rail.values, field.key));
	const { box, markAdded } = useFocusAdded(rows.length);
	const set = (next: readonly PairRow[]) => actions.setValue(field.key, next);
	return (
		<fieldset
			ref={box}
			aria-labelledby={bind.ids.label}
			className="m-0 flex min-w-0 flex-col gap-1.5 border-0 p-0"
		>
			{rows.map((row, index) => (
				<Row
					key={row.id}
					props={props}
					row={row}
					index={index}
					onEdit={(next) =>
						set(rows.map((item) => (item.id === row.id ? next : item)))
					}
					onRemove={() => set(rows.filter((item) => item.id !== row.id))}
				/>
			))}
			<button
				type="button"
				disabled={disabled}
				{...(rows.length === 0 ? bind.focus : {})}
				onClick={() => {
					markAdded();
					set([...rows, { id: newRowId(), key: "", value: "" }]);
				}}
				className={cx(
					"inline-flex w-fit items-center gap-1.5 rounded-lg border border-border bg-card px-2.5 text-[12.5px]/4.5 font-medium text-foreground hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring",
					bind.touch ? "min-h-11" : "h-7",
				)}
			>
				<Plus aria-hidden className="size-3.5" />
				{t("workbench.field.pairs.addRow", "Add row")}
			</button>
		</fieldset>
	);
}
