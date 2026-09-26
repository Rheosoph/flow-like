"use client";

import {
	formatAbsoluteDateTime,
	formatRelativeTime,
	parseTemporalValue,
} from "../../../../lib/date";
import { resolveStorageFile } from "../../../../lib/storage-file";
import { cn } from "../../../../lib/utils";
import { accountIdFromValue } from "../../../../state/backend-state/user-state";
import {
	BinaryCellPreview,
	BinaryValueDetail,
} from "../../../ui/binary-value-cell";
import { GeometryCell, GeometryDetails } from "../../../ui/geometry-cell";
import { RelativeTime } from "../../../ui/relative-time";
import { StorageFileCell } from "../../../ui/storage-file-cell";
import { UserIdentityCard, UserInlineTag } from "../../../ui/user-identity";
import {
	type ColumnKind,
	cellToString,
	formatNumber,
	isNullish,
} from "./column-types";

interface ResultValueProps {
	value: unknown;
	kind: ColumnKind;
	name: string;
	appId?: string;
	metadata?: Record<string, string>;
}

/** One row-height reading of a result value, shared by every compact result surface. */
export function ResultCellValue({
	value,
	kind,
	name,
	appId,
	metadata,
}: Readonly<ResultValueProps>) {
	if (isNullish(value)) {
		return (
			<span className="select-none italic text-muted-foreground/50">NULL</span>
		);
	}
	if (kind === "geometry")
		return <GeometryCell value={value} metadata={metadata} />;
	if (kind === "binary") return <BinaryCellPreview value={value} />;
	if (kind === "boolean") {
		const truthy = value === true || value === "true" || value === 1;
		return (
			<span className="flex items-center gap-1.5">
				<span
					className={cn(
						"h-1.5 w-1.5 rounded-full",
						truthy ? "bg-chart-2" : "bg-muted-foreground/40",
					)}
				/>
				{String(value)}
			</span>
		);
	}
	if (kind === "number") {
		return <span className="tabular-nums">{formatNumber(value)}</span>;
	}
	if (kind === "temporal") {
		return <RelativeTime value={value} className="min-w-0 truncate" />;
	}
	// A user column still holds text for rows that name no account, so the tag is
	// used only where the value is an id the directory could answer for.
	if (kind === "user") {
		const userId = accountIdFromValue(value);
		if (userId) return <UserInlineTag userId={userId} />;
	}
	// Same story for files: the column holds paths, but a row may hold a path that
	// points nowhere this app can open, and that row stays text.
	if (kind === "file") {
		const file = resolveStorageFile(name, value, appId);
		if (file && appId) return <StorageFileCell appId={appId} file={file} />;
	}
	return <span className="min-w-0 truncate">{cellToString(value)}</span>;
}

/** A detail view has room for the exact instant, with the relative reading beside it. */
export function TemporalDetailValue({ value }: Readonly<{ value: unknown }>) {
	const parsed = parseTemporalValue(value);
	if (!parsed) return <>{cellToString(value)}</>;

	return (
		<>
			{formatAbsoluteDateTime(parsed)}
			<span className="ml-1.5 text-xs text-muted-foreground">
				{formatRelativeTime(parsed, "long")}
			</span>
		</>
	);
}

/** The roomy reading of a result value, for row details and single records. */
export function ResultDetailValue({
	value,
	kind,
	name,
	appId,
	metadata,
}: Readonly<ResultValueProps>) {
	if (kind === "geometry")
		return <GeometryDetails value={value} metadata={metadata} />;
	if (kind === "binary")
		return <BinaryValueDetail value={value} className="mt-1 font-sans" />;
	if (kind === "temporal") return <TemporalDetailValue value={value} />;

	const userId = kind === "user" ? accountIdFromValue(value) : null;
	if (userId) return <UserIdentityCard userId={userId} className="mt-1" />;

	const file = appId ? resolveStorageFile(name, value, appId) : null;
	if (file && appId)
		return <StorageFileCell appId={appId} file={file} className="-ml-2 mt-1" />;

	return <>{cellToString(value)}</>;
}
