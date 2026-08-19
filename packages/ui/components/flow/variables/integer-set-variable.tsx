import { useTranslation } from "@flow-like/locales";
import { PlusCircleIcon, XIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Input } from "../../../components/ui/input";
import type { IVariable } from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { Button, Separator } from "../../ui";

export function IntegerSetVariable({
	disabled,
	variable,
	onChange,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onChange: (variable: IVariable) => void;
}>) {
	const { t } = useTranslation("flow");
	const [newValue, setNewValue] = useState("");

	const values = useMemo<number[]>(() => {
		const parsed = parseUint8ArrayToJson(variable.default_value);
		if (!Array.isArray(parsed)) return [];
		return parsed.map((v) => {
			const n = Number.parseInt(String(v), 10);
			return Number.isNaN(n) ? 0 : n;
		});
	}, [variable.default_value]);

	const handleAdd = useCallback(() => {
		if (disabled) return;
		const trimmed = newValue.trim();
		if (!trimmed) return;
		const num = Number.parseInt(trimmed, 10);
		if (Number.isNaN(num)) return;
		const updated = [...values, num];
		onChange({
			...variable,
			default_value: convertJsonToUint8Array(Array.from(new Set(updated))),
		});
		setNewValue("");
	}, [disabled, newValue, values, onChange, variable]);

	const handleRemove = useCallback(
		(index: number) => {
			if (disabled) return;
			const updated = values.filter((_, i) => i !== index);
			onChange({
				...variable,
				default_value: convertJsonToUint8Array(Array.from(new Set(updated))),
			});
		},
		[disabled, values, onChange, variable],
	);

	return (
		<div className="flex flex-col gap-3 w-full min-w-0">
			<div className="flex gap-2 w-full min-w-0">
				<Input
					disabled={disabled}
					value={newValue}
					onChange={(e) => setNewValue(e.target.value)}
					onKeyDown={(e) => e.key === "Enter" && handleAdd()}
					type={variable.secret ? "password" : "number"}
					placeholder={t('addUniqueInteger', 'Add unique integer...')}
					step={1}
					className="flex-1 min-w-0"
				/>
				<Button
					size="icon"
					variant="default"
					onClick={handleAdd}
					disabled={
						newValue.trim() === "" || Number.isNaN(Number(newValue)) || disabled
					}
					className="shrink-0"
				>
					<PlusCircleIcon className="w-4 h-4" />
				</Button>
			</div>

			{values.length > 0 && (
				<>
					<Separator />
					<div className="flex flex-col gap-2 rounded-md border p-3">
						{values.map((value, idx) => (
							<div
								key={`${variable.name}-${idx}`}
								className="group flex items-center gap-2 rounded-md bg-secondary px-3 py-2 text-sm"
							>
								<span className="flex-1">{value}</span>
								<Button
									size="icon"
									variant="ghost"
									onClick={() => handleRemove(idx)}
									disabled={disabled}
									className="h-5 w-5 shrink-0 rounded-sm hover:bg-destructive hover:text-destructive-foreground"
								>
									<XIcon className="h-3 w-3" />
								</Button>
							</div>
						))}
					</div>
				</>
			)}
		</div>
	);
}
