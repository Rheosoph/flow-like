"use client";
import { useMemo } from "react";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";

export interface AppOption {
	readonly id: string;
	readonly name: string;
}

interface AppLinkPickerProps {
	readonly apps: ReadonlyArray<AppOption>;
	readonly value: { appId: string; alias: string; purpose: string };
	readonly onChange: (next: {
		appId: string;
		alias: string;
		purpose: string;
	}) => void;
	readonly disabled?: boolean;
}

export function AppLinkPicker({
	apps,
	value,
	onChange,
	disabled,
}: AppLinkPickerProps) {
	const sortedApps = useMemo(
		() =>
			[...apps].sort((a, b) =>
				(a.name ?? a.id).localeCompare(b.name ?? b.id),
			),
		[apps],
	);
	return (
		<div className="grid grid-cols-1 md:grid-cols-3 gap-3">
			<div className="space-y-2">
				<Label>App</Label>
				<Select
					value={value.appId}
					onValueChange={(v) => onChange({ ...value, appId: v })}
					disabled={disabled}
				>
					<SelectTrigger>
						<SelectValue placeholder="Pick an app" />
					</SelectTrigger>
					<SelectContent>
						{sortedApps.map((a) => (
							<SelectItem key={a.id} value={a.id}>
								{a.name || a.id}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
			<div className="space-y-2">
				<Label>Alias</Label>
				<Input
					value={value.alias}
					onChange={(e) => onChange({ ...value, alias: e.target.value })}
					placeholder="starter"
					disabled={disabled}
				/>
			</div>
			<div className="space-y-2">
				<Label>Purpose</Label>
				<Select
					value={value.purpose}
					onValueChange={(v) => onChange({ ...value, purpose: v })}
					disabled={disabled}
				>
					<SelectTrigger>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="SHARED_TEMPLATE">
							Shared template (forks per user)
						</SelectItem>
						<SelectItem value="REFERENCE">Reference (read-only)</SelectItem>
						<SelectItem value="PLAYGROUND">Playground (forks)</SelectItem>
					</SelectContent>
				</Select>
			</div>
		</div>
	);
}
