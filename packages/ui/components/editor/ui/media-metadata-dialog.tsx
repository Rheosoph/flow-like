"use client";

import { useState } from "react";
import { useEditorRef, useElement } from "platejs/react";
import {
	Button,
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	Input,
	Label,
} from "../../..";
import type { EditorialMediaElement } from "../media-metadata";

export function MediaMetadataDialog() {
	const editor = useEditorRef();
	const element = useElement<EditorialMediaElement>();
	const [open, setOpen] = useState(false);
	const update = (key: string, value: unknown) => {
		const at = editor.api.findPath(element);
		if (at) {
			if (value === undefined) editor.tf.unsetNodes(key, { at });
			else editor.tf.setNodes({ [key]: value }, { at });
		}
	};
	return (
		<Dialog open={open} onOpenChange={setOpen}>
			<DialogTrigger asChild>
				<Button size="sm" variant="ghost">
					Media details
				</Button>
			</DialogTrigger>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>Media details</DialogTitle>
				</DialogHeader>
				<div className="space-y-4">
					{(
						[
							["alt", "Alternative text"],
							["credit", "Credit"],
							["license", "License or rights statement"],
						] as const
					).map(([key, label]) => (
						<Label key={key} className="grid gap-2">
							{label}
							<Input
								value={element[key] ?? ""}
								onChange={(event) => update(key, event.target.value)}
							/>
						</Label>
					))}
					<Label className="grid gap-2">
						Rights expiry date
						<Input
							type="date"
							value={
								element.rightsExpiresAt &&
								Number.isFinite(element.rightsExpiresAt)
									? new Date(element.rightsExpiresAt).toISOString().slice(0, 10)
									: ""
							}
							onChange={(event) =>
								update(
									"rightsExpiresAt",
									event.target.value
										? Date.parse(`${event.target.value}T23:59:59.999Z`)
										: undefined,
								)
							}
						/>
					</Label>
					{element.type === "img" && (
						<fieldset className="grid grid-cols-2 gap-3">
							<legend className="mb-2 text-sm">Focal point (%)</legend>
							{(["x", "y"] as const).map((axis) => (
								<Label key={axis} className="grid gap-2">
									{axis === "x" ? "Horizontal" : "Vertical"}
									<Input
										type="number"
										min={0}
										max={100}
										value={element.focalPoint?.[axis] ?? 50}
										onChange={(event) =>
											update("focalPoint", {
												x: 50,
												y: 50,
												...element.focalPoint,
												[axis]: Math.max(
													0,
													Math.min(100, Number(event.target.value)),
												),
											})
										}
									/>
								</Label>
							))}
						</fieldset>
					)}
					<Button onClick={() => setOpen(false)}>Done</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
