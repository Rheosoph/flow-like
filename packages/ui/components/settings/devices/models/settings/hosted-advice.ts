import {
	ENGINE_FIELDS,
	type SettingsAdvice,
	type SettingsField,
	flashForCache,
	recommendSettings,
} from "../../../../../lib/device-management/model/models/fit";
import type {
	HostedModel,
	ModelSettings,
	ModelsOverview,
	Recommendation,
	Residency,
} from "../../../../../lib/device-management/models";
import type { DevicesT } from "../../primitives/area-context";
import type { DiffRow } from "../../primitives/diff-rows";
import { residencyLabel } from "../models-copy";
import { recommendationCopy } from "../recommendation-copy";
import {
	type ReasonContext,
	adviceReason,
	fieldLabel,
	valueLabel,
} from "./settings-copy";
import {
	type SettingsAdviceView,
	sameResidency,
	sameValue,
} from "./settings-form";

/*
 * Recommended settings of a model the device hosts. Its file sizes aren't
 * known here, so the baseline is the engine's best practice on this
 * hardware (plan §3.4); where the device itself recommends a change for the
 * model (a `configure` fix, plan §3.6), that value and its sentence win.
 * Changes compare what the model stores: a field it leaves out is the
 * device's default, and setting it is a change.
 */

type ConfigureFix = Extract<
	NonNullable<Recommendation["fix"]>,
	{ kind: "configure" }
>;

/** The device's own `configure` fix for the model, if one of the listed recommendations carries it. */
function deviceFix(model: HostedModel, overview: ModelsOverview) {
	for (const recommendation of overview.recommendations) {
		const fix = recommendation.fix;
		if (recommendation.model_id === model.id && fix?.kind === "configure")
			return { recommendation, fix: fix as ConfigureFix };
	}
	return undefined;
}

/** Text of the advice's reason for each field. */
function reasonTexts(
	t: DevicesT,
	advice: SettingsAdvice,
	context: ReasonContext,
) {
	const reasons: Partial<Record<SettingsField, string>> = {};
	for (const [field, reason] of Object.entries(advice.reasons))
		if (reason)
			reasons[field as SettingsField] = adviceReason(t, reason, context);
	return reasons;
}

/** The form's view of fresh advice: its values are the recommendation. */
export function adviceView(
	t: DevicesT,
	advice: SettingsAdvice,
	context: ReasonContext,
) {
	const view: SettingsAdviceView = {
		recommended: advice.settings,
		reasons: reasonTexts(t, advice, context),
		residency: advice.residency,
		residencyReason: adviceReason(t, advice.residencyReason, context),
	};
	return view;
}

/** Fields the fix changes on the model, with the fix's values; one it leaves out goes back to the device default. */
function fixChanges(model: HostedModel, fix: ConfigureFix) {
	const changes: Record<string, unknown> = {};
	for (const field of ENGINE_FIELDS[model.engine])
		if (!sameValue(fix.settings[field], model.settings[field]))
			changes[field] = fix.settings[field];
	return changes as ModelSettings;
}

export interface HostedAdviceContext {
	device: string;
}

export function hostedAdvice(
	t: DevicesT,
	model: HostedModel,
	overview: ModelsOverview,
	context: HostedAdviceContext,
) {
	const base = recommendSettings(
		{ kind: model.kind, engine: model.engine },
		overview,
	);
	const reasons = reasonTexts(t, base, {
		cores: overview.system.cpu.physical_cores,
	});
	const found = deviceFix(model, overview);
	const changes = found ? fixChanges(model, found.fix) : {};
	const sentence = found
		? recommendationCopy(t, found.recommendation, {
				device: context.device,
				modelName: (id) => (id === model.id ? model.display_name : undefined),
			}).sentence
		: "";
	for (const field of Object.keys(changes))
		reasons[field as SettingsField] = sentence;
	const fixResidency =
		found && !sameResidency(found.fix.residency, model.residency)
			? found.fix.residency
			: undefined;
	const view: SettingsAdviceView = {
		recommended: { ...base.settings, ...changes },
		reasons,
		residency: fixResidency ?? base.residency,
		residencyReason: fixResidency
			? sentence
			: adviceReason(t, base.residencyReason, { cores: 0 }),
	};
	return view;
}

/** What Configure changes, field by field, in the words the sheet uses. */
export function settingsChanges(
	t: DevicesT,
	model: HostedModel,
	change: { settings: ModelSettings; residency: Residency },
) {
	const rows: DiffRow[] = [];
	for (const field of ENGINE_FIELDS[model.engine])
		if (!sameValue(model.settings[field], change.settings[field]))
			rows.push({
				kind: "changed",
				label: fieldLabel(t, field),
				before: valueLabel(t, field, model.settings),
				after: valueLabel(t, field, change.settings),
			});
	if (!sameResidency(model.residency, change.residency))
		rows.push({
			kind: "changed",
			label: t("devices:models.settings.residency.label", "When it runs"),
			before: residencyLabel(t, model.residency),
			after: residencyLabel(t, change.residency),
		});
	return rows;
}

/**
 * What Configure sends: the fields the engine reads as edited, the others as
 * the model stores them, since the device restarts a model on any difference.
 */
export function configuredSettings(model: HostedModel, draft: ModelSettings) {
	const reads = new Set<string>(ENGINE_FIELDS[model.engine]);
	const settings: Record<string, unknown> = {};
	for (const [field, value] of Object.entries(model.settings))
		if (!reads.has(field)) settings[field] = value;
	for (const field of ENGINE_FIELDS[model.engine])
		if (draft[field] !== undefined) settings[field] = draft[field];
	return settings as ModelSettings;
}

export interface SettingsEdits {
	/** Fields set to another value than the model had, `undefined` for one left out now. */
	settings: ModelSettings;
	residency?: Residency;
}

/** What the person changed against the revision they started from. */
export function editsSince(
	base: HostedModel,
	draft: ModelSettings,
	residency: Residency,
) {
	const settings: Record<string, unknown> = {};
	for (const field of ENGINE_FIELDS[base.engine])
		if (!sameValue(draft[field], base.settings[field]))
			settings[field] = draft[field];
	const edits: SettingsEdits = {
		settings: settings as ModelSettings,
		...(sameResidency(residency, base.residency) ? {} : { residency }),
	};
	return edits;
}

/** The person's edits on top of a newer revision: what others changed stays unless they changed it too. */
export function rebasedDraft(edits: SettingsEdits, next: HostedModel) {
	return {
		settings: flashForCache(next.engine, {
			...next.settings,
			...edits.settings,
		}),
		residency: edits.residency ?? next.residency,
	};
}
