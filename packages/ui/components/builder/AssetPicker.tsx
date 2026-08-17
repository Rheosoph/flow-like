"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { AlertCircle, FolderOpen, ImageIcon, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import type { IStorageItem } from "../../lib/schema/storage/storage-item";
import { useBackend } from "../../state/backend-state";
import {
	Button,
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
	Input,
	ScrollArea,
} from "../ui";
import {
	basename,
	getExtension,
	matchesAccept,
	parentPrefix,
	resolveAssetPath,
} from "./asset-path";

export interface AssetPickerProps {
	appId: string;
	value?: string;
	onChange: (value: string) => void;
	accept?:
		| "image"
		| "model"
		| "video"
		| "audio"
		| "document"
		| "animation"
		| "environment"
		| "all";
	placeholder?: string;
	disabled?: boolean;
}

function FileItem({
	item,
	onSelect,
	onNavigate,
	accept,
}: {
	item: IStorageItem;
	onSelect: (location: string) => void;
	onNavigate: (location: string) => void;
	accept: string;
}) {
	const isDir = item.is_dir;
	const name = basename(item.location);
	const isSelectable = !isDir && matchesAccept(item.location, accept);
	const ext = getExtension(item.location);

	const handleClick = () => {
		if (isDir) {
			onNavigate(item.location);
		} else if (isSelectable) {
			onSelect(item.location);
		}
	};

	return (
		<button
			type="button"
			onClick={handleClick}
			disabled={!isDir && !isSelectable}
			className={`
				flex items-center gap-2 w-full p-2 rounded-md text-left text-sm
				${isDir ? "hover:bg-accent cursor-pointer" : ""}
				${isSelectable ? "hover:bg-primary/10 cursor-pointer" : ""}
				${!isDir && !isSelectable ? "opacity-50 cursor-not-allowed" : ""}
			`}
		>
			{isDir ? (
				<FolderOpen className="h-4 w-4 text-muted-foreground shrink-0" />
			) : (
				<ImageIcon className="h-4 w-4 text-muted-foreground shrink-0" />
			)}
			<span className="truncate flex-1">{name}</span>
			{!isDir && ext && (
				<span className="text-xs text-muted-foreground uppercase">{ext}</span>
			)}
		</button>
	);
}

