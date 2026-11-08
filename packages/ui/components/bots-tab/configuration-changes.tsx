import { useTranslation } from "@flow-like/locales";

const FIELD_LABELS: Record<string, string> = {
	name: "Name",
	description: "Description",
	instructions: "System prompt",
	capabilities: "Capabilities",
	model: "Chat models",
	embedding_model: "Embedding model",
	stt_model: "Speech recognition model",
	tts_model: "Speech model",
	image_model: "Image model",
	vision_model: "Vision model",
	subtask_model: "Subtask model",
};

/** Show every proposed value before consent, including long prompts and model mappings. */
export function BotConfigurationChanges({
	changes,
}: {
	changes: Record<string, unknown>;
}) {
	const { t } = useTranslation("bots");
	return (
		<section className="space-y-2" data-slot="bot-configuration-changes">
			<p className="text-sm font-medium">
				{t("crew.approval.configurationChanges", "Proposed changes")}
			</p>
			<dl className="max-h-72 space-y-3 overflow-auto rounded-lg border border-border/60 bg-muted/30 p-3">
				{Object.entries(changes).map(([field, value]) => (
					<div key={field}>
						<dt className="mb-1 text-xs font-medium text-muted-foreground">
							{String(
								t(
									`crew.approval.fields.${field}`,
									FIELD_LABELS[field] ?? field,
								),
							)}
						</dt>
						<dd className="whitespace-pre-wrap break-words text-sm [overflow-wrap:anywhere]">
							{value === null
								? t("crew.approval.clearValue", "Clear value")
								: typeof value === "string"
									? value
									: JSON.stringify(value, null, 2)}
						</dd>
					</div>
				))}
			</dl>
		</section>
	);
}
