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

export function LLMConfiguration({
	bit,
	setBit,
}: { bit: IBit; setBit: Dispatch<SetStateAction<IBit>> }) {
	const { t } = useTranslation("common");
	const parameters = bit.parameters as ILlmParameters;

	const updateParameter = (key: keyof ILlmParameters, value: any) => {
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

	return (
		<div className="space-y-6 w-full max-w-screen-lg">
			<Card className="w-full">
				<CardHeader className="w-full">
					<CardTitle className="flex items-center justify-between w-full">
						<p>{t("llmConfiguration", "LLM Configuration")}</p>
						{bit.size && (
							<small className="font-normal text-muted-foreground">
								{humanFileSize(bit.size)}
							</small>
						)}
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
									updateProvider("provider_name", value)
								}
							>
								<SelectTrigger id="provider-name">
									<SelectValue
										placeholder={t("selectProvider", "Select provider")}
									/>
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="Local">{t("local", "Local")}</SelectItem>
									<SelectItem value="MLX">MLX</SelectItem>
									<SelectItem value="Premium">
										{t("premium", "Premium")}
									</SelectItem>
								</SelectContent>
							</Select>
						</div>

						<div className="space-y-2">
							<Label htmlFor="model-id">{t("modelId", "Model ID")}</Label>
							<Input
								disabled={parameters?.provider?.provider_name === "Local"}
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
								disabled={parameters?.provider?.provider_name === "Local"}
								id="version"
								value={parameters?.provider?.version || ""}
								onChange={(e) =>
									updateProvider("version", e.target.value || null)
								}
								placeholder={t("optionalVersion", "Optional version")}
							/>
						</div>
					</div>
				</CardContent>
			</Card>

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
		</div>
	);
}
