"use client";

import { useTranslation } from "@flow-like/locales";
import { FileIcon, KeyIcon, PlusCircleIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IVariable } from "../../../lib/schema/flow/variable";
import { IVariableType } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { useBackend } from "../../../state/backend-state";
import { Button, Input, Label, Separator, Switch, Textarea } from "../../ui";

type MapEntries = Record<string, unknown>;

const isRecord = (value: unknown): value is MapEntries =>
	typeof value === "object" && value !== null && !Array.isArray(value);

function initialValueForType(dataType: IVariableType): {
	value: unknown;
	valid: boolean;
} {
	switch (dataType) {
		case IVariableType.Boolean:
			return { value: false, valid: true };
		case IVariableType.Integer:
		case IVariableType.Byte:
		case IVariableType.Float:
			return { value: 0, valid: true };
		case IVariableType.Date:
		case IVariableType.PathBuf:
			return { value: "", valid: false };
		case IVariableType.Struct:
			return { value: {}, valid: true };
		case IVariableType.Generic:
			return { value: null, valid: true };
		default:
			return { value: "", valid: true };
	}
}

const pad = (n: number) => String(n).padStart(2, "0");

const isoToLocalInput = (iso: unknown): string => {
	if (typeof iso !== "string" || iso === "") return "";
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return "";
	return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
};

const localInputToIso = (local: string): string => {
	if (!local) return "";
	const date = new Date(local);
	if (Number.isNaN(date.getTime())) return "";
	return date.toISOString();
};

function renderValueDisplay(dataType: IVariableType, value: unknown): string {
	switch (dataType) {
		case IVariableType.Boolean:
			return value ? "True" : "False";
		case IVariableType.Date: {
			if (typeof value !== "string") return String(value);
			const date = new Date(value);
			return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
		}
		case IVariableType.PathBuf:
			return typeof value === "string"
				? (value.split("/").pop() ?? value)
				: String(value);
		case IVariableType.Struct:
		case IVariableType.Generic:
			return JSON.stringify(value);
		default:
			return String(value);
	}
}

