import { type IPin, IPinType } from "../../../../lib/schema/flow/pin";
import { PinLabel } from "../pin-chrome";

export function VariableDescription({ pin }: Readonly<{ pin: IPin }>) {
	return (
		<PinLabel
			text={pin.friendly_name}
			align={pin.pin_type === IPinType.Input ? "start" : "end"}
		/>
	);
}
