import { ChevronDown } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useBackend } from "../../../..";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import type { IAccessibleApp } from "../../../../state/backend-state/types";

function normalizeStringValue(value: number[] | undefined | null): string {
	const parsed = parseUint8ArrayToJson(value);
	return typeof parsed === "string" ? parsed : "";
}

export function RemoteProjectSelect({
	pin,
	value,
	appId,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	setValue: (value: number[] | undefined) => void;
}>) {
	const backend = useBackend();
	const [open, setOpen] = useState(false);
	const [apps, setApps] = useState<IAccessibleApp[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState(false);
	const hasLoadedRef = useRef(false);
	const selectedAppId = normalizeStringValue(value);

	useEffect(() => {
		if (!appId) return;
		const needsInitialLabel = Boolean(selectedAppId) && !hasLoadedRef.current;
		if (!open && !needsInitialLabel) return;

		let cancelled = false;

		async function loadAccessibleApps() {
			setLoading(true);
			setError(false);

			try {
				const accessibleApps = await backend.teamState.getAccessibleApps(appId);

				if (cancelled) return;

				hasLoadedRef.current = true;
				setApps(accessibleApps);
			} catch {
				if (!cancelled) setError(true);
			} finally {
				if (!cancelled) setLoading(false);
			}
		}

		void loadAccessibleApps();

		return () => {
			cancelled = true;
		};
	}, [appId, backend.teamState, open, selectedAppId]);

	const handleOpenChange = useCallback((isOpen: boolean) => {
		setOpen(isOpen);
	}, []);

	const selectedApp = apps.find((app) => app.app_id === selectedAppId);

	return (
		<div
			className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden"
			onMouseDown={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			<Select
				open={open}
				onOpenChange={handleOpenChange}
				value={selectedAppId || undefined}
				onValueChange={(targetAppId) =>
					setValue(convertJsonToUint8Array(targetAppId))
				}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{selectedAppId
							? (selectedApp?.name ?? selectedAppId)
							: "Select project"}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
				</SelectTrigger>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{loading && apps.length === 0 && (
							<SelectLabel>Loading projects...</SelectLabel>
						)}
						{error && <SelectLabel>Could not load accessible apps</SelectLabel>}
						{!loading && !error && apps.length === 0 && (
							<SelectLabel>No accessible apps found</SelectLabel>
						)}
						{apps.map((app) => (
							<SelectItem key={app.app_id} value={app.app_id}>
								<div className="flex min-w-0 flex-col items-start gap-0">
									<span className="max-w-48 truncate">
										{app.name ?? app.app_id}
									</span>
									<span className="max-w-48 truncate text-xs text-muted-foreground">
										{app.app_id}
									</span>
								</div>
							</SelectItem>
						))}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}
