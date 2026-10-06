"use client";

import { useId } from "react";
import {
	SYSTEMONE_CUSTOM_PROVIDERS,
	SYSTEMONE_HOSTED_PROVIDERS,
} from "../../lib/bit/systemone-model";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { record } from "./bit-editor-model";

export function SystemOneConfiguration({
	parameters,
	onChange,
	scope = "admin",
}: {
	parameters: Record<string, unknown>;
	onChange: (parameters: Record<string, unknown>) => void;
	scope?: "admin" | "custom";
}) {
	const id = useId();
	const provider = record(parameters.provider);
	const settings = record(provider.params);
	const name = String(provider.provider_name ?? "Local");
	const hosted = name.toLowerCase().startsWith("hosted:");
	const custom = name.startsWith("custom:");
	const device = name === "device";
	const options = [
		{ value: "Local", label: "Local GGUF (llama.cpp)" },
		...(scope === "admin"
			? SYSTEMONE_HOSTED_PROVIDERS
			: SYSTEMONE_CUSTOM_PROVIDERS),
	];
	const updateProvider = (patch: Record<string, unknown>) =>
		onChange({ ...parameters, provider: { ...provider, ...patch } });
	const updateSetting = (key: string, value: string) =>
		updateProvider({ params: { ...settings, [key]: value } });
	return (
		<div className="space-y-4 rounded-xl border p-4">
			<div>
				<h3 className="font-medium">SystemOne decisions</h3>
				<p className="mt-1 text-sm text-muted-foreground">
					Answer typed choice, score, and yes/no questions about a state. Use
					this model with the SystemOne node.
				</p>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				<div className="space-y-2">
					<Label htmlFor={`${id}-provider`}>Provider</Label>
					<select
						id={`${id}-provider`}
						className="h-9 w-full rounded-md border bg-background px-3 text-sm"
						value={name}
						disabled={device}
						onChange={(event) =>
							onChange({
								...parameters,
								provider: {
									provider_name: event.target.value,
									model_id: provider.model_id ?? null,
								},
							})
						}
					>
						{!options.some((option) => option.value === name) && (
							<option value={name}>{name}</option>
						)}
						{options.map((option) => (
							<option key={option.value} value={option.value}>
								{option.label}
							</option>
						))}
					</select>
				</div>
				<div className="space-y-2">
					<Label htmlFor={`${id}-context`}>Context length</Label>
					<Input
						id={`${id}-context`}
						type="number"
						min={1}
						max={4294967295}
						step={1}
						value={
							typeof parameters.context_length === "number"
								? parameters.context_length
								: ""
						}
						onChange={(event) =>
							onChange({
								...parameters,
								context_length: Number(event.target.value),
							})
						}
					/>
				</div>
				{(hosted || custom) && (
					<div className="space-y-2">
						<Label htmlFor={`${id}-model`}>Model ID</Label>
						<Input
							id={`${id}-model`}
							value={String(provider.model_id ?? "")}
							onChange={(event) =>
								updateProvider({ model_id: event.target.value })
							}
						/>
					</div>
				)}
				{device && (
					<p className="text-xs text-muted-foreground">
						This model runs on its configured device. Manage the connection in
						Devices.
					</p>
				)}
				{hosted && (
					<div className="space-y-2">
						<Label htmlFor={`${id}-tier`}>Access tier</Label>
						<Input
							id={`${id}-tier`}
							placeholder="Server default"
							value={String(settings.tier ?? "")}
							onChange={(event) => updateSetting("tier", event.target.value)}
						/>
					</div>
				)}
				{custom && (
					<>
						<div className="space-y-2">
							<Label htmlFor={`${id}-endpoint`}>
								{name === "custom:systemone"
									? "Endpoint URL"
									: "Endpoint URL (optional)"}
							</Label>
							<Input
								id={`${id}-endpoint`}
								type="url"
								value={String(settings.endpoint ?? "")}
								onChange={(event) =>
									updateSetting("endpoint", event.target.value)
								}
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor={`${id}-key`}>API key (optional)</Label>
							<Input
								id={`${id}-key`}
								type="password"
								autoComplete="new-password"
								value={String(settings.api_key ?? "")}
								onChange={(event) =>
									updateSetting("api_key", event.target.value)
								}
							/>
						</div>
					</>
				)}
			</div>
			{hosted && (
				<p className="text-xs text-muted-foreground">
					The server supplies the hosted endpoint and credentials.
				</p>
			)}
			{name.toLowerCase() === "local" && (
				<p className="text-xs text-muted-foreground">
					Requires a SystemOne GGUF model and a desktop llama.cpp runtime with
					SystemOne support.
				</p>
			)}
		</div>
	);
}
