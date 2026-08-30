"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, ChevronsUpDown } from "lucide-react";
import { useState } from "react";
import { cn } from "../../../../lib/utils";
import { Button } from "../../button";
import {
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "../../command";
import { Popover, PopoverContent, PopoverTrigger } from "../../popover";

export interface ColumnPickerProps {
	columns: string[];
	value: string;
	onChange: (value: string) => void;
	placeholder?: string;
	disabled?: boolean;
}

export function ColumnPicker({
	columns,
	value,
	onChange,
	placeholder,
	disabled,
}: ColumnPickerProps) {
	const { t } = useTranslation("common");
	const [open, setOpen] = useState(false);

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<Button
					variant="outline"
					size="sm"
					className="w-full justify-between text-xs font-normal h-8"
					disabled={disabled}
				>
					<span className="truncate">
						{value || placeholder || t("selectColumn2", "Select column...")}
					</span>
					<ChevronsUpDown className="ml-1 h-3 w-3 shrink-0 opacity-50" />
				</Button>
			</PopoverTrigger>
			<PopoverContent className="w-[200px] p-0" align="start">
				<Command>
					<CommandInput
						placeholder={t("filterColumns2", "Filter columns...")}
						className="h-8 text-xs"
					/>
					<CommandList>
						<CommandEmpty className="text-xs p-2">
							{t("noColumnFound", "No column found.")}
						</CommandEmpty>
						<CommandGroup>
							{columns.map((col) => (
								<CommandItem
									key={col}
									value={col}
									onSelect={() => {
										onChange(col);
										setOpen(false);
									}}
									className="text-xs"
								>
									<Check
										className={cn(
											"mr-2 h-3 w-3",
											value === col ? "opacity-100" : "opacity-0",
										)}
									/>
									{col}
								</CommandItem>
							))}
						</CommandGroup>
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}
