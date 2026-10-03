import type {
	DevicesRoute,
	DevicesScope,
} from "../../../lib/device-management/model/types";

export interface ScreenProps {
	route: DevicesRoute;
	scope: DevicesScope;
}

export interface DeviceTabProps extends ScreenProps {
	deviceId: string;
}

export interface ServiceTabProps extends DeviceTabProps {
	serviceId: string;
}
