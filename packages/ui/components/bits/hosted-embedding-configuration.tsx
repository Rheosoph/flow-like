"use client";

import { useTranslation } from "@flow-like/locales";
import { useId } from "react";
import {
	IRemoteEmbeddingProvider,
	type IRemoteExecutionConfig,
} from "../../lib/schema/bit/bit/embedding-model-parameters";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Switch } from "../ui/switch";
import {
	record,
	updateBitPricingField,
	validateHostedEmbeddingParameters,
} from "./bit-editor-model";
import { HostedPricingField } from "./hosted-pricing-field";

const PROVIDERS = [
	{ value: IRemoteEmbeddingProvider.Internal, label: "Internal gateway" },
	{
		value: IRemoteEmbeddingProvider.CloudflareWorkersAI,
		label: "Cloudflare Workers AI",
	},
	{ value: IRemoteEmbeddingProvider.OpenAI, label: "OpenAI" },
	{ value: IRemoteEmbeddingProvider.OpenRouter, label: "OpenRouter" },
	{ value: IRemoteEmbeddingProvider.Cohere, label: "Cohere" },
	{ value: IRemoteEmbeddingProvider.VoyageAI, label: "Voyage AI" },
	{ value: IRemoteEmbeddingProvider.AzureOpenAI, label: "Azure OpenAI" },
	{
		value: IRemoteEmbeddingProvider.HuggingfaceEndpoint,
		label: "Hugging Face endpoint",
	},
	{
		value: IRemoteEmbeddingProvider.OpenAICompatible,
		label: "OpenAI-compatible endpoint",
	},
];

