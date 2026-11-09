import {
	DATA_CLASSIFICATIONS,
	type DataClassification,
	type IDataSensitivity,
	combineDataSensitivity,
} from "@flow-like/flow-like-ui/lib/data-classification";
import { LuFingerprint } from "react-icons/lu";
import {
	dataClassificationConfidential,
	dataClassificationConfidentialHint,
	dataClassificationInternal,
	dataClassificationInternalHint,
	dataClassificationPublic,
	dataClassificationPublicHint,
	dataClassificationRestricted,
	dataClassificationRestrictedHint,
	piiBadge,
	piiHint,
	piiLabel,
} from "../../../../../packages/locales/locales/en/common.json";

export type { DataClassification, IDataSensitivity };
export { combineDataSensitivity, DATA_CLASSIFICATIONS };

/** The product's own wording for each level. */
export const LEVEL_COPY: Readonly<
	Record<DataClassification, { label: string; hint: string }>
> = {
	public: {
		label: dataClassificationPublic,
		hint: dataClassificationPublicHint,
	},
	internal: {
		label: dataClassificationInternal,
		hint: dataClassificationInternalHint,
	},
	confidential: {
		label: dataClassificationConfidential,
		hint: dataClassificationConfidentialHint,
	},
	restricted: {
		label: dataClassificationRestricted,
		hint: dataClassificationRestrictedHint,
	},
};

export const PII_COPY = { badge: piiBadge, label: piiLabel, hint: piiHint };

export function LevelBadge({ level }: { level: DataClassification }) {
	return (
		<span
			className="dt-badge"
			data-level={level}
			title={LEVEL_COPY[level].hint}
		>
			<i />
			{LEVEL_COPY[level].label}
		</span>
	);
}

export function PiiBadge() {
	return (
		<span className="dt-badge dt-badge--pii" title={PII_COPY.label}>
			<LuFingerprint aria-hidden="true" />
			{PII_COPY.badge}
		</span>
	);
}

export function SensitivityBadges({ value }: { value: IDataSensitivity }) {
	return (
		<span className="dt-badges">
			{value.classification ? (
				<LevelBadge level={value.classification} />
			) : null}
			{value.pii ? <PiiBadge /> : null}
		</span>
	);
}
