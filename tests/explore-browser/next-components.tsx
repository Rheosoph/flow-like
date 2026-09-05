import {
	type ComponentProps,
	type ComponentType,
	Suspense,
	forwardRef,
	lazy,
} from "react";
import { router } from "./next-navigation";
type LinkProps = ComponentProps<"a"> & {
	prefetch?: boolean;
	replace?: boolean;
	scroll?: boolean;
};
export const Link = forwardRef<HTMLAnchorElement, LinkProps>(
	({ children, prefetch, replace, scroll, onClick, ...props }, ref) => (
		<a
			{...props}
			href={props.href}
			ref={ref}
			onClick={(event) => {
				onClick?.(event);
				if (
					event.defaultPrevented ||
					event.button !== 0 ||
					event.metaKey ||
					event.ctrlKey ||
					event.shiftKey ||
					event.altKey ||
					!props.href?.startsWith("/")
				)
					return;
				event.preventDefault();
				if (replace) router.replace(props.href);
				else router.push(props.href);
			}}
		>
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
