/**
 * Motion primitives shared by the soap-film loops.
 *
 * Everything here is frame-rate independent or a pure function of elapsed time. The per-frame
 * constants these replaced ran at double speed on a 120Hz display and stuttered whenever a frame
 * was dropped, which reads as "not smooth" long before anyone can say why.
 */

export const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

export const smoothstep = (value: number) => {
	const t = clamp01(value);
	return t * t * (3 - 2 * t);
};

/** 0 before `from`, 1 after `to`, linear between. The spine of a timed choreography. */
export const segment = (t: number, from: number, to: number) =>
	clamp01((t - from) / (to - from));

export const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

/** Frame-rate independent damping toward a target. */
export const approach = (
	current: number,
	target: number,
	rate: number,
	dt: number,
) => current + (target - current) * (1 - Math.exp(-rate * dt));

/**
 * Step response of a damped spring, as a pure function of time rather than an integrator.
 *
 * A choreography written this way can be evaluated at any instant, which is what lets reduced
 * motion pose the mark at its resting time instead of freezing its first frame, and what keeps
 * two displays running at different refresh rates on the same curve.
 *
 * @param frequency undamped natural frequency, in Hz.
 * @param damping 0…1. Below ~0.5 overshoots visibly; 1 would not return.
 */
export function springStep(t: number, frequency: number, damping: number) {
	if (t <= 0) return 0;
	const w = 2 * Math.PI * frequency;
	const wd = w * Math.sqrt(1 - damping * damping);
	return (
		1 -
		Math.exp(-damping * w * t) *
			(Math.cos(wd * t) + ((damping * w) / wd) * Math.sin(wd * t))
	);
}
