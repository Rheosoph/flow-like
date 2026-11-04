import { Fragment } from "react";
import { decodeStorageSegment } from "../../lib/storage-tree";
import { cn } from "../../lib/utils";
import {
	Breadcrumb,
	BreadcrumbEllipsis,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "../ui";

export function StorageBreadcrumbs({
	appId,
	prefix,
	updatePrefix,
}: Readonly<{
	appId: string;
	prefix: string;
	updatePrefix: (prefix: string) => void;
}>) {
	const segments = prefix.split("/").filter((segment) => segment !== "");
	const parentIndex = segments.length - 2;

	return (
		<Breadcrumb>
			<BreadcrumbList className="flex-nowrap">
				<BreadcrumbItem>
					<BreadcrumbLink
						className="cursor-pointer"
						onClick={(e) => {
							e.preventDefault();
							updatePrefix("");
						}}
					>
						Uploads
					</BreadcrumbLink>
				</BreadcrumbItem>
				{parentIndex > 0 && (
					<>
						<BreadcrumbSeparator className="hidden @max-xl/storage:block" />
						<BreadcrumbItem className="hidden @max-xl/storage:inline-flex">
							<BreadcrumbEllipsis className="size-5" />
						</BreadcrumbItem>
					</>
				)}
				{segments.slice(0, -1).map((part, index) => (
					<Fragment key={segments.slice(0, index + 1).join("/")}>
						<BreadcrumbSeparator
							className={
								index < parentIndex ? "@max-xl/storage:hidden" : undefined
							}
						/>
						<BreadcrumbItem
							className={cn(
								"min-w-0",
								index < parentIndex && "@max-xl/storage:hidden",
							)}
						>
							<BreadcrumbLink
								className="max-w-40 cursor-pointer truncate"
								onClick={(e) => {
									e.preventDefault();
									const newPrefix = segments.slice(0, index + 1).join("/");
									updatePrefix(newPrefix);
								}}
							>
								{decodeStorageSegment(part)}
							</BreadcrumbLink>
						</BreadcrumbItem>
					</Fragment>
				))}
				{segments.length > 0 && (
					<>
						<BreadcrumbSeparator />
						<BreadcrumbItem className="min-w-0">
							<BreadcrumbPage className="block max-w-48 truncate">
								{decodeStorageSegment(segments[segments.length - 1])}
							</BreadcrumbPage>
						</BreadcrumbItem>
					</>
				)}
			</BreadcrumbList>
		</Breadcrumb>
	);
}
