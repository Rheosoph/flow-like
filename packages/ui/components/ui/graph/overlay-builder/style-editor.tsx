"use client";

import type {
	LabelStyle,
	NodeSize,
} from "../../../../state/backend-state/graph-state";
import { Input } from "../../input";
import { Label } from "../../label";
import { ScrollArea } from "../../scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../select";
import { GRAPH_ICONS, type IconKey, getGraphIcon } from "../icons";

export interface StyleEditorProps {
	style: LabelStyle;
	onChange: (style: LabelStyle) => void;
}

function formatIconName(key: string): string {
	return key
		.replace(/([a-z])([A-Z])/g, "$1 $2")
		.replace(/^./, (c) => c.toUpperCase());
}

export function StyleEditor({ style, onChange }: StyleEditorProps) {
	const iconKeys = Object.keys(GRAPH_ICONS) as IconKey[];

	return (
		<div className="grid grid-cols-2 gap-3">
			<div className="space-y-1.5">
				<Label className="text-xs">Color</Label>
				<div className="flex items-center gap-2">
					<input
						type="color"
						value={style.color}
						onChange={(e) => onChange({ ...style, color: e.target.value })}
						className="w-8 h-8 rounded border cursor-pointer"
					/>
					<Input
						value={style.color}
						onChange={(e) => onChange({ ...style, color: e.target.value })}
						className="h-8 text-xs font-mono flex-1"
						placeholder="#3b82f6"
					/>
				</div>
			</div>

			<div className="space-y-1.5">
				<Label className="text-xs">Icon</Label>
				<Select
					value={style.icon}
					onValueChange={(v) => onChange({ ...style, icon: v })}
				>
					<SelectTrigger className="h-8 text-xs">
						{style.icon ? (
							(() => {
								const Icon = getGraphIcon(style.icon);
								return (
									<span className="flex items-center gap-1.5">
										<Icon className="h-3.5 w-3.5" />
										{formatIconName(style.icon)}
									</span>
								);
							})()
						) : (
							<SelectValue placeholder="Select icon" />
						)}
					</SelectTrigger>
					<SelectContent position="popper" className="max-h-60">
						<ScrollArea className="max-h-56">
							{iconKeys.map((key) => {
								const Icon = GRAPH_ICONS[key];
								return (
									<SelectItem key={key} value={key} className="text-xs">
										<span className="flex items-center gap-1.5">
											{Icon && <Icon className="h-3.5 w-3.5" />}
											{formatIconName(key)}
										</span>
									</SelectItem>
								);
							})}
						</ScrollArea>
					</SelectContent>
				</Select>
			</div>

			<div className="space-y-1.5">
				<Label className="text-xs">Size Mode</Label>
				<Select
					value={style.size.mode}
					onValueChange={(v) =>
						onChange({
							...style,
							size: { ...style.size, mode: v as NodeSize["mode"] },
						})
					}
				>
					<SelectTrigger className="h-8 text-xs">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="fixed" className="text-xs">
							Fixed
						</SelectItem>
						<SelectItem value="by-degree" className="text-xs">
							By Degree
						</SelectItem>
						<SelectItem value="by-column" className="text-xs">
							By Column
						</SelectItem>
					</SelectContent>
				</Select>
			</div>

			{style.size.mode === "fixed" && (
				<div className="space-y-1.5">
					<Label className="text-xs">Size</Label>
					<Input
						type="number"
						value={style.size.value ?? 10}
						onChange={(e) =>
							onChange({
								...style,
								size: { ...style.size, value: Number(e.target.value) },
							})
						}
						className="h-8 text-xs"
						min={1}
						max={100}
					/>
				</div>
			)}

			{style.size.mode === "by-column" && (
				<div className="space-y-1.5">
					<Label className="text-xs">Size Column</Label>
					<Input
						value={style.size.column ?? ""}
						onChange={(e) =>
							onChange({
								...style,
								size: { ...style.size, column: e.target.value },
							})
						}
						className="h-8 text-xs"
						placeholder="column_name"
					/>
				</div>
			)}

			<div className="space-y-1.5">
				<Label className="text-xs">Edge Width</Label>
				<Input
					type="number"
					value={style.width ?? 2}
					onChange={(e) =>
						onChange({ ...style, width: Number(e.target.value) })
					}
					className="h-8 text-xs"
					min={1}
					max={20}
				/>
			</div>
		</div>
	);
}
