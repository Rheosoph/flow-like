"use client";

import { VariableIcon } from "lucide-react";
import {
	type FC,
	type RefObject,
	memo,
	useCallback,
	useEffect,
	useState,
} from "react";
import { Button } from "../../../components/ui/button";
import { IValueType } from "../../../lib";
import type { IBoard } from "../../../lib/schema/flow/board";
import {
	type IPin,
	IPinType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import useFlowControlState from "../../../state/flow-control-state";
import type { FlowSelectorDataRef } from "../flow-selector-data";
import { BitVariable } from "./variable-types/bit-select";
import { BooleanVariable } from "./variable-types/boolean-variable";
import { VariableDescription } from "./variable-types/default-text";
import { ElementSelect } from "./variable-types/element-select";
import { EnumVariable } from "./variable-types/enum-variable";
import { FnVariable } from "./variable-types/fn-select";
import {
	OntologyActionSelect,
	OntologyObjectSelect,
	OntologySelect,
	RemoteOntologyActionSelect,
	RemoteOntologyObjectSelect,
	RemoteOntologySelect,
} from "./variable-types/ontology-pin-selects";
import { ProjectUserSelect } from "./variable-types/project-user-select";
import { RemoteDatabaseSelect } from "./variable-types/remote-database-select";
import { RemoteEventSelect } from "./variable-types/remote-event-select";
import { RemoteProjectSelect } from "./variable-types/remote-project-select";
import { VarVariable } from "./variable-types/var-select";
import { WidgetVariable } from "./variable-types/widget-select";

type PinDefaultValue = IPin["default_value"];

interface PinEditProps {
	readonly nodeId: string;
	readonly nodeName?: string;
	readonly pin: IPin;
	readonly defaultValue: PinDefaultValue;
	readonly appId: string;
	readonly boardId?: string;
	readonly boardRef?: RefObject<IBoard | undefined>;
	readonly changeDefaultValue: (value: PinDefaultValue) => void;
	readonly saveDefaultValue: (value: PinDefaultValue) => Promise<void>;
	readonly currentLayerId?: string;
	readonly selectorDataRef?: FlowSelectorDataRef;
	readonly selectorDataVersion?: number;
}

export const PinEdit: FC<PinEditProps> = memo(function PinEdit({
	nodeId,
	nodeName,
	pin,
	defaultValue,
	appId,
	boardId,
	boardRef,
	changeDefaultValue,
	saveDefaultValue,
	currentLayerId,
	selectorDataRef,
}: PinEditProps) {
	const [cachedDefaultValue, setCachedDefaultValue] = useState(defaultValue);

	// Sync cached value when prop changes (e.g., after board refetch)
	useEffect(() => {
		setCachedDefaultValue(defaultValue);
	}, [defaultValue]);

	const updateDefaultValue = useCallback(
		async (value: unknown) => {
			const nextValue = value as PinDefaultValue;
			setCachedDefaultValue(nextValue);
			changeDefaultValue(nextValue);
			await saveDefaultValue(nextValue);
		},
		[changeDefaultValue, saveDefaultValue],
	);

	const previewDefaultValue = useCallback(
		(value: PinDefaultValue) => {
			setCachedDefaultValue(value);
			changeDefaultValue(value);
		},
		[changeDefaultValue],
	);

	if (pin.pin_type === IPinType.Output)
		return <VariableDescription pin={pin} />;
	if (pin.depends_on.length > 0) return <VariableDescription pin={pin} />;
	if (
		pin.name === "_flow_user_sub" &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<ProjectUserSelect
				pin={pin}
				value={cachedDefaultValue}
				appId={appId}
				setValue={updateDefaultValue}
			/>
		);
	}

	if (
		pin.name === "_flow_remote_app_id" &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<RemoteProjectSelect
				pin={pin}
				value={cachedDefaultValue}
				appId={appId}
				setValue={updateDefaultValue}
			/>
		);
	}

	if (
		pin.name === "_flow_remote_database" &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<RemoteDatabaseSelect
				pin={pin}
				value={cachedDefaultValue}
				appId={appId}
				nodeId={nodeId}
				boardRef={boardRef}
				setValue={updateDefaultValue}
			/>
		);
	}

	if (
		pin.name === "_flow_remote_event" &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<RemoteEventSelect
				pin={pin}
				value={cachedDefaultValue}
				appId={appId}
				boardId={boardId}
				nodeId={nodeId}
				boardRef={boardRef}
				setValue={updateDefaultValue}
				onPreviewValue={previewDefaultValue}
			/>
		);
	}

	if (pin.name === "_flow_remote_event_meta") {
		return <VariableDescription pin={pin} />;
	}

	if (
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		const ontologyProps = {
			pin,
			value: cachedDefaultValue,
			appId,
			boardId,
			nodeId,
			currentLayerId,
			boardRef,
			setValue: updateDefaultValue,
		} as const;
		if (
			(nodeName === "ontology_query_objects" ||
				nodeName === "ontology_action_request" ||
				nodeName === "ontology_action_input") &&
			pin.name === "ontology_id"
		) {
			return <OntologySelect {...ontologyProps} />;
		}
		if (nodeName === "ontology_query_objects" && pin.name === "object_type") {
			return <OntologyObjectSelect {...ontologyProps} />;
		}
		if (
			(nodeName === "ontology_action_request" ||
				nodeName === "ontology_action_input") &&
			pin.name === "action_id"
		) {
			return <OntologyActionSelect {...ontologyProps} />;
		}
		if (
			(nodeName === "ontology_query_remote_objects" ||
				nodeName === "ontology_query_remote_children" ||
				nodeName === "ontology_action_request_remote") &&
			pin.name === "binding_id"
		) {
			return <RemoteOntologySelect {...ontologyProps} />;
		}
		if (
			(nodeName === "ontology_query_remote_objects" ||
				nodeName === "ontology_query_remote_children") &&
			pin.name === "object_type"
		) {
			return <RemoteOntologyObjectSelect {...ontologyProps} />;
		}
		if (
			nodeName === "ontology_action_request_remote" &&
			pin.name === "action_id"
		) {
			return <RemoteOntologyActionSelect {...ontologyProps} />;
		}
	}

	if (
		nodeName === "a2ui_instantiate_widget" &&
		pin.name === "widget_selector" &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<WidgetVariable
				pin={pin}
				value={cachedDefaultValue}
				appId={appId}
				setValue={updateDefaultValue}
			/>
		);
	}

	if (pin.data_type === IVariableType.Boolean)
		return (
			<BooleanVariable
				pin={pin}
				value={cachedDefaultValue}
				setValue={updateDefaultValue}
			/>
		);
	if (
		(pin.options?.valid_values?.length ?? 0) > 0 &&
		pin.data_type === IVariableType.String
	)
		return (
			<EnumVariable
				pin={pin}
				value={cachedDefaultValue}
				setValue={updateDefaultValue}
			/>
		);

	if (
		pin.name.startsWith("bit_id") &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<BitVariable
				pin={pin}
				value={cachedDefaultValue}
				setValue={updateDefaultValue}
				selectorDataRef={selectorDataRef}
			/>
		);
	}

	if (
		pin.name.startsWith("fn_ref") &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<FnVariable
				boardRef={boardRef}
				pin={pin}
				value={cachedDefaultValue}
				setValue={updateDefaultValue}
			/>
		);
	}

	if (
		pin.name.startsWith("var_ref") &&
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<VarVariable
				boardRef={boardRef}
				pin={pin}
				value={cachedDefaultValue}
				currentLayerId={currentLayerId}
				setValue={updateDefaultValue}
			/>
		);
	}

	if (
		pin.name.startsWith("element_ref") &&
		pin.value_type === IValueType.Normal
	) {
		return (
			<ElementSelect
				pin={pin}
				value={cachedDefaultValue}
				setValue={updateDefaultValue}
				selectorDataRef={selectorDataRef}
			/>
		);
	}

	return (
		<WithMenu nodeId={nodeId} pin={pin} defaultValue={cachedDefaultValue} />
	);
});

function WithMenuInner({
	nodeId,
	pin,
	defaultValue,
}: Readonly<{
	nodeId: string;
	pin: IPin;
	defaultValue: number[] | undefined | null;
}>) {
	const { editPin } = useFlowControlState();
	const isConnected = pin.connected_to && pin.connected_to.length > 0;
	const hasNoDefaultValue =
		typeof defaultValue === "undefined" || defaultValue === null;

	return (
		<>
			<VariableDescription pin={pin} />
			{!isConnected && (
				<Button
					size={"icon"}
					variant={"ghost"}
					className="w-fit h-fit text-foreground"
					onClick={() => {
						editPin(nodeId, pin);
					}}
				>
					<VariableIcon
						className={`size-[0.45rem] ${hasNoDefaultValue && "text-primary"}`}
					/>
				</Button>
			)}
		</>
	);
}

const WithMenu = memo(WithMenuInner) as typeof WithMenuInner;