function PathValueInput({
	value,
	onChange,
	disabled,
}: Readonly<{
	value: unknown;
	onChange: (value: unknown, valid: boolean) => void;
	disabled?: boolean;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const path = typeof value === "string" ? value : "";

	const handlePick = useCallback(async () => {
		if (disabled) return;
		const selection = await backend.helperState.openFileOrFolderMenu(
			false,
			false,
			true,
		);
		const pathBuf = Array.isArray(selection) ? selection[0] : selection;
		if (!pathBuf) return;
		const meta = await backend.helperState.getPathMeta(pathBuf);
		if (!meta || meta.length === 0) return;
		onChange(meta[0].location, true);
	}, [backend, disabled, onChange]);

	return (
		<Button
			variant="outline"
			className="flex-1 justify-start text-left font-normal min-w-0"
			onClick={handlePick}
			disabled={disabled}
		>
			<FileIcon className="mr-2 h-4 w-4 shrink-0" />
			<span className="truncate">
				{path ? (path.split("/").pop() ?? path) : t('chooseFile', 'Choose file...')}
			</span>
		</Button>
	);
}

function JsonValueInput({
	value,
	onChange,
	disabled,
	requireObject,
	placeholder,
}: Readonly<{
	value: unknown;
	onChange: (value: unknown, valid: boolean) => void;
	disabled?: boolean;
	requireObject?: boolean;
	placeholder?: string;
}>) {
	const [draft, setDraft] = useState(() =>
		value === undefined || value === null ? "" : JSON.stringify(value, null, 2),
	);
	const [error, setError] = useState<string | null>(null);

	const handleChange = useCallback(
		(next: string) => {
			setDraft(next);
			if (next.trim() === "") {
				if (requireObject) {
					setError("Required");
					onChange(undefined, false);
				} else {
					setError(null);
					onChange(null, true);
				}
				return;
			}
			try {
				const parsed = JSON.parse(next);
				if (requireObject && !isRecord(parsed)) {
					setError("Must be an object");
					onChange(parsed, false);
					return;
				}
				setError(null);
				onChange(parsed, true);
			} catch {
				setError("Invalid JSON");
				onChange(undefined, false);
			}
		},
		[onChange, requireObject],
	);

	return (
		<div className="flex flex-col gap-1 flex-1 min-w-0">
			<Textarea
				disabled={disabled}
				className="font-mono text-xs min-h-16"
				value={draft}
				onChange={(e) => handleChange(e.target.value)}
				placeholder={placeholder}
			/>
			{error && <p className="text-xs text-destructive">{error}</p>}
		</div>
	);
}

function MapValueInput({
	dataType,
	value,
	onChange,
	disabled,
	secret,
}: Readonly<{
	dataType: IVariableType;
	value: unknown;
	onChange: (value: unknown, valid: boolean) => void;
	disabled?: boolean;
	secret?: boolean;
}>) {
	const { t } = useTranslation("flow");
	switch (dataType) {
		case IVariableType.Boolean:
			return (
				<div className="flex items-center gap-2 flex-1">
					<Switch
						disabled={disabled}
						checked={Boolean(value)}
						onCheckedChange={(checked) => onChange(checked, true)}
					/>
					<Label className="text-sm">{value ? "True" : "False"}</Label>
				</div>
			);
		case IVariableType.Integer:
			return (
				<Input
					disabled={disabled}
					type={secret ? "password" : "number"}
					step={1}
					placeholder="Value..."
					className="flex-1 min-w-0"
					value={
						value === "" || value === undefined || value === null
							? ""
							: String(value)
					}
					onChange={(e) => {
						const raw = e.target.value;
						if (raw === "") return onChange("", false);
						const num = Number.parseInt(raw, 10);
						onChange(Number.isNaN(num) ? raw : num, !Number.isNaN(num));
					}}
				/>
			);
		case IVariableType.Byte:
			return (
				<Input
					disabled={disabled}
					type={secret ? "password" : "number"}
					step={1}
					min={0}
					max={255}
					placeholder={`0-255...`}
					className="flex-1 min-w-0"
					value={
						value === "" || value === undefined || value === null
							? ""
							: String(value)
					}
					onChange={(e) => {
						const raw = e.target.value;
						if (raw === "") return onChange("", false);
						const num = Number.parseInt(raw, 10);
						const valid = !Number.isNaN(num) && num >= 0 && num <= 255;
						onChange(Number.isNaN(num) ? raw : num, valid);
					}}
				/>
			);
		case IVariableType.Float:
			return (
				<Input
					disabled={disabled}
					type={secret ? "password" : "number"}
					step="any"
					placeholder="Value..."
					className="flex-1 min-w-0"
					value={
						value === "" || value === undefined || value === null
							? ""
							: String(value)
					}
					onChange={(e) => {
						const raw = e.target.value;
						if (raw === "") return onChange("", false);
						const num = Number.parseFloat(raw);
						onChange(Number.isNaN(num) ? raw : num, !Number.isNaN(num));
					}}
				/>
			);
		case IVariableType.Date:
			return (
				<Input
					disabled={disabled}
					type="datetime-local"
					className="flex-1 min-w-0"
					value={isoToLocalInput(value)}
					onChange={(e) => {
						const iso = localInputToIso(e.target.value);
						onChange(iso, iso !== "");
					}}
				/>
			);
		case IVariableType.PathBuf:
			return (
				<PathValueInput value={value} onChange={onChange} disabled={disabled} />
			);
		case IVariableType.Struct:
			return (
				<JsonValueInput
					value={value}
					onChange={onChange}
					disabled={disabled}
					requireObject
					placeholder={`{"key": "value"}`}
				/>
			);
		case IVariableType.Generic:
			return (
				<JsonValueInput
					value={value}
					onChange={onChange}
					disabled={disabled}
					placeholder={t('jsonValue', 'JSON value...')}
				/>
			);
		default:
			return (
				<Input
					disabled={disabled}
					type={secret ? "password" : "text"}
					placeholder="Value..."
					className="flex-1 min-w-0"
					value={typeof value === "string" ? value : ""}
					onChange={(e) => onChange(e.target.value, true)}
				/>
			);
	}
}

export function MapVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
}>) {
	const { t } = useTranslation("flow");
	const dataType = variable.data_type;

	const entries = useMemo<MapEntries>(() => {
		const parsed = parseUint8ArrayToJson(variable.default_value);
		return isRecord(parsed) ? parsed : {};
	}, [variable.default_value]);

	const [newKey, setNewKey] = useState("");
	const [{ value: newValue, valid: newValueValid }, setNewEntry] = useState(
		() => initialValueForType(dataType),
	);
	// Bumped on every reset to remount the value input, clearing any internal
	// draft state (e.g. the JSON textarea for Struct/Generic values).
	const [entryNonce, setEntryNonce] = useState(0);

	const resetEntry = useCallback(() => {
		setNewKey("");
		setNewEntry(initialValueForType(dataType));
		setEntryNonce((nonce) => nonce + 1);
	}, [dataType]);

	// Reset the pending entry when the map's value type changes.
	const dataTypeRef = useRef(dataType);
	useEffect(() => {
		if (dataTypeRef.current === dataType) return;
		dataTypeRef.current = dataType;
		resetEntry();
	}, [dataType, resetEntry]);

	const commit = useCallback(
		(next: MapEntries) => {
			onChange({
				...variable,
				default_value: convertJsonToUint8Array(next),
			});
		},
		[onChange, variable],
	);

	const keyExists = newKey.trim() !== "" && newKey.trim() in entries;

	const handleAdd = useCallback(() => {
		if (disabled) return;
		const key = newKey.trim();
		if (!key || !newValueValid) return;
		commit({ ...entries, [key]: newValue });
		resetEntry();
	}, [disabled, newKey, newValueValid, newValue, entries, commit, resetEntry]);

	const handleRemove = useCallback(
		(key: string) => {
			if (disabled) return;
			const { [key]: _removed, ...rest } = entries;
			commit(rest);
		},
		[disabled, entries, commit],
	);

	const entryList = Object.entries(entries);

	return (
		<div className="flex flex-col gap-3 w-full min-w-0">
			<div className="flex flex-col gap-2 w-full min-w-0">
				<div className="flex gap-2 w-full min-w-0 items-start">
					<div className="relative flex-1 min-w-0">
						<KeyIcon className="absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
						<Input
							value={newKey}
							onChange={(e) => setNewKey(e.target.value)}
							onKeyDown={(e) => e.key === "Enter" && handleAdd()}
							placeholder="Key..."
							disabled={disabled}
							className="pl-7"
						/>
					</div>
					<MapValueInput
						key={entryNonce}
						dataType={dataType}
						value={newValue}
						onChange={(value, valid) => setNewEntry({ value, valid })}
						disabled={disabled}
						secret={variable.secret}
					/>
					<Button
						size="icon"
						variant="default"
						onClick={handleAdd}
						disabled={!newKey.trim() || !newValueValid || disabled}
						className="shrink-0"
					>
						<PlusCircleIcon className="w-4 h-4" />
					</Button>
				</div>
				{keyExists && (
					<p className="text-xs text-muted-foreground">
						{t(
							"keyAlreadyExistsAddingWillOverwriteIt",
							'Key "{{key}}" already exists — adding will overwrite it.',
							{ key: newKey.trim() },
						)}
					</p>
				)}
			</div>

			{entryList.length > 0 && (
				<>
					<Separator />
					<div className="flex flex-col gap-2 rounded-md border p-3">
						{entryList.map(([key, value]) => (
							<div
								key={`${variable.name}-${key}`}
								className="group flex items-start gap-2 rounded-md bg-secondary px-3 py-2 text-sm"
							>
								<span className="font-medium wrap-break-word min-w-0 max-w-[40%] shrink-0">
									{key}
								</span>
								<span className="text-muted-foreground shrink-0">:</span>
								<span className="flex-1 wrap-break-word min-w-0 font-mono text-xs">
									{variable.secret
										? "••••••••"
										: renderValueDisplay(dataType, value)}
								</span>
								<Button
									size="icon"
									variant="ghost"
									onClick={() => handleRemove(key)}
									disabled={disabled}
									className="h-5 w-5 shrink-0 rounded-sm hover:bg-destructive hover:text-destructive-foreground"
								>
									<XIcon className="h-3 w-3" />
								</Button>
							</div>
						))}
					</div>
				</>
			)}
		</div>
	);
}