export function HostedEmbeddingConfiguration({
	parameters,
	onChange,
}: {
	parameters: Record<string, unknown>;
	onChange: (parameters: Record<string, unknown>) => void;
}) {
	const { t } = useTranslation("common");
	const id = useId();
	const remote = parameters.remote as IRemoteExecutionConfig | null | undefined;
	const setRemote = (next: IRemoteExecutionConfig | null) =>
		onChange({ ...parameters, remote: next });
	const implementation =
		remote?.implementation ?? IRemoteEmbeddingProvider.Internal;
	const isOpenRouter = implementation === IRemoteEmbeddingProvider.OpenRouter;
	const update = (updates: Partial<IRemoteExecutionConfig>) => {
		setRemote({ ...remote, ...updates });
	};

	const supportsEndpoint = ![
		IRemoteEmbeddingProvider.CloudflareWorkersAI,
		IRemoteEmbeddingProvider.OpenAI,
		IRemoteEmbeddingProvider.OpenRouter,
		IRemoteEmbeddingProvider.Cohere,
		IRemoteEmbeddingProvider.VoyageAI,
	].includes(implementation);
	const showPricing = implementation !== IRemoteEmbeddingProvider.Internal;
	const pricing = record(parameters.pricing);
	const bytePricing =
		!isOpenRouter &&
		Object.hasOwn(pricing, "input_micro_usd_per_million_bytes");
	const priceKey = bytePricing
		? "input_micro_usd_per_million_bytes"
		: "input_micro_usd_per_million_tokens";
	const error = validateHostedEmbeddingParameters(parameters);
	const cloudflareBatchLimit =
		implementation === IRemoteEmbeddingProvider.CloudflareWorkersAI
			? remote?.model_id?.trim() === "@cf/qwen/qwen3-embedding-0.6b"
				? 32
				: [
							"@cf/google/embeddinggemma-300m",
							"@cf/baai/bge-large-en-v1.5",
						].includes(remote?.model_id?.trim() ?? "")
					? 100
					: undefined
			: undefined;

	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("hostedEmbeddings", "Hosted embeddings")}</CardTitle>
				<CardDescription>
					{t(
						"hostedEmbeddingsDescription",
						"Choose the service that runs this model through the Flow-Like server. Local model settings remain available for on-device use.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="flex items-center gap-3">
					<Switch
						id={`${id}-enabled`}
						checked={remote != null}
						onCheckedChange={(enabled) =>
							onChange({
								...parameters,
								remote: enabled
									? { implementation: IRemoteEmbeddingProvider.Internal }
									: null,
								pricing: undefined,
							})
						}
					/>
					<Label htmlFor={`${id}-enabled`}>
						{t("enableHostedEmbeddings", "Enable hosted embeddings")}
					</Label>
				</div>
				{remote != null && (
					<>
						<div className="grid gap-4 sm:grid-cols-2">
							<div className="space-y-2">
								<Label htmlFor={`${id}-provider`}>
									{t("hostedEmbeddingProvider", "Hosted provider")}
								</Label>
								<Select
									value={implementation}
									onValueChange={(value) => {
										if (
											value !== implementation &&
											PROVIDERS.some((provider) => provider.value === value)
										) {
											onChange({
												...parameters,
												remote: {
													implementation: value as IRemoteEmbeddingProvider,
												},
												pricing: undefined,
											});
										}
									}}
								>
									<SelectTrigger id={`${id}-provider`}>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										{PROVIDERS.map((provider) => (
											<SelectItem key={provider.value} value={provider.value}>
												{provider.label}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
							<div className="space-y-2">
								<Label htmlFor={`${id}-model`}>
									{t(
										"hostedEmbeddingModelId",
										"Upstream model or deployment ID",
									)}
								</Label>
								<Input
									id={`${id}-model`}
									value={remote.model_id ?? ""}
									onChange={(event) =>
										update({ model_id: event.target.value || null })
									}
									placeholder={
										implementation ===
										IRemoteEmbeddingProvider.CloudflareWorkersAI
											? "@cf/qwen/qwen3-embedding-0.6b"
											: implementation === IRemoteEmbeddingProvider.OpenRouter
												? "google/gemini-embedding-2"
												: undefined
									}
								/>
							</div>
						</div>
						{isOpenRouter && (
							<div className="space-y-2">
								<div className="flex items-center gap-3">
									<Switch
										id={`${id}-dimensions`}
										aria-describedby={`${id}-dimensions-help`}
										checked={remote.supports_dimensions ?? true}
										onCheckedChange={(supports_dimensions) =>
											update({ supports_dimensions })
										}
									/>
									<Label htmlFor={`${id}-dimensions`}>
										{t(
											"openRouterRequestVectorDimensions",
											"Request vector dimensions",
										)}
									</Label>
								</div>
								<p
									id={`${id}-dimensions-help`}
									className="text-sm text-muted-foreground"
								>
									{t(
										"openRouterRequestVectorDimensionsHelp",
										"When enabled, the server requests this Bit's vector length. Turn off for models with a fixed output size and set the Bit's vector length to the model's native size. Returned vectors must match the Bit's vector length.",
									)}
								</p>
							</div>
						)}
						{cloudflareBatchLimit && (
							<p className="text-sm text-muted-foreground">
								{t(
									"cloudflareEmbeddingBatchLimit",
									"This Cloudflare model accepts at most {{count}} inputs per request.",
									{ count: cloudflareBatchLimit },
								)}
							</p>
						)}
						{implementation === IRemoteEmbeddingProvider.CloudflareWorkersAI &&
							remote.model_id?.trim() === "@cf/google/embeddinggemma-2" && (
								<div className="space-y-2 text-sm text-muted-foreground">
									<p>
										{t(
											"cloudflareGemma2EmbeddingCapabilities",
											"EmbeddingGemma 2 accepts text, images, audio and video. Set 128, 256, 512 or 768 vector dimensions and an input length of at most 8192 tokens. The server requests the vector length configured for this Bit; the model's default is 768.",
										)}
									</p>
									<p>
										{t(
											"cloudflareGemma2EmbeddingPrefixes",
											'For search, set the query prefix to "task: search result | query: " and the paragraph prefix to "title: none | text: ", including the trailing spaces. Prefixes apply to text.',
										)}
									</p>
								</div>
							)}
						{implementation === IRemoteEmbeddingProvider.OpenRouter &&
							remote.model_id?.trim() === "google/gemini-embedding-2" && (
								<p className="text-sm text-muted-foreground">
									{t(
										"openRouterGeminiEmbeddingCapabilities",
										"Gemini Embedding 2 accepts text, images, audio, video and PDF files. Set 128 to 3072 vector dimensions and an input length of at most 8192 tokens. The returned vector length must match this Bit's configuration.",
									)}
								</p>
							)}
						{implementation === IRemoteEmbeddingProvider.Cohere &&
							remote.model_id?.trim() === "embed-v4.0" && (
								<p className="text-sm text-muted-foreground">
									{t(
										"cohereEmbeddingDimensions",
										"Cohere embed-v4.0 supports 256, 512, 1024 and 1536 vector dimensions. The server requests the vector length configured for this Bit.",
									)}
								</p>
							)}
						<p
							id={`${id}-secrets-help`}
							className="text-sm text-muted-foreground"
						>
							{t(
								"hostedEmbeddingSecretNamesHelp",
								"Enter names of secrets configured on the server. Leave them blank to use the provider defaults. Secret values stay on the server.",
							)}
						</p>
						<div className="grid gap-4 sm:grid-cols-2">
							<div className="space-y-2">
								<Label htmlFor={`${id}-secret`}>
									{t("hostedEmbeddingApiSecretName", "API key secret name")}
								</Label>
								<Input
									id={`${id}-secret`}
									aria-describedby={`${id}-secrets-help`}
									autoComplete="off"
									value={remote.secret_name ?? ""}
									placeholder={
										implementation === IRemoteEmbeddingProvider.OpenRouter
											? "OPENROUTER_API_KEY"
											: undefined
									}
									onChange={(event) =>
										update({ secret_name: event.target.value || null })
									}
								/>
							</div>
							{supportsEndpoint && (
								<div className="space-y-2">
									<Label htmlFor={`${id}-endpoint`}>
										{t(
											"hostedEmbeddingEndpointSecretName",
											"Endpoint secret name",
										)}
									</Label>
									<Input
										id={`${id}-endpoint`}
										aria-describedby={`${id}-secrets-help`}
										autoComplete="off"
										value={remote.endpoint_secret_name ?? ""}
										onChange={(event) =>
											update({
												endpoint_secret_name: event.target.value || null,
											})
										}
									/>
								</div>
							)}
						</div>
						{!showPricing && (
							<div className="space-y-2">
								<p className="text-sm text-muted-foreground">
									{t(
										"internalEmbeddingPricingHelp",
										"The internal gateway uses the tariff configured on the server.",
									)}
								</p>
								{parameters.pricing != null && (
									<Button
										type="button"
										variant="outline"
										onClick={() => {
											onChange({ ...parameters, pricing: undefined });
										}}
									>
										{t("removeUnusedEmbeddingPricing", "Remove unused pricing")}
									</Button>
								)}
							</div>
						)}
						{showPricing && (
							<div className="space-y-4 border-t pt-4">
								<div>
									<h4 className="text-sm font-medium">
										{t("hostedEmbeddingPricing", "Hosted embedding pricing")}
									</h4>
									<p className="mt-1 text-sm text-muted-foreground">
										{isOpenRouter
											? t(
													"openRouterEmbeddingPricingHelp",
													"Set the highest input token rate among this model's modalities and routed providers. This rate reserves credits before a request; the final charge uses the cost reported by OpenRouter, plus any request fee.",
												)
											: t(
													"hostedEmbeddingPricingHelp",
													"Set the rate charged for hosted embedding requests. Use 0 for a free input rate.",
												)}
									</p>
								</div>
								{!isOpenRouter && (
									<div className="space-y-2">
										<Label htmlFor={`${id}-pricing-unit`}>
											{t("hostedEmbeddingPriceUnit", "Input price unit")}
										</Label>
										<select
											id={`${id}-pricing-unit`}
											className="h-10 w-full rounded-md border bg-background px-3 text-sm"
											value={bytePricing ? "bytes" : "tokens"}
											onChange={(event) => {
												const {
													input_micro_usd_per_million_tokens: _tokens,
													input_micro_usd_per_million_bytes: _bytes,
													max_input_bytes: _limit,
													...rest
												} = pricing;
												onChange({
													...parameters,
													pricing: {
														...rest,
														[event.target.value === "bytes"
															? "input_micro_usd_per_million_bytes"
															: "input_micro_usd_per_million_tokens"]: "",
													},
												});
											}}
										>
											<option value="tokens">
												{t("hostedEmbeddingTokens", "Provider-reported tokens")}
											</option>
											<option value="bytes">
												{t("hostedEmbeddingBytes", "Input bytes")}
											</option>
										</select>
									</div>
								)}
								{!isOpenRouter && (
									<p className="text-sm text-muted-foreground">
										{t(
											"hostedEmbeddingPriceUnitHelp",
											"Token pricing requires the provider to report token usage. Choose input bytes when the provider does not report it.",
										)}
									</p>
								)}
								<div className="grid gap-4 sm:grid-cols-2">
									<HostedPricingField
										label={
											isOpenRouter
												? "Maximum USD per 1M input tokens"
												: bytePricing
													? "USD per 1M input bytes"
													: "USD per 1M input tokens"
										}
										value={pricing[priceKey]}
										optional={false}
										onChange={(text) => {
											const next = updateBitPricingField(
												parameters,
												priceKey,
												text,
											);
											if (isOpenRouter) {
												const {
													input_micro_usd_per_million_bytes: _bytes,
													max_input_bytes: _limit,
													...tokenPricing
												} = record(next.pricing);
												next.pricing = tokenPricing;
											}
											if (!text.trim())
												next.pricing = {
													...record(next.pricing),
													[priceKey]: "",
												};
											onChange(next);
										}}
									/>
									<HostedPricingField
										label="USD per request"
										value={pricing.request_micro_usd}
										optional
										onChange={(text) =>
											onChange(
												updateBitPricingField(
													parameters,
													"request_micro_usd",
													text,
												),
											)
										}
									/>
								</div>
								{bytePricing && (
									<div className="space-y-2">
										<Label htmlFor={`${id}-byte-limit`}>
											{t(
												"hostedEmbeddingByteLimit",
												"Maximum input bytes per batch",
											)}
										</Label>
										<Input
											id={`${id}-byte-limit`}
											type="number"
											min={1}
											step={1}
											value={String(pricing.max_input_bytes ?? "")}
											onChange={(event) =>
												onChange({
													...parameters,
													pricing: {
														...pricing,
														max_input_bytes:
															event.target.value === ""
																? ""
																: Number(event.target.value),
													},
												})
											}
										/>
									</div>
								)}
							</div>
						)}
						{error && (
							<p role="alert" className="text-sm text-destructive">
								{error}
							</p>
						)}
					</>
				)}
			</CardContent>
		</Card>
	);
}
