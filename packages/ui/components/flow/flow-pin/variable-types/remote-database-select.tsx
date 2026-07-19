import { ChevronDown } from "lucide-react";
import { type RefObject, useCallback, useEffect, useState } from "react";
import { useBackend } from "../../../..";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import type { IBoard } from "../../../../lib/schema/flow/board";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";

const REMOTE_APP_PIN_NAME = "_flow_remote_app_id";

function normalizeStringValue(value: number[] | undefined | null): string {
	const parsed = parseUint8ArrayToJson(value);
	return typeof parsed === "string" ? parsed : "";
}

export function RemoteDatabaseSelect({
	pin,
	value,
	appId,
	nodeId,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	nodeId: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const backend = useBackend();
	const [open, setOpen] = useState(false);
	const [loadedTables, setLoadedTables] = useState<{
		targetAppId: string;
		tables: string[];
	}>({ targetAppId: "", tables: [] });
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState(false);
	const selectedTable = normalizeStringValue(value);

	const remoteAppPin = Object.values(
		boardRef?.current?.nodes?.[nodeId]?.pins ?? {},
	).find((nodePin) => nodePin.name === REMOTE_APP_PIN_NAME);
	const targetAppId = normalizeStringValue(remoteAppPin?.default_value);

	const tables =
		loadedTables.targetAppId === targetAppId ? loadedTables.tables : [];
	const tablesLoaded = loadedTables.targetAppId === targetAppId && !loading;
	const selectedTableMissing =
		Boolean(selectedTable) &&
		tablesLoaded &&
		!error &&
		!tables.includes(selectedTable);

	useEffect(() => {
		if (!appId || !targetAppId || !open) return;

		let cancelled = false;

		async function loadTables() {
			setLoading(true);
			setError(false);

			try {
				const remoteTables = await backend.teamState.getRemoteTables(
					appId,
					targetAppId,
				);

				if (cancelled) return;

				setLoadedTables({ targetAppId, tables: remoteTables });
			} catch {
				if (!cancelled) setError(true);
			} finally {
				if (!cancelled) setLoading(false);
			}
		}

		void loadTables();

		return () => {
			cancelled = true;
		};
	}, [appId, backend.teamState, open, targetAppId]);

	const handleOpenChange = useCallback((isOpen: boolean) => {
		setOpen(isOpen);
	}, []);

	return (
		<div
			className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden"
			onMouseDown={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			<Select
				disabled={!targetAppId}
				open={open}
				onOpenChange={handleOpenChange}
				value={selectedTable || undefined}
				onValueChange={(table) => setValue(convertJsonToUint8Array(table))}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{!targetAppId && "Select a project first"}
						{targetAppId && (selectedTable || "Select database")}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
				</SelectTrigger>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{loading && tables.length === 0 && (
							<SelectLabel>Loading databases...</SelectLabel>
						)}
						{error && (
							<SelectLabel>Could not load remote databases</SelectLabel>
						)}
						{!loading && !error && tables.length === 0 && !selectedTable && (
							<SelectLabel>No shared databases found</SelectLabel>
						)}
						{tables.map((table) => (
							<SelectItem key={table} value={table}>
								{table}
							</SelectItem>
						))}
						{selectedTable && !tables.includes(selectedTable) && (
							<SelectItem key={selectedTable} value={selectedTable}>
								{selectedTable}
								{selectedTableMissing && (
									<span className="text-muted-foreground">
										{" "}
										(not found in project)
									</span>
								)}
							</SelectItem>
						)}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}
