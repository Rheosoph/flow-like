import { useTranslation } from "@flow-like/locales";
import { Label } from "../../../components/ui/label";
import { Switch } from "../../../components/ui/switch";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";

export function BoolVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<div className="flex items-center space-x-2">
			<Switch
				disabled={disabled}
				checked={parseUint8ArrayToJson(variable.default_value)}
				onCheckedChange={(checked) =>
					onChange({
						...variable,
						default_value: convertJsonToUint8Array(checked),
					})
				}
				id="default_value"
			/>
			<Label htmlFor="default_value">{t('defaultValue', 'Default Value')}</Label>
		</div>
	);
}
