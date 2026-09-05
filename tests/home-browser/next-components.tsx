import { Suspense, forwardRef, lazy } from "react";
export const Link = forwardRef<HTMLAnchorElement, any>(
	({ children, prefetch, replace, scroll, ...props }, ref) => (
		<a {...props} ref={ref}>
			{children}
		</a>
	),
);
export const Image = ({
	fill,
	priority,
	unoptimized,
	loader,
	...props
}: any) => <img {...props} />;
export function dynamic(loader: () => Promise<any>, options: any = {}) {
	const Component = lazy(async () => {
		const result = await loader();
		return { default: result.default ?? result };
	});
	return (props: any) => (
		<Suspense fallback={options.loading ? <options.loading /> : null}>
			<Component {...props} />
		</Suspense>
	);
}
