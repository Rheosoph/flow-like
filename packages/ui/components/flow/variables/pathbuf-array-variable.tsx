import { useTranslation } from "@flow-like/locales";
import { FileIcon, FolderIcon, XIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Button } from "../../../components/ui/button";
import { Label } from "../../../components/ui/label";
import { Switch } from "../../../components/ui/switch";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Badge, Separator } from "../../ui";

export function PathbufArrayVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();

	// parse once from default_value
	const items = useMemo<string[]>(() => {
		const parsed = parseUint8ArrayToJson(variable.default_value);
		return Array.isArray(parsed) ? parsed : [];
	}, [variable.default_value]);

	const [isFolder, setIsFolder] = useState<boolean>(false);

	// add a new path
	const handleAdd = useCallback(async () => {
		if (disabled) return;
		const pathBuf: any = await backend.helperState.openFileOrFolderMenu(
			false,
			isFolder,
			true,
		);
		if (!pathBuf) return;

		let finalPath = pathBuf;

		if (!isFolder) {
			const meta = await backend.helperState.getPathMeta(pathBuf);
			if (!meta || meta.length === 0) return;
			finalPath = meta[0].location;
		}

		const updated = [...items, finalPath];
		onChange({
			...variable,
			default_value: convertJsonToUint8Array(updated),
		});
	}, [disabled, backend, isFolder, items, onChange, variable]);

	const handleRemove = useCallback(
		(idx: number) => {
			if (disabled) return;
			const updated = items.filter((_, i) => i !== idx);
			onChange({
				...variable,
				default_value: convertJsonToUint8Array(updated),
			});
		},
		[disabled, items, onChange, variable],
	);

	return (
		<div className="flex flex-col gap-3 w-full min-w-0">
			<div className="flex gap-2 items-center">
				<div className="flex items-center gap-2">
					<Switch
						checked={isFolder}
						onCheckedChange={setIsFolder}
						id="is_folder"
						disabled={disabled}
					/>
					<Label htmlFor="is_folder" className="cursor-pointer">
						{t('folder', 'Folder')}
					</Label>
				</div>

				<Button
					variant="outline"
					className={cn(
						"flex-1 justify-start text-left font-normal min-w-0",
						items.length === 0 && "text-muted-foreground",
					)}
					disabled={disabled}
					onClick={handleAdd}
				>
					{isFolder ? (
						<FolderIcon className="mr-2 h-4 w-4 shrink-0" />
					) : (
						<FileIcon className="mr-2 h-4 w-4 shrink-0" />
					)}
					<span className="truncate">
						{isFolder ? t('addFolder', 'Add Folder') : t('addFile', 'Add File')}
					</span>
				</Button>
			</div>

			{items.length > 0 && (
				<>
					<Separator />
					<div className="flex flex-col gap-2 rounded-md border p-3">
						{items.map((path, idx) => (
							<Badge
								key={`${variable.name}-${idx}`}
								variant="secondary"
								className="group flex items-center gap-2 pr-1 max-w-full justify-between"
							>
								<div className="flex items-center gap-2 min-w-0 flex-1">
									{!path.split("/").pop()?.includes(".") ? (
										<FolderIcon className="h-4 w-4 shrink-0" />
									) : (
										<FileIcon className="h-4 w-4 shrink-0" />
									)}
									<span className="break-all text-xs">
										{path.split("/").pop()}
									</span>
								</div>
								<Button
									disabled={disabled}
									size="icon"
									variant="ghost"
									onClick={() => handleRemove(idx)}
									className="h-4 w-4 shrink-0 rounded-sm hover:bg-destructive hover:text-destructive-foreground"
								>
									<XIcon className="h-3 w-3" />
								</Button>
							</Badge>
						))}
					</div>
				</>
			)}
		</div>
	);
}
