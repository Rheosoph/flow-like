import {
	type IBit,
	type IBitModelClassification,
	type ILlmParameters,
	type IModelProvider,
	Input,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Slider,
	humanFileSize,
} from "@flow-like/flow-like-ui";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@flow-like/flow-like-ui";
import { Label } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import type { Dispatch, SetStateAction } from "react";

const PROVIDER_OPTIONS = [
	"Local",
	"MLX",
	"Premium",
	"Internal",
	"Hosted",
	"hosted:openrouter",
	"hosted:openai",
	"hosted:anthropic",
	"hosted:bedrock",
	"hosted:azure",
	"hosted:vertex",
] as const;

function isHostedProviderName(providerName?: null | string) {
	const normalized = providerName?.trim().toLowerCase() ?? "";
	return (
		normalized === "premium" ||
		normalized === "internal" ||
		normalized === "hosted" ||
		normalized.startsWith("hosted:")
	);
}

function getProviderParams(provider: IModelProvider | undefined) {
	const params = provider?.params;
	if (!params || typeof params !== "object" || Array.isArray(params)) {
		return {} as Record<string, unknown>;
	}
	return params as Record<string, unknown>;
}

export function LLMConfiguration({
	bit,
	setBit,
	isHosted = false,
}: { bit: IBit; setBit: Dispatch<SetStateAction<IBit>>; isHosted?: boolean }) {
	const { t } = useTranslation("common");
	const parameters = bit.parameters as ILlmParameters;
	const providerParams = getProviderParams(parameters?.provider);
	const isHostedProvider = isHostedProviderName(
		parameters?.provider?.provider_name,
	);

	const updateParameter = (key: keyof ILlmParameters, value: unknown) => {
		setBit((old) => ({
			...old,
			parameters: {
				...old.parameters,
				[key]: value,
			},
		}));
	};

	const updateClassification = (
		key: keyof IBitModelClassification,
		value: number,
	) => {
		updateParameter("model_classification", {
			...parameters.model_classification,
			[key]: value,
		});
	};

	const updateProvider = (key: keyof IModelProvider, value: string | null) => {
		updateParameter("provider", {
			...parameters.provider,
			[key]: value,
		});
	};

	const updateProviderParams = (nextParams: Record<string, unknown>) => {
		updateParameter("provider", {
			...parameters.provider,
			params: nextParams,
		});
	};

	return (
		<div className="space-y-6 w-full max-w-screen-lg">
			<Card className="w-full">
				<CardHeader className="w-full">
					<CardTitle className="flex items-center justify-between w-full">
						<p>{t("llmConfiguration", "LLM Configuration")}</p>
						{bit.size ? (
							<small className="font-normal text-muted-foreground">
								{humanFileSize(bit.size)}
							</small>
						) : null}
					</CardTitle>
					<CardDescription>
						{t(
							"configureModelContextAndProcessingCapabilities",
							"Configure model context and processing capabilities",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="context-length">
							{t("contextLength", "Context Length")}
						</Label>
						<Input
							id="context-length"
							type="number"
							value={parameters?.context_length || 2048}
							onChange={(e) =>
								updateParameter(
									"context_length",
									Number.parseInt(e.target.value) || 2048,
								)
							}
							placeholder="2048"
							min="1"
							max="2000000"
						/>
						<p className="text-xs text-muted-foreground">
							{t(
								"maximumNumberOfTokensTheModelCanProcess",
								"Maximum number of tokens the model can process",
							)}
						</p>
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>{t("providerSettings", "Provider Settings")}</CardTitle>
					<CardDescription>
						{t(
							"configureTheModelProviderAndIdentification",
							"Configure the model provider and identification",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
						<div className="space-y-2">
							<Label htmlFor="provider-name">
								{t("providerName", "Provider Name *")}
							</Label>
							<Select
								value={parameters?.provider?.provider_name || "Local"}
								onValueChange={(value) =>
									updateParameter("provider", {
										...parameters.provider,
										provider_name: value,
										params: isHostedProviderName(value)
											? typeof providerParams.tier === "string"
												? { tier: providerParams.tier }
												: {}
											: getProviderParams(parameters?.provider),
									})
								}
							>
								<SelectTrigger id="provider-name">
									<SelectValue
										placeholder={t("selectProvider", "Select provider")}
									/>
								</SelectTrigger>
								<SelectContent>
									{PROVIDER_OPTIONS.map((providerName) => (
										<SelectItem key={providerName} value={providerName}>
											{providerName}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>

						<div className="space-y-2">
							<Label htmlFor="model-id">{t("modelId", "Model ID")}</Label>
							<Input
								id="model-id"
								value={parameters?.provider?.model_id || ""}
								onChange={(e) =>
									updateProvider("model_id", e.target.value || null)
								}
								placeholder={t(
									"optionalModelIdentifier",
									"Optional model identifier",
								)}
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="version">{t("version", "Version")}</Label>
							<Input
								id="version"
								value={parameters?.provider?.version || ""}
								onChange={(e) =>
									updateProvider("version", e.target.value || null)
								}
								placeholder={t("optionalVersion", "Optional version")}
							/>
						</div>
					</div>

					{isHostedProvider ? (
						<div className="grid grid-cols-1 gap-4 md:grid-cols-2">
							<div className="space-y-2">
								<Label htmlFor="provider-tier">{t("tier", "Tier")}</Label>
								<Input
									id="provider-tier"
									value={
										typeof providerParams.tier === "string"
											? providerParams.tier
											: ""
									}
									onChange={(e) =>
										updateProviderParams(
											e.target.value.trim()
												? { tier: e.target.value.trim() }
												: {},
										)
									}
									placeholder={t("optionalAccessTier", "Optional access tier")}
								/>
								<p className="text-xs text-muted-foreground">
									{t(
										"optionalEntitlementMetadataStoredUnderProviderParams",
										"Optional entitlement metadata stored under provider params. The server controls the upstream endpoint.",
									)}
								</p>
							</div>
						</div>
					) : null}
				</CardContent>
			</Card>

			{isHosted ? (
				<Card>
					<CardHeader>
						<CardTitle>
							{t("modelClassification", "Model Classification")}
						</CardTitle>
						<CardDescription>
							{`Capability scores are automatically computed from the model slug — no manual configuration needed.`}
						</CardDescription>
					</CardHeader>
				</Card>
			) : (
				<Card>
					<CardHeader>
						<CardTitle>
							{t("modelClassification", "Model Classification")}
						</CardTitle>
						<CardDescription>
							{`Rate each capability from 0.0 (poor) to 1.0 (excellent)`}
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
							{Object.entries(parameters?.model_classification || {}).map(
								([key, value]) => {
									if (typeof value !== "number") return null;
									const label = key
										.replace(/_/g, " ")
										.replace(/\b\w/g, (l) => l.toUpperCase());
									return (
										<div key={key} className="space-y-2">
											<div className="flex justify-between items-center">
												<Label htmlFor={key}>{label}</Label>
												<span className="text-sm text-muted-foreground">
													{value.toFixed(1)}
												</span>
											</div>
											<Slider
												id={key}
												min={0}
												max={1}
												step={0.1}
												value={[value]}
												onValueChange={(val) =>
													updateClassification(
														key as keyof IBitModelClassification,
														val[0],
													)
												}
											/>
											<div className="flex justify-between text-xs text-muted-foreground">
												<span>{t("poor", "Poor")}</span>
												<span>{t("excellent", "Excellent")}</span>
											</div>
										</div>
									);
								},
							)}
						</div>
					</CardContent>
				</Card>
			)}
		</div>
	);
}
