"use client";

import { useId } from "react";
import {
	HOSTED_CLOUDFLARE_IMAGE_MODELS,
	HOSTED_IMAGE_PROVIDERS,
	updateHostedImageTier,
	validateHostedImageParameters,
} from "../../lib/bit/hosted-image-generation";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { record, updateBitPricingField } from "./bit-editor-model";
import { HostedPricingField } from "./hosted-pricing-field";

export function HostedImageGenerationConfiguration({
	parameters,
	onChange,
}: {
	parameters: Record<string, unknown>;
	onChange: (parameters: Record<string, unknown>) => void;
}) {
	const id = useId();
	const provider = record(parameters.provider);
	const name = String(provider.provider_name ?? "");
	const settings = record(provider.params);
	const openRouter = name === "hosted:openrouter";
	const error = validateHostedImageParameters(parameters);
	return (
		<div className="space-y-4 rounded-xl border p-4">
			<div>
				<h3 className="font-medium">Hosted image generation</h3>
				<p className="mt-1 text-sm text-muted-foreground">
					Generate images through the server using the selected provider. No
					model files are downloaded.
				</p>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				<div className="space-y-2">
					<Label htmlFor={`${id}-provider`}>Provider</Label>
					<select
						id={`${id}-provider`}
						className="h-9 w-full rounded-md border bg-background px-3 text-sm"
						value={name}
						onChange={(event) =>
							onChange({
								...parameters,
								provider: {
									provider_name: event.target.value,
									model_id:
										event.target.value === "hosted:cloudflare"
											? HOSTED_CLOUDFLARE_IMAGE_MODELS[0].value
											: "",
									params:
										typeof settings.tier === "string" && settings.tier.trim()
											? { tier: settings.tier }
											: {},
								},
								pricing: {},
							})
						}
					>
						{!HOSTED_IMAGE_PROVIDERS.some(
							(option) => option.value === name,
						) && <option value={name}>{name || "Choose a provider"}</option>}
						{HOSTED_IMAGE_PROVIDERS.map((option) => (
							<option key={option.value} value={option.value}>
								{option.label}
							</option>
						))}
					</select>
				</div>
				<div className="space-y-2">
					<Label htmlFor={`${id}-model`}>Model ID</Label>
					{!openRouter ? (
						<select
							id={`${id}-model`}
							className="h-9 w-full rounded-md border bg-background px-3 text-sm"
							value={String(provider.model_id ?? "")}
							onChange={(event) =>
								onChange({
									...parameters,
									provider: { ...provider, model_id: event.target.value },
									pricing: {},
								})
							}
						>
							{!HOSTED_CLOUDFLARE_IMAGE_MODELS.some(
								(model) => model.value === provider.model_id,
							) && (
								<option value={String(provider.model_id ?? "")}>
									{String(provider.model_id ?? "Choose a model")}
								</option>
							)}
							{HOSTED_CLOUDFLARE_IMAGE_MODELS.map((model) => (
								<option key={model.value} value={model.value}>
									{model.label}
								</option>
							))}
						</select>
					) : (
						<Input
							id={`${id}-model`}
							maxLength={256}
							value={String(provider.model_id ?? "")}
							placeholder="google/gemini-2.5-flash-image"
							onChange={(event) =>
								onChange({
									...parameters,
									provider: { ...provider, model_id: event.target.value },
									pricing: {},
								})
							}
						/>
					)}
				</div>
				<div className="space-y-2">
					<Label htmlFor={`${id}-tier`}>Access tier</Label>
					<Input
						id={`${id}-tier`}
						value={String(settings.tier ?? "")}
						placeholder="Server default (ENTERPRISE)"
						onChange={(event) =>
							onChange(updateHostedImageTier(parameters, event.target.value))
						}
					/>
				</div>
			</div>
			<HostedPricingField
				label={openRouter ? "Maximum USD per image" : "USD per image"}
				value={record(parameters.pricing).request_micro_usd}
				optional={false}
				onChange={(text) =>
					onChange(updateBitPricingField(parameters, "request_micro_usd", text))
				}
			/>
			<p className="text-xs text-muted-foreground">
				{openRouter
					? "Set the highest provider cost across all permitted image settings and routed providers to reserve credits before each image. The final charge uses the cost reported by OpenRouter."
					: "Set the provider cost for each generated image. Use a model and generation settings covered by this price."}{" "}
				Enter 0 only when the provider cost is zero. Serving fees still apply.
			</p>
			<p className="text-xs text-muted-foreground">
				{openRouter
					? "Configure OPENROUTER_API_KEY on the server."
					: "Configure HOSTED_CLOUDFLARE_API_TOKEN and HOSTED_CLOUDFLARE_ACCOUNT_ID on the server."}
			</p>
			{!openRouter && (
				<p className="text-xs text-muted-foreground">
					Cloudflare supports FLUX.1 Schnell and Stable Diffusion XL Base 1.0.
					FLUX.1 Schnell uses its default square size. SDXL also accepts size,
					aspect ratio and negative prompt options. Each request generates one
					image.
				</p>
			)}
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}
