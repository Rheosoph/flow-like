"use client";
import { Globe, KeyRound, Lock, Shield } from "lucide-react";
import { IAppVisibility } from "../../lib/schema/app/app";

export function visibilityLabel(v: IAppVisibility) {
	switch (v) {
		case IAppVisibility.Public:
			return "Public";
		case IAppVisibility.Private:
			return "Private";
		case IAppVisibility.Prototype:
			return "Prototype";
		case IAppVisibility.PublicRequestAccess:
			return "Request access";
		case IAppVisibility.Offline:
			return "Offline";
		default:
			return "Unknown";
	}
}

export function visibilityIcon(v: IAppVisibility) {
	const cl = "h-4 w-4";
	switch (v) {
		case IAppVisibility.Public:
			return <Globe className={cl} />;
		case IAppVisibility.Private:
			return <Lock className={cl} />;
		case IAppVisibility.Prototype:
			return <Shield className={cl} />;
		case IAppVisibility.PublicRequestAccess:
			return <KeyRound className={cl} />;
		case IAppVisibility.Offline:
			return <Lock className={cl} />;
		default:
			return <Shield className={cl} />;
	}
}
