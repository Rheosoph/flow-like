import { EllipsisVerticalIcon, GripIcon, ListIcon } from "lucide-react";
import { IValueType, type IVariable, type IVariableType } from "../../../lib";
import { Badge, Label } from "../../ui";
import { typeToColor } from "../utils";
import { VariablesMenuEdit } from "./variables-menu-edit";

export function VariableConfigCard({
	disabled,
	variable,
	onUpdate,
	refs,
}: Readonly<{
	disabled?: boolean;
	variable: IVariable;
	onUpdate: (variable: IVariable) => Promise<void>;
	refs?: Record<string, string>;
}>) {
	return (
		<div className="border rounded-lg p-5 bg-card hover:bg-accent/5 transition-all duration-200 group">
			<div className="flex flex-col items-start justify-between gap-4 w-full">
				<div className="flex items-center gap-4 flex-1">
					<div className="shrink-0 mt-1">
						<VariableTypeIndicator
							type={variable.value_type}
							valueType={variable.data_type}
						/>
					</div>
					<div className="flex-1 min-w-0 space-y-2">
						<div className="flex items-center gap-3">
							<Label className="font-semibold text-base">{variable.name}</Label>
							<VariableTypeBadge
								type={variable.value_type}
								valueType={variable.data_type}
							/>
						</div>
						{variable.description && (
							<p className="text-sm text-muted-foreground leading-relaxed">
								{variable.description}
							</p>
						)}
					</div>
				</div>
				<div className="shrink-0 z-0 opacity-60 group-hover:opacity-100 transition-opacity w-full">
					<VariablesMenuEdit
						disabled={disabled}
						variable={variable}
						refs={refs}
						updateVariable={async (updatedVariable) => {
							await onUpdate(updatedVariable);
						}}
					/>
				</div>
			</div>
		</div>
	);
}

export function VariableTypeIndicator({
	type,
	valueType,
}: Readonly<{
	type: IValueType;
	valueType: IVariableType;
}>) {
	const color = typeToColor(valueType);
	const baseStyle = "w-6 h-6 p-1 flex items-center justify-center";

	switch (type) {
		case IValueType.Normal:
			return (
				<div className={baseStyle} style={{ borderColor: color }}>
					<div
						className="w-4 h-2 rounded-full"
						style={{ backgroundColor: color }}
					/>
				</div>
			);
		case IValueType.Array:
			return (
				<div className={baseStyle} style={{ borderColor: color }}>
					<GripIcon className="w-5 h-5" style={{ color }} />
				</div>
			);
		case IValueType.HashSet:
			return (
				<div className={baseStyle} style={{ borderColor: color }}>
					<EllipsisVerticalIcon className="w-5 h-5" style={{ color }} />
				</div>
			);
		case IValueType.HashMap:
			return (
				<div className={baseStyle} style={{ borderColor: color }}>
					<ListIcon className="w-5 h-5" style={{ color }} />
				</div>
			);
	}

	return (
		<div className="w-10 h-10 rounded-lg flex items-center justify-center border bg-background">
			<span className="text-xs font-mono">{type}</span>
		</div>
	);
}

function VariableTypeBadge({
	type,
	valueType,
}: Readonly<{
	type: IValueType;
	valueType: IVariableType;
}>) {
	const getTypeLabel = () => {
		switch (type) {
			case IValueType.Array:
				return `List<${valueType}>`;
			case IValueType.HashSet:
				return `Set<${valueType}>`;
			case IValueType.HashMap:
				return `Map<${valueType}>`;
			default:
				return valueType;
		}
	};

	const baseColor = typeToColor(valueType);

	return (
		<Badge
			variant="secondary"
			className="text-xs font-mono bg-muted/50 hover:bg-muted transition-colors"
			style={{
				borderColor: `${baseColor}60`,
				color: baseColor,
				backgroundColor: `${baseColor}10`,
			}}
		>
			{getTypeLabel()}
		</Badge>
	);
}
