import {
	type ComponentProps,
	type ComponentType,
	Suspense,
	forwardRef,
	lazy,
} from "react";
type LinkProps = ComponentProps<"a"> & {
	prefetch?: boolean;
	replace?: boolean;
	scroll?: boolean;
};
export const Link = forwardRef<HTMLAnchorElement, LinkProps>(
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
	alt,
	...props
}: ComponentProps<"img"> & {
	fill?: boolean;
	priority?: boolean;
	unoptimized?: boolean;
	loader?: unknown;
}) => <img {...props} alt={alt} />;
export function dynamic<Props extends object>(
	loader: () => Promise<
		ComponentType<Props> | { default: ComponentType<Props> }
	>,
	options: { loading?: ComponentType; ssr?: boolean } = {},
) {
	const Component = lazy(async () => {
		const result = await loader();
		return { default: "default" in result ? result.default : result };
	});
	return (props: Props) => (
		<Suspense fallback={options.loading ? <options.loading /> : null}>
			<Component {...props} />
		</Suspense>
	);
}