export function AssetPicker({
	appId,
	value,
	onChange,
	accept = "all",
	placeholder = i18next.t("selectAsset", "Select asset..."),
	disabled = false,
}: AssetPickerProps) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const [open, setOpen] = useState(false);
	const [prefix, setPrefix] = useState("");
	const [inputValue, setInputValue] = useState(value ?? "");

	// Sync input value with external value
	useEffect(() => {
		setInputValue(value ?? "");
	}, [value]);

	useEffect(() => {
		if (disabled) setOpen(false);
	}, [disabled]);

	const items = useInvoke(
		backend.storageState.listStorageItems,
		backend.storageState,
		[appId, prefix],
		open && !disabled && typeof appId === "string" && appId.length > 0,
	);

	const handleSelect = useCallback(
		(location: string) => {
			const selected = resolveAssetPath(prefix, location);
			onChange(selected);
			setInputValue(selected);
			setOpen(false);
		},
		[prefix, onChange],
	);

	const handleNavigate = useCallback(
		(location: string) => {
			setPrefix(resolveAssetPath(prefix, location));
		},
		[prefix],
	);

	const handleGoUp = useCallback(() => {
		setPrefix(parentPrefix(prefix));
	}, [prefix]);

	const handleInputChange = (newValue: string) => {
		setInputValue(newValue);
		onChange(newValue);
	};

	const handleClear = () => {
		setInputValue("");
		onChange("");
	};

	// Sort items: directories first, then files. Directory-marker objects resolve
	// to the browsed folder itself and carry no name, so they are dropped.
	const sortedItems = useMemo(
		() =>
			(items.data ?? [])
				.filter((item) => basename(item.location).length > 0)
				.sort((a, b) => {
					if (a.is_dir && !b.is_dir) return -1;
					if (!a.is_dir && b.is_dir) return 1;
					return a.location.localeCompare(b.location);
				}),
		[items.data],
	);

	// Filter to only show directories and matching files
	const filteredItems = useMemo(
		() =>
			sortedItems.filter(
				(item) => item.is_dir || matchesAccept(item.location, accept),
			),
		[sortedItems, accept],
	);

	const hiddenCount = sortedItems.length - filteredItems.length;
	const breadcrumbParts = prefix.split("/").filter(Boolean);

	return (
		<div className="flex gap-1.5">
			<div className="relative flex-1">
				<Input
					disabled={disabled}
					value={inputValue}
					onChange={(e) => handleInputChange(e.target.value)}
					placeholder={placeholder}
					className="h-8 text-sm pr-8"
				/>
				{inputValue && !disabled && (
					<button
						type="button"
						onClick={handleClear}
						className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
					>
						<X className="h-3.5 w-3.5" />
					</button>
				)}
			</div>
			<Button
				type="button"
				variant="outline"
				size="sm"
				onClick={() => setOpen(true)}
				disabled={disabled}
				className="h-8 px-2"
			>
				<FolderOpen className="h-4 w-4" />
			</Button>

			<Dialog
				open={open && !disabled}
				onOpenChange={(nextOpen) => {
					if (!disabled) setOpen(nextOpen);
				}}
			>
				<DialogContent className="max-w-md">
					<DialogHeader>
						<DialogTitle>{t("selectAsset2", "Select Asset")}</DialogTitle>
					</DialogHeader>

					{/* Breadcrumbs */}
					<div className="flex items-center gap-1 text-sm">
						<button
							type="button"
							onClick={() => setPrefix("")}
							className="text-muted-foreground hover:text-foreground"
						>
							{t("root", "Root")}
						</button>
						{breadcrumbParts.map((part, index) => (
							<div key={part} className="flex items-center gap-1">
								<span className="text-muted-foreground">/</span>
								<button
									type="button"
									onClick={() =>
										setPrefix(breadcrumbParts.slice(0, index + 1).join("/"))
									}
									className="text-muted-foreground hover:text-foreground"
								>
									{part}
								</button>
							</div>
						))}
					</div>

					{/* File list */}
					<ScrollArea className="h-[300px] border rounded-md">
						<div className="p-2 space-y-0.5">
							{prefix && (
								<button
									type="button"
									onClick={handleGoUp}
									className="flex items-center gap-2 w-full p-2 rounded-md text-left text-sm hover:bg-accent"
								>
									<FolderOpen className="h-4 w-4 text-muted-foreground" />
									<span className="text-muted-foreground">..</span>
								</button>
							)}
							{items.isLoading ? (
								<div className="p-4 text-center text-sm text-muted-foreground">
									{t("loading", "Loading...")}
								</div>
							) : items.isError ? (
								<div className="flex flex-col items-center gap-1 p-4 text-center text-sm text-destructive">
									<AlertCircle className="h-4 w-4" />
									<span>
										{t("failedToLoadAssets", "Failed to load assets")}
									</span>
									<span className="text-xs text-muted-foreground">
										{items.error?.message}
									</span>
								</div>
							) : filteredItems.length === 0 ? (
								<div className="p-4 text-center text-sm text-muted-foreground">
									{hiddenCount > 0
										? t("noMatchingAssetsValHidden", {
												val: hiddenCount,
											})
										: t("noAssetsFound", "No assets found")}
								</div>
							) : (
								filteredItems.map((item) => (
									<FileItem
										key={item.location}
										item={item}
										onSelect={handleSelect}
										onNavigate={handleNavigate}
										accept={accept}
									/>
								))
							)}
						</div>
					</ScrollArea>
				</DialogContent>
			</Dialog>
		</div>
	);
}
