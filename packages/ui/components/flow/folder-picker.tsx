"use client";

import { useTranslation } from "@flow-like/locales";
import { FolderIcon } from "lucide-react";
import {
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../components/ui";
import { normalizeCategory } from "./category-tree";

/** Sentinel for "no folder" — Radix Select cannot hold an empty string value. */
export const ROOT_FOLDER = "__root__";
/** Sentinel for "type a new folder path". */
export const CUSTOM_FOLDER = "__custom__";

/** Resolves the picker's state back to a `category` string. */
export const resolveFolder = (
	folder: string,
	customFolder: string,
): string | undefined => {
	if (folder === CUSTOM_FOLDER) return normalizeCategory(customFolder);
	if (folder === ROOT_FOLDER) return undefined;
	return normalizeCategory(folder);
};

export interface IFolderPickerProps {
	folder: string;
	onFolderChange: (next: string) => void;
	customFolder: string;
	onCustomFolderChange: (next: string) => void;
	options: string[];
	hint: string;
	disabled?: boolean;
}

/**
 * Folder selection for anything carrying a `"/"`-separated `category`.
 *
 * A select over the folders that already exist, plus a free-text path for a new
 * one — the raw `Main/Bools` text field it replaces gave no clue which folders
 * were already there, so every typo silently created a new one.
 */
export function FolderPicker({
	folder,
	onFolderChange,
	customFolder,
	onCustomFolderChange,
	options,
	hint,
	disabled,
}: Readonly<IFolderPickerProps>) {
	const { t } = useTranslation("flow");

	return (
		<div className="border-b border-border/40 px-4 py-3">
			<div className="flex max-w-xl flex-wrap items-end gap-3">
				<div className="min-w-[12rem] flex-1 space-y-1.5">
					<Label className="text-xs">{t("folder", "Folder")}</Label>
					<Select
						value={folder}
						onValueChange={onFolderChange}
						disabled={disabled}
					>
						<SelectTrigger className="h-8 w-full text-xs">
							<SelectValue placeholder={t("topLevel", "Top level")} />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value={ROOT_FOLDER}>
								{t("topLevel", "Top level")}
							</SelectItem>
							{options.map((path) => (
								<SelectItem key={path} value={path}>
									<span className="flex items-center gap-2">
										<FolderIcon className="size-3.5 text-muted-foreground" />
										{path}
									</span>
								</SelectItem>
							))}
							<SelectItem value={CUSTOM_FOLDER}>
								{t("newFolder", "New folder…")}
							</SelectItem>
						</SelectContent>
					</Select>
				</div>

				{folder === CUSTOM_FOLDER && (
					<div className="min-w-[12rem] flex-1 space-y-1.5">
						<Label className="text-xs">{t("folderPath", "Folder path")}</Label>
						<Input
							autoFocus
							disabled={disabled}
							className="h-8 text-xs"
							value={customFolder}
							onChange={(event) => onCustomFolderChange(event.target.value)}
							placeholder={t("egUtilsmath", "e.g. Utils/Math")}
						/>
					</div>
				)}

				<p className="w-full text-xs text-muted-foreground">{hint}</p>
			</div>
		</div>
	);
}
