"use client";

import { Cloud, Globe, Info } from "lucide-react";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	Input,
	Label,
} from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

type RestSink = {
	sink_type?: "rest";
	prefix?: string;
};

export function RestConfig({
	config,
	onConfigUpdate,
	isEditing,
}: IConfigInterfaceProps) {
	const current = (config as RestSink) ?? {};
	const prefix = current.prefix ?? "";

	const setValue = (key: keyof RestSink, value: unknown) => {
		onConfigUpdate?.({
			...(config as RestSink),
			sink_type: "rest",
			[key]: value,
		} as any);
	};

	return (
		<div className="w-full space-y-4">
			<Alert>
				<Globe className="h-4 w-4" />
				<AlertTitle className="flex items-center gap-2">
					REST API Server
					<span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs">
						<Cloud className="h-3 w-3" /> Remote only
					</span>
				</AlertTitle>
				<AlertDescription>
					Spins up a remote REST API server. Endpoints, methods, authentication
					and request schemas are declared inside the workflow board — the
					event itself only mounts the server.
				</AlertDescription>
			</Alert>

			<div className="space-y-2">
				<Label htmlFor="rest_prefix">Prefix</Label>
				<Input
					id="rest_prefix"
					placeholder="/api"
					disabled={!isEditing}
					value={prefix}
					onChange={(event) => setValue("prefix", event.target.value)}
				/>
				<p className="text-xs text-muted-foreground flex items-center gap-1">
					<Info className="h-3 w-3" />
					Path prefix the REST API is mounted under (e.g. <code>/api</code>).
					All endpoints declared by the workflow are served below it.
				</p>
			</div>
		</div>
	);
}
