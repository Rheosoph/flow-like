"use client";
import { i18n as i18next } from "@flow-like/locales";
import { PaletteIcon } from "lucide-react";
import { memo, useMemo, useState } from "react";
import { HexAlphaColorPicker } from "react-colorful";
import { cn } from "../../../lib/utils";
import { Input } from "../../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

interface CommentColorPickerProps {
	value: string;
	onChange: (value: string) => void;
	onClose?: () => void;
}

const THEME_PRESETS = [
	// Neutrals - work with both light/dark
	{ color: "hsl(var(--card))", name: "Default" },
	// Warm colors
	{ color: "#fef3c7", name: "Yellow", darkVariant: "#78350f" },
	{ color: "#fed7aa", name: "Orange", darkVariant: "#7c2d12" },
	{ color: "#fecaca", name: "Red", darkVariant: "#7f1d1d" },
	{ color: "#fbcfe8", name: "Pink", darkVariant: "#831843" },
	// Cool colors
	{ color: "#ddd6fe", name: "Purple", darkVariant: "#4c1d95" },
	{ color: "#bfdbfe", name: "Blue", darkVariant: "#1e3a8a" },
	{ color: "#a5f3fc", name: "Cyan", darkVariant: "#164e63" },
	{ color: "#bbf7d0", name: "Green", darkVariant: "#14532d" },
] as const;

const PresetButton = memo(
	({
		color,
		name,
		isSelected,
		onClick,
	}: {
		color: string;
		name: string;
		isSelected: boolean;
		onClick: () => void;
	}) => (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					onClick={onClick}
					className={cn(
						"w-6 h-6 rounded-md border-2 transition-all hover:scale-110",
						isSelected
							? "border-primary ring-2 ring-primary/30"
							: "border-transparent hover:border-muted-foreground/30",
					)}
					style={{ backgroundColor: color }}
				/>
			</TooltipTrigger>
			<TooltipContent side="bottom" className="text-[10px] px-1.5 py-0.5">
				{name}
			</TooltipContent>
		</Tooltip>
	),
);

PresetButton.displayName = "PresetButton";

const CommentColorPicker = memo(
	({ value, onChange, onClose }: CommentColorPickerProps) => {
		const [showWheel, setShowWheel] = useState(false);
		const [open, setOpen] = useState(false);
		const parsedValue = useMemo(() => value || "#ffffff", [value]);

		const handleOpenChange = (isOpen: boolean) => {
			setOpen(isOpen);
			if (!isOpen) {
				setShowWheel(false);
				onClose?.();
			}
		};

		return (
			<Popover open={open} onOpenChange={handleOpenChange}>
				<PopoverTrigger asChild>
					<button
						type="button"
						className="absolute -left-8 top-1/2 -translate-y-1/2 z-50
							w-6 h-6 rounded-full border-2 border-white/50
							shadow-md hover:scale-110 transition-transform
							flex items-center justify-center"
						style={{ backgroundColor: parsedValue }}
					>
						<span className="sr-only">{i18next.t('pickColor', 'Pick color')}</span>
					</button>
				</PopoverTrigger>
				<PopoverContent
					side="left"
					align="center"
					className="w-auto p-2 bg-zinc-900 border-white/10 rounded-xl"
				>
					{showWheel ? (
						<div className="flex flex-col gap-2">
							<div className="flex items-center justify-between mb-1">
								<button
									type="button"
									onClick={() => setShowWheel(false)}
									className="text-[10px] text-zinc-400 hover:text-zinc-200"
								>
									{i18next.t('back2', '← Back')}
								</button>
							</div>
							<HexAlphaColorPicker
								color={parsedValue}
								onChange={onChange}
								style={{ width: "160px", height: "140px" }}
							/>
							<Input
								className="w-full h-7 text-xs bg-zinc-800 border-white/10 text-zinc-100"
								maxLength={9}
								onChange={(e) => onChange(e.currentTarget.value)}
								value={parsedValue}
							/>
						</div>
					) : (
						<div className="flex flex-col gap-2">
							<div className="grid grid-cols-5 gap-1.5">
								{THEME_PRESETS.map((preset) => (
									<PresetButton
										key={preset.name}
										color={preset.color}
										name={preset.name}
										isSelected={
											value === preset.color ||
											("darkVariant" in preset && value === preset.darkVariant)
										}
										onClick={() => onChange(preset.color)}
									/>
								))}
								<Tooltip>
									<TooltipTrigger asChild>
										<button
											type="button"
											onClick={() => setShowWheel(true)}
											className="w-6 h-6 rounded-md border-2 border-dashed border-zinc-600
												hover:border-zinc-400 transition-colors
												flex items-center justify-center text-zinc-400 hover:text-zinc-200"
										>
											<PaletteIcon className="w-3 h-3" />
										</button>
									</TooltipTrigger>
									<TooltipContent
										side="bottom"
										className="text-[10px] px-1.5 py-0.5"
									>
										{i18next.t('customColor', 'Custom Color')}
									</TooltipContent>
								</Tooltip>
							</div>
						</div>
					)}
				</PopoverContent>
			</Popover>
		);
	},
);

CommentColorPicker.displayName = "CommentColorPicker";

export { CommentColorPicker };
