"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useState } from "react";
import type { ILayer } from "../../lib/schema/flow/board";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "../ui";

export function FlowBreadCrumb({
	currentPath,
	layers,
	onAdjustPath,
}: Readonly<{
	currentPath: string | undefined;
	onAdjustPath: (path?: string) => void;
	layers:
		| {
				[key: string]: ILayer;
		  }
		| undefined;
}>) {
	const { t } = useTranslation("flow");
	const [segments, setSegments] = useState<string[]>(
		currentPath?.split("/") ?? [],
	);

	useEffect(() => {
		setSegments(currentPath?.split("/") ?? []);
	}, [currentPath]);

	return (
		<div className="px-2 py-1 bg-background rounded-md">
			<Breadcrumb>
				<BreadcrumbList>
					<BreadcrumbItem>
						<BreadcrumbLink
							onClick={() => {
								setSegments([]);
								onAdjustPath();
							}}
							className="cursor-pointer"
						>
							{t("board", "Board")}
						</BreadcrumbLink>
					</BreadcrumbItem>
					{segments.map((segment, index, array) => {
						if (index === array.length - 1) {
							return (
								<>
									<BreadcrumbSeparator />
									<BreadcrumbItem key={index}>
										<BreadcrumbPage>
											{layers?.[segment]?.name ?? segment}
										</BreadcrumbPage>
									</BreadcrumbItem>
								</>
							);
						}
						return (
							<>
								<BreadcrumbSeparator />
								<BreadcrumbItem key={index}>
									<BreadcrumbLink
										className="cursor-pointer"
										onClick={() => {
											const newPath = segments.slice(0, index + 1).join("/");
											setSegments(segments.slice(0, index + 1));
											onAdjustPath(newPath);
										}}
									>
										{layers?.[segment]?.name ?? segment}
									</BreadcrumbLink>
								</BreadcrumbItem>
							</>
						);
					})}
				</BreadcrumbList>
			</Breadcrumb>
		</div>
	);
}
