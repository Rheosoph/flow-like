"use client";

import { useId, useRef } from "react";
import type { OfflineWritesConfig } from "../../../lib/device-management/deployment";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

const defaults = (): OfflineWritesConfig => ({
	tables: [],
	files: [],
	max_queue_bytes: 256 * 1048576,
	max_operations: 10000,
	max_age_seconds: 7 * 86400,
	max_mirror_bytes: 2 * 1024 ** 3,
});

type KeyedRow<T> = { value: T; key: string };
function retainRowKeys<T>(values: T[], previous: KeyedRow<T>[]): KeyedRow<T>[] {
	return values.map((value, index) => ({
		value,
		key:
			previous.find((row) => row.value === value)?.key ??
			(previous[index] && !values.includes(previous[index].value)
				? previous[index].key
				: crypto.randomUUID()),
	}));
}

export function DeviceOfflineWritesFields({
	value,
	onChange,
}: {
	value: OfflineWritesConfig | null;
	onChange: (value: OfflineWritesConfig | null) => void;
}) {
	const id = useId();
	const tableRows = useRef<KeyedRow<OfflineWritesConfig["tables"][number]>[]>(
		[],
	);
	const fileRows = useRef<KeyedRow<OfflineWritesConfig["files"][number]>[]>([]);
	tableRows.current = retainRowKeys(value?.tables ?? [], tableRows.current);
	fileRows.current = retainRowKeys(value?.files ?? [], fileRows.current);
	return (
		<fieldset className="space-y-3 rounded border p-3">
			<legend className="px-1 text-sm font-medium">Offline writes</legend>
			<label className="flex items-center gap-2 text-sm">
				<input
					type="checkbox"
					checked={value !== null}
					onChange={(event) =>
						onChange(event.target.checked ? defaults() : null)
					}
				/>
				Buffer selected writes on this device
			</label>
			<p className="text-xs text-muted-foreground">
				When enabled, selected writes are accepted on the device and replayed in
				order when the cloud is available. Workflow Write State reports pending
				cloud replay. Conflicts or denied access block the queue for review.
			</p>
			{value && (
				<>
					<div className="space-y-2">
						<p className="text-sm font-medium">Tables</p>
						<p className="text-xs text-muted-foreground">
							Select each table in the db directory and its stable primary key.
							A complete local table copy provides reads while writes are
							pending.
						</p>
						{tableRows.current.map(({ value: table, key }, index) => (
							<div
								className="grid gap-2 rounded border p-2 sm:grid-cols-2"
								key={key}
							>
								<label className="text-sm">
									Table scope
									<select
										className="block w-full rounded border bg-background p-2"
										value={table.purpose}
										onChange={(event) =>
											onChange({
												...value,
												tables: value.tables.map((entry, i) =>
													i === index
														? {
																...entry,
																purpose: event.target.value as
																	| "storage"
																	| "user",
															}
														: entry,
												),
											})
										}
									>
										<option value="storage">Project storage</option>
										<option value="user">Delegating user</option>
									</select>
								</label>
								{(
									[
										["table", "Table name"],
										["primary_key", "Primary key column"],
									] as const
								).map(([field, label]) => (
									<label
										className="text-sm"
										key={field}
										htmlFor={`${id}-${key}-${field}`}
									>
										{label}
										<Input
											id={`${id}-${key}-${field}`}
											value={table[field]}
											onChange={(event) =>
												onChange({
													...value,
													tables: value.tables.map((entry, i) =>
														i === index
															? { ...entry, [field]: event.target.value }
															: entry,
													),
												})
											}
										/>
									</label>
								))}
								<Button
									type="button"
									variant="outline"
									aria-label={`Remove buffered table ${index + 1}`}
									onClick={() =>
										onChange({
											...value,
											tables: value.tables.filter((_, i) => i !== index),
										})
									}
								>
									Remove table
								</Button>
							</div>
						))}
						<Button
							type="button"
							variant="outline"
							disabled={value.tables.length + value.files.length >= 64}
							onClick={() =>
								onChange({
									...value,
									tables: [
										...value.tables,
										{
											purpose: "storage",
											database: "db",
											table: "",
											primary_key: "id",
										},
									],
								})
							}
						>
							Add buffered table
						</Button>
					</div>
					<div className="space-y-2">
						<p className="text-sm font-medium">File directories</p>
						<p className="text-xs text-muted-foreground">
							Paths are relative to the selected cloud storage area. Select
							application files outside Lance table directories.
						</p>
						{fileRows.current.map(({ value: file, key }, index) => (
							<div
								className="grid gap-2 rounded border p-2 sm:grid-cols-2"
								key={key}
							>
								<label className="text-sm">
									File scope
									<select
										className="block w-full rounded border bg-background p-2"
										value={file.purpose}
										onChange={(event) =>
											onChange({
												...value,
												files: value.files.map((entry, i) =>
													i === index
														? {
																...entry,
																purpose: event.target
																	.value as OfflineWritesConfig["files"][number]["purpose"],
															}
														: entry,
												),
											})
										}
									>
										<option value="files">Uploads</option>
										<option value="storage">Project storage</option>
										<option value="user">Delegating user</option>
										<option value="temporary">User temporary files</option>
									</select>
								</label>
								<label className="text-sm" htmlFor={`${id}-${key}-prefix`}>
									Directory prefix
									<Input
										id={`${id}-${key}-prefix`}
										placeholder="exports"
										value={file.prefix}
										onChange={(event) =>
											onChange({
												...value,
												files: value.files.map((entry, i) =>
													i === index
														? { ...entry, prefix: event.target.value }
														: entry,
												),
											})
										}
									/>
								</label>
								<Button
									type="button"
									variant="outline"
									aria-label={`Remove buffered directory ${index + 1}`}
									onClick={() =>
										onChange({
											...value,
											files: value.files.filter((_, i) => i !== index),
										})
									}
								>
									Remove directory
								</Button>
							</div>
						))}
						<Button
							type="button"
							variant="outline"
							disabled={value.tables.length + value.files.length >= 64}
							onClick={() =>
								onChange({
									...value,
									files: [...value.files, { purpose: "storage", prefix: "" }],
								})
							}
						>
							Add buffered directory
						</Button>
					</div>
					<div className="grid gap-2 sm:grid-cols-2">
						{(
							[
								["max_queue_bytes", "Queue budget (MiB)", 1048576, 1, 65536],
								[
									"max_mirror_bytes",
									"Table copy budget (MiB)",
									1048576,
									1,
									1048576,
								],
								["max_operations", "Maximum queued operations", 1, 1, 100000],
								[
									"max_age_seconds",
									"Maximum queue age (minutes)",
									60,
									1,
									43200,
								],
							] as const
						).map(([field, label, unit, min, max]) => (
							<label className="text-sm" key={field} htmlFor={`${id}-${field}`}>
								{label}
								<Input
									id={`${id}-${field}`}
									type="number"
									min={min}
									max={max}
									step={1}
									value={value[field] / unit}
									onChange={(event) =>
										onChange({
											...value,
											[field]: Number(event.target.value) * unit,
										})
									}
								/>
							</label>
						))}
					</div>
					<p className="text-xs text-muted-foreground">
						Limits reject new writes when the queue is full or too old. Accepted
						operations are not evicted to make room.
					</p>
				</>
			)}
		</fieldset>
	);
}
