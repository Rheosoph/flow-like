//! A person at the controls, for the bot browser's human mode: where the pointer travels, how
//! long a click or a keystroke lasts, how the wheel turns, and what the hand does while the bot
//! thinks. One [`HumanPacing`] keeps one persona (speed, steadiness, typing rhythm, wheel or
//! trackpad) for a whole session, so the rhythm a page sees stays consistent.
//!
//! Pointer moves follow Fitts's law for their duration and a minimum-jerk speed profile skewed
//! towards an early peak, along a bent path with physiological tremor. Long moves overshoot now
//! and then; most land a little short and end with a corrective submovement. Samples are spaced
//! 7–11 ms apart with jitter, so Chrome coalesces about two of them per frame as it does for a
//! real mouse.

use std::f64::consts::TAU;
use std::sync::Mutex;
use std::time::Duration;

use flow_like_browser::input::keys::{KeyEvent, KeyEventKind};
use flow_like_browser::input::paced::{Pacing, PointerStep, WheelStep};
use flow_like_types::rand::rngs::StdRng;
use flow_like_types::rand::{Rng, SeedableRng};

const MODIFIER_KEYS: [&str; 4] = ["Shift", "Control", "Alt", "Meta"];
const LEFT_HAND: &str = "qwertasdfgzxcvb12345`";
const SENTENCE_MARKS: &str = ".,;:!?";
/// A move this short is a nudge, not an aimed movement.
const NUDGE: f64 = 2.0;

/// What the hand does next while the bot is thinking.
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    /// Nothing happens for this long.
    Rest(Duration),
    Move(Vec<PointerStep>),
    /// A short look further down the page, to be undone by a later [`Gesture::Unpeek`].
    Peek(Vec<WheelStep>),
    /// Scrolls back what the last peeks moved.
    Unpeek(Vec<WheelStep>),
}

#[derive(Clone, Copy, Debug)]
struct Persona {
    /// Fitts's law: duration = a + b · log2(distance / width + 1), in milliseconds.
    fitts_a: f64,
    fitts_b: f64,
    /// Exponent on movement time before the minimum-jerk profile; below 1 the peak comes early.
    skew: f64,
    /// Preferred bend of a path, as a fraction of its length.
    arc: f64,
    tremor_px: f64,
    tremor_hz: f64,
    sample_ms: f64,
    aim_bias: (f64, f64),
    settle_ms: f64,
    hold_ms: f64,
    dwell_ms: f64,
    flight_ms: f64,
    /// Pixels per wheel notch; `None` scrolls like a trackpad, with momentum.
    notch: Option<f64>,
}

impl Persona {
    fn sample(rng: &mut StdRng) -> Self {
        let trackpad = cfg!(target_os = "macos") && rng.random_bool(0.7);
        Self {
            fitts_a: rng.random_range(60.0..130.0),
            fitts_b: rng.random_range(110.0..165.0),
            skew: rng.random_range(0.78..0.92),
            arc: normal(rng, 0.0, 0.06).clamp(-0.12, 0.12),
            tremor_px: rng.random_range(0.3..0.8),
            tremor_hz: rng.random_range(8.0..12.0),
            sample_ms: rng.random_range(7.0..11.0),
            aim_bias: (normal(rng, 0.0, 0.04), normal(rng, 0.0, 0.03)),
            settle_ms: rng.random_range(110.0..240.0),
            hold_ms: rng.random_range(75.0..125.0),
            dwell_ms: rng.random_range(70.0..110.0),
            flight_ms: rng.random_range(90.0..180.0),
            notch: (!trackpad).then_some(100.0),
        }
    }
}

pub struct HumanPacing {
    persona: Persona,
    rng: Mutex<StdRng>,
}

impl HumanPacing {
    pub fn new() -> Self {
        Self::seeded(flow_like_types::rand::rng().random())
    }

    pub fn seeded(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        Self {
            persona: Persona::sample(&mut rng),
            rng: Mutex::new(rng),
        }
    }

    fn rng(&self) -> std::sync::MutexGuard<'_, StdRng> {
        self.rng.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wheel events that scroll about `distance` CSS pixels (positive: down).
    pub fn wheel(&self, distance: f64) -> Vec<WheelStep> {
        wheel_steps(&mut self.rng(), &self.persona, distance)
    }

    /// A pause a person takes between two of their own actions (looking where the page went).
    pub fn glance(&self) -> Duration {
        lognormal(&mut self.rng(), 320.0, 0.35, 150.0, 900.0)
    }

    /// The small moves of a hand holding the button down at `at` for about `seconds`.
    pub fn hold_still(&self, at: (f64, f64), seconds: (f64, f64)) -> Vec<PointerStep> {
        let rng = &mut *self.rng();
        let total = rng.random_range(seconds.0..seconds.1) * 1000.0;
        let mut steps = Vec::new();
        let mut elapsed = 0.0;
        while elapsed < total {
            let wait = rng.random_range(60.0..160.0);
            elapsed += wait;
            steps.push(PointerStep {
                point: (
                    at.0 + normal(rng, 0.0, 0.7).clamp(-2.0, 2.0),
                    at.1 + normal(rng, 0.0, 0.7).clamp(-2.0, 2.0),
                ),
                wait: ms(wait),
            });
        }
        steps
    }

    /// A spot inside a box of `size` around `centre`, as a person would hit it.
    pub fn spot(&self, centre: (f64, f64), size: (f64, f64)) -> (f64, f64) {
        let (u, v) = self.aim(size);
        (centre.0 + (u - 0.5) * size.0, centre.1 + (v - 0.5) * size.1)
    }

    /// The next thing the hand does after `quiet` without a bot action. `owed` is how far the
    /// page is still scrolled from earlier peeks.
    pub fn idle(
        &self,
        pointer: Option<(f64, f64)>,
        viewport: (f64, f64),
        quiet: Duration,
        owed: f64,
    ) -> Gesture {
        let rng = &mut *self.rng();
        let persona = &self.persona;
        let quiet = quiet.as_secs_f64();
        if quiet < 0.3 {
            return Gesture::Rest(ms(rng.random_range(250.0..700.0)));
        }
        if owed.abs() >= 1.0 && rng.random_bool(0.45) {
            return Gesture::Unpeek(wheel_steps(rng, persona, -owed));
        }
        let rest = (0.3 + quiet / 150.0).min(0.8);
        if rng.random_bool(rest) {
            let median = 1500.0 * (1.0 + quiet / 60.0);
            return Gesture::Rest(lognormal(rng, median, 0.5, 400.0, 9000.0));
        }
        let Some(at) = pointer else {
            let to = content_point(rng, viewport);
            let from = entry(rng, viewport);
            return Gesture::Move(reach(rng, persona, from, to, 60.0));
        };
        match rng.random_range(0..100) {
            0..50 => Gesture::Move(tremor(rng, at, viewport)),
            50..80 => {
                let to = clamp_to_content(
                    (at.0 + normal(rng, 0.0, 70.0), at.1 + normal(rng, 0.0, 50.0)),
                    viewport,
                );
                Gesture::Move(relaxed(reach(rng, persona, at, to, 60.0), 1.6))
            }
            80..94 => Gesture::Move(read_sweep(rng, persona, at, viewport)),
            _ if owed.abs() < 1.0 && viewport.1 > 300.0 => {
                let distance = persona
                    .notch
                    .unwrap_or_else(|| rng.random_range(90.0..160.0));
                Gesture::Peek(wheel_steps(rng, persona, distance))
            }
            _ => Gesture::Move(tremor(rng, at, viewport)),
        }
    }
}

impl Default for HumanPacing {
    fn default() -> Self {
        Self::new()
    }
}

impl Pacing for HumanPacing {
    fn aim(&self, size: (f64, f64)) -> (f64, f64) {
        let rng = &mut *self.rng();
        let bias = self.persona.aim_bias;
        (
            aim_along(rng, bias.0, size.0),
            aim_along(rng, bias.1, size.1),
        )
    }

    fn reach(
        &self,
        from: Option<(f64, f64)>,
        to: (f64, f64),
        size: (f64, f64),
        viewport: (f64, f64),
    ) -> Vec<PointerStep> {
        let rng = &mut *self.rng();
        let start = from.unwrap_or_else(|| entry(rng, viewport));
        let width = size.0.min(size.1).max(1.0);
        reach(rng, &self.persona, start, to, width)
    }

    fn settle(&self) -> Duration {
        lognormal(&mut self.rng(), self.persona.settle_ms, 0.35, 50.0, 600.0)
    }

    fn hold(&self) -> Duration {
        lognormal(&mut self.rng(), self.persona.hold_ms, 0.22, 45.0, 220.0)
    }

    fn key_wait(&self, previous: Option<&KeyEvent>, next: &KeyEvent) -> Duration {
        key_wait(&mut self.rng(), &self.persona, previous, next)
    }
}

fn ms(millis: f64) -> Duration {
    Duration::from_secs_f64(millis.max(0.0) / 1000.0)
}

/// Box–Muller.
fn normal(rng: &mut StdRng, mean: f64, sd: f64) -> f64 {
    let u1: f64 = rng.random_range(f64::EPSILON..1.0);
    let u2: f64 = rng.random();
    mean + sd * (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos()
}

fn lognormal(rng: &mut StdRng, median_ms: f64, sigma: f64, min: f64, max: f64) -> Duration {
    ms((median_ms * normal(rng, 0.0, sigma).exp()).clamp(min, max))
}

/// A fraction along one side of an element: near the middle, never in the outer few pixels.
fn aim_along(rng: &mut StdRng, bias: f64, extent: f64) -> f64 {
    let spread = if extent < 14.0 { 0.06 } else { 0.15 };
    let margin = (6.0 / extent.max(1.0)).clamp(0.15, 0.45);
    (0.5 + bias + normal(rng, 0.0, spread)).clamp(margin, 1.0 - margin)
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// Where a pointer comes from that was not on the page yet: the browser's toolbar above it.
fn entry(rng: &mut StdRng, viewport: (f64, f64)) -> (f64, f64) {
    (
        viewport.0 * rng.random_range(0.25..0.65),
        rng.random_range(1.0..(viewport.1 * 0.04).max(2.0)),
    )
}

fn clamp_to_content(point: (f64, f64), viewport: (f64, f64)) -> (f64, f64) {
    (
        point.0.clamp(viewport.0 * 0.06, viewport.0 * 0.94),
        point.1.clamp(viewport.1 * 0.08, viewport.1 * 0.92),
    )
}

fn content_point(rng: &mut StdRng, viewport: (f64, f64)) -> (f64, f64) {
    (
        viewport.0 * rng.random_range(0.3..0.7),
        viewport.1 * rng.random_range(0.3..0.65),
    )
}

/// The minimum-jerk position profile, 0 → 1 with zero speed and acceleration at both ends.
fn minimum_jerk(tau: f64) -> f64 {
    let tau = tau.clamp(0.0, 1.0);
    tau.powi(3) * (10.0 - 15.0 * tau + 6.0 * tau * tau)
}

fn bezier(p: [(f64, f64); 4], t: f64) -> (f64, f64) {
    let mt = 1.0 - t;
    let weights = [mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t];
    p.iter()
        .zip(weights)
        .fold((0.0, 0.0), |(x, y), (point, weight)| {
            (x + point.0 * weight, y + point.1 * weight)
        })
}

/// The steps from `from` to `to`, a target `width` pixels across: a primary movement that may
/// miss a little, then a correction onto the target.
fn reach(
    rng: &mut StdRng,
    persona: &Persona,
    from: (f64, f64),
    to: (f64, f64),
    width: f64,
) -> Vec<PointerStep> {
    let length = distance(from, to);
    if length < NUDGE {
        return vec![PointerStep {
            point: to,
            wait: ms(rng.random_range(20.0..60.0)),
        }];
    }
    let direction = ((to.0 - from.0) / length, (to.1 - from.1) / length);
    let across = (-direction.1, direction.0);
    let landing = if length > 150.0 && rng.random_bool(((length - 150.0) / 1200.0).min(0.35)) {
        let past = (length * rng.random_range(0.03..0.08)).min(60.0);
        let side = normal(rng, 0.0, 4.0);
        (
            to.0 + direction.0 * past + across.0 * side,
            to.1 + direction.1 * past + across.1 * side,
        )
    } else if length > 60.0 && rng.random_bool(0.65) {
        let short = length * rng.random_range(0.02..0.07);
        let side = normal(rng, 0.0, 3.0);
        (
            to.0 - direction.0 * short + across.0 * side,
            to.1 - direction.1 * short + across.1 * side,
        )
    } else {
        to
    };
    let mut steps = movement(rng, persona, from, landing, width, 1.0, Duration::ZERO);
    if distance(landing, to) >= 0.5 {
        let look = ms(rng.random_range(40.0..140.0));
        steps.extend(movement(rng, persona, landing, to, width, 0.3, look));
    }
    steps
}

/// One submovement. `bend` scales the persona's arc; `lead` is the pause before it starts.
fn movement(
    rng: &mut StdRng,
    persona: &Persona,
    from: (f64, f64),
    to: (f64, f64),
    width: f64,
    bend: f64,
    lead: Duration,
) -> Vec<PointerStep> {
    let length = distance(from, to);
    if length < NUDGE {
        return vec![PointerStep {
            point: to,
            wait: lead + ms(rng.random_range(15.0..40.0)),
        }];
    }
    let index = (length / width.max(1.0) + 1.0).log2();
    let duration = ((persona.fitts_a + persona.fitts_b * index) * normal(rng, 0.0, 0.12).exp())
        .clamp(80.0, 1600.0);
    let direction = ((to.0 - from.0) / length, (to.1 - from.1) / length);
    let across = (-direction.1, direction.0);
    let arc = (persona.arc + normal(rng, 0.0, 0.07)).clamp(-0.3, 0.3) * length * bend;
    let control = |along: f64, bulge: f64| {
        (
            from.0 + direction.0 * length * along + across.0 * arc * bulge,
            from.1 + direction.1 * length * along + across.1 * arc * bulge,
        )
    };
    let curve = [
        from,
        control(0.3, rng.random_range(0.7..1.2)),
        control(0.7, rng.random_range(0.4..0.9)),
        to,
    ];
    let phase = rng.random_range(0.0..TAU);
    let mut steps = Vec::new();
    let mut clock = 0.0;
    let mut wait = lead;
    loop {
        let gap = persona.sample_ms * rng.random_range(0.65..1.35);
        if clock + gap >= duration {
            steps.push(PointerStep {
                point: to,
                wait: wait + ms((duration - clock).max(1.0)),
            });
            return steps;
        }
        clock += gap;
        let tau = clock / duration;
        let (x, y) = bezier(curve, minimum_jerk(tau.powf(persona.skew)));
        let fade = ((1.0 - tau) * 4.0).min(1.0);
        let wobble = persona.tremor_px
            * fade
            * ((TAU * persona.tremor_hz * clock / 1000.0 + phase).sin() + normal(rng, 0.0, 0.3));
        steps.push(PointerStep {
            point: (x + across.0 * wobble, y + across.1 * wobble),
            wait: wait + ms(gap),
        });
        wait = Duration::ZERO;
    }
}

/// The same path, unhurried.
fn relaxed(steps: Vec<PointerStep>, factor: f64) -> Vec<PointerStep> {
    steps
        .into_iter()
        .map(|step| PointerStep {
            wait: step.wait.mul_f64(factor),
            ..step
        })
        .collect()
}

/// A resting hand: the pointer shifts by a pixel or two now and then.
fn tremor(rng: &mut StdRng, at: (f64, f64), viewport: (f64, f64)) -> Vec<PointerStep> {
    let count = rng.random_range(3..10);
    let mut point = at;
    (0..count)
        .map(|_| {
            let drift = (normal(rng, 0.0, 1.0), normal(rng, 0.0, 1.0));
            point = (
                (point.0 + drift.0)
                    .clamp(at.0 - 3.0, at.0 + 3.0)
                    .clamp(0.0, viewport.0 - 1.0),
                (point.1 + drift.1)
                    .clamp(at.1 - 3.0, at.1 + 3.0)
                    .clamp(0.0, viewport.1 - 1.0),
            );
            PointerStep {
                point,
                wait: ms(rng.random_range(25.0..90.0)),
            }
        })
        .collect()
}

/// The pointer follows a line of text to the right, drifting down a little.
fn read_sweep(
    rng: &mut StdRng,
    persona: &Persona,
    at: (f64, f64),
    viewport: (f64, f64),
) -> Vec<PointerStep> {
    let to = clamp_to_content(
        (
            at.0 + rng.random_range(120.0..380.0),
            at.1 + rng.random_range(-4.0..18.0),
        ),
        viewport,
    );
    relaxed(reach(rng, persona, at, to, 200.0), 2.2)
}

fn wheel_steps(rng: &mut StdRng, persona: &Persona, distance: f64) -> Vec<WheelStep> {
    if distance.abs() < 1.0 {
        return Vec::new();
    }
    let sign = distance.signum();
    match persona.notch {
        Some(notch) => notches(rng, notch, sign, distance.abs()),
        None => flicks(rng, sign, distance.abs()),
    }
}

/// A wheel with detents: notches in bursts of two to six.
fn notches(rng: &mut StdRng, notch: f64, sign: f64, distance: f64) -> Vec<WheelStep> {
    let mut left = (distance / notch).round().max(1.0) as u32;
    let mut steps = Vec::new();
    while left > 0 {
        let burst = rng.random_range(2..=6).min(left);
        for index in 0..burst {
            let wait = if index == 0 && !steps.is_empty() {
                rng.random_range(180.0..650.0)
            } else if index == 0 {
                rng.random_range(0.0..40.0)
            } else {
                rng.random_range(30.0..90.0)
            };
            steps.push(WheelStep {
                delta: (0.0, sign * notch),
                wait: ms(wait),
            });
        }
        left -= burst;
    }
    steps
}

/// A trackpad: flicks whose deltas decay like momentum scrolling, one event a frame.
fn flicks(rng: &mut StdRng, sign: f64, distance: f64) -> Vec<WheelStep> {
    let mut left = distance;
    let mut steps = Vec::new();
    while left >= 1.0 {
        let flick = if left < 700.0 {
            left
        } else {
            rng.random_range(250.0_f64..700.0).min(left)
        };
        let decay: f64 = rng.random_range(0.86..0.93);
        let mut delta = flick * (1.0 - decay);
        let mut moved = 0.0;
        let mut first = true;
        while moved < flick - 0.5 {
            let size = delta.round().max(1.0).min((flick - moved).round().max(1.0));
            let wait = if first && !steps.is_empty() {
                rng.random_range(120.0..400.0)
            } else if first {
                rng.random_range(0.0..30.0)
            } else {
                rng.random_range(14.0..20.0)
            };
            steps.push(WheelStep {
                delta: (0.0, sign * size),
                wait: ms(wait),
            });
            moved += size;
            delta *= decay;
            first = false;
        }
        left -= moved;
    }
    steps
}

fn is_modifier(event: &KeyEvent) -> bool {
    MODIFIER_KEYS.contains(&event.key.as_str())
}

fn letter(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let first = chars.next()?.to_ascii_lowercase();
    chars.next().is_none().then_some(first)
}

/// Rhythm between two keys: repeats and keys on alternating hands come quicker.
fn digraph(previous: &str, next: &str) -> f64 {
    let (Some(a), Some(b)) = (letter(previous), letter(next)) else {
        return 1.0;
    };
    if a == b {
        0.85
    } else if !a.is_alphanumeric() || !b.is_alphanumeric() {
        1.0
    } else if LEFT_HAND.contains(a) != LEFT_HAND.contains(b) {
        0.88
    } else {
        1.08
    }
}

fn key_wait(
    rng: &mut StdRng,
    persona: &Persona,
    previous: Option<&KeyEvent>,
    next: &KeyEvent,
) -> Duration {
    match next.kind {
        KeyEventKind::Char => Duration::ZERO,
        KeyEventKind::KeyUp if is_modifier(next) => lognormal(rng, 35.0, 0.4, 10.0, 120.0),
        KeyEventKind::KeyUp => lognormal(rng, persona.dwell_ms, 0.25, 35.0, 220.0),
        KeyEventKind::RawKeyDown | KeyEventKind::KeyDown => match previous {
            None => lognormal(rng, 70.0, 0.4, 20.0, 250.0),
            Some(previous) if is_modifier(previous) && previous.kind != KeyEventKind::KeyUp => {
                lognormal(rng, 70.0, 0.35, 25.0, 200.0)
            }
            Some(previous) => {
                let mut wait = persona.flight_ms
                    * normal(rng, 0.0, 0.38).exp()
                    * digraph(&previous.key, &next.key);
                if previous.key == " " && rng.random_bool(0.15) {
                    wait += rng.random_range(200.0..650.0);
                }
                if previous.key.len() == 1 && SENTENCE_MARKS.contains(previous.key.as_str()) {
                    wait += rng.random_range(120.0..380.0);
                }
                if rng.random_bool(0.025) {
                    wait += rng.random_range(400.0..1100.0);
                }
                ms(wait.clamp(25.0, 2500.0))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use flow_like_browser::input::keys::{Modifiers, convert_keys};

    use super::*;

    const VIEWPORT: (f64, f64) = (1280.0, 900.0);

    fn total(steps: &[PointerStep]) -> Duration {
        steps.iter().map(|step| step.wait).sum()
    }

    fn deviation(steps: &[PointerStep], from: (f64, f64), to: (f64, f64)) -> f64 {
        let length = distance(from, to);
        steps
            .iter()
            .map(|step| {
                ((to.0 - from.0) * (from.1 - step.point.1)
                    - (from.0 - step.point.0) * (to.1 - from.1))
                    .abs()
                    / length
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn every_reach_ends_on_the_target_and_takes_human_time() {
        for seed in 0..200 {
            let pacing = HumanPacing::seeded(seed);
            let from = (100.0, 700.0);
            let to = (900.0, 180.0);
            let steps = pacing.reach(Some(from), to, (120.0, 32.0), VIEWPORT);
            assert_eq!(steps.last().unwrap().point, to, "seed {seed}");
            let took = total(&steps);
            assert!(
                (Duration::from_millis(250)..Duration::from_millis(2600)).contains(&took),
                "seed {seed}: {took:?}"
            );
            assert!(steps.len() > 20, "seed {seed}: {} samples", steps.len());
            let gaps: Vec<_> = steps[1..steps.len() - 1].iter().map(|s| s.wait).collect();
            assert!(
                gaps.iter().all(|gap| *gap <= Duration::from_millis(200)),
                "seed {seed}: a sample gap is too long"
            );
            assert!(
                deviation(&steps, from, to) > 0.5,
                "seed {seed}: a straight line"
            );
        }
    }

    #[test]
    fn longer_and_smaller_targets_take_longer() {
        let average = |to: (f64, f64), size: (f64, f64)| {
            (0..100)
                .map(|seed| {
                    let pacing = HumanPacing::seeded(seed);
                    total(&pacing.reach(Some((50.0, 50.0)), to, size, VIEWPORT)).as_secs_f64()
                })
                .sum::<f64>()
                / 100.0
        };
        let near_big = average((150.0, 80.0), (200.0, 60.0));
        let far_big = average((1100.0, 800.0), (200.0, 60.0));
        let far_small = average((1100.0, 800.0), (12.0, 12.0));
        assert!(near_big < far_big && far_big < far_small);
    }

    #[test]
    fn a_pointer_new_to_the_page_comes_from_the_toolbar() {
        let pacing = HumanPacing::seeded(7);
        let steps = pacing.reach(None, (640.0, 500.0), (80.0, 30.0), VIEWPORT);
        assert!(steps[0].point.1 < 60.0, "first sample {:?}", steps[0].point);
    }

    #[test]
    fn aims_stay_inside_and_off_the_edges() {
        for seed in 0..300 {
            let pacing = HumanPacing::seeded(seed);
            let (u, v) = pacing.aim((200.0, 30.0));
            assert!((0.15..=0.85).contains(&u) && (0.2..=0.8).contains(&v));
            let (u, v) = pacing.aim((8.0, 8.0));
            assert!((0.45..=0.55).contains(&u) && (0.45..=0.55).contains(&v));
        }
        let centred = (0..300)
            .filter(|seed| HumanPacing::seeded(*seed).aim((200.0, 40.0)) == (0.5, 0.5))
            .count();
        assert_eq!(centred, 0, "a click never lands on the exact centre");
    }

    #[test]
    fn clicks_rest_before_pressing_and_hold_the_button() {
        for seed in 0..100 {
            let pacing = HumanPacing::seeded(seed);
            assert!((50..=600).contains(&pacing.settle().as_millis()));
            assert!((45..=220).contains(&pacing.hold().as_millis()));
        }
    }

    fn typed_waits(pacing: &HumanPacing, text: &str) -> Vec<(KeyEvent, Duration)> {
        let mut sticky = Modifiers::NONE;
        let events = convert_keys(text, &mut sticky, true).unwrap();
        let mut previous = None;
        events
            .iter()
            .map(|event| {
                let wait = pacing.key_wait(previous, event);
                previous = Some(event);
                (event.clone(), wait)
            })
            .collect()
    }

    #[test]
    fn typing_has_dwell_flight_and_a_shift_lead() {
        let pacing = HumanPacing::seeded(3);
        let waits = typed_waits(&pacing, "Hello world");
        assert_eq!(waits[0].0.key, "Shift");
        let chars: Vec<_> = waits
            .iter()
            .filter(|(event, _)| event.kind == KeyEventKind::Char)
            .collect();
        assert!(chars.iter().all(|(_, wait)| wait.is_zero()));
        let key_ups: Vec<_> = waits
            .iter()
            .filter(|(event, _)| event.kind == KeyEventKind::KeyUp && !is_modifier(event))
            .map(|(_, wait)| wait.as_millis())
            .collect();
        assert!(key_ups.iter().all(|wait| (35..=220).contains(wait)));
        let total: Duration = waits.iter().map(|(_, wait)| *wait).sum();
        assert!(
            (Duration::from_millis(900)..Duration::from_secs(12)).contains(&total),
            "eleven keys took {total:?}"
        );
    }

    #[test]
    fn key_rhythm_varies() {
        let pacing = HumanPacing::seeded(11);
        let waits: Vec<_> = typed_waits(&pacing, "the quick brown fox")
            .into_iter()
            .filter(|(event, _)| event.kind == KeyEventKind::RawKeyDown)
            .map(|(_, wait)| wait.as_millis())
            .collect();
        let distinct: std::collections::BTreeSet<_> = waits.iter().collect();
        assert!(distinct.len() > waits.len() / 2, "{waits:?}");
    }

    #[test]
    fn the_wheel_covers_the_distance_in_bursts() {
        for seed in 0..100 {
            let pacing = HumanPacing::seeded(seed);
            let steps = pacing.wheel(720.0);
            let moved: f64 = steps.iter().map(|step| step.delta.1).sum();
            let tolerance = pacing.persona.notch.map_or(1.0, |notch| notch / 2.0);
            assert!((moved - 720.0).abs() <= tolerance, "seed {seed}: {moved}");
            assert!(steps.iter().all(|step| step.delta.1 > 0.0));
            let up: f64 = pacing.wheel(-300.0).iter().map(|step| step.delta.1).sum();
            assert!(up < 0.0);
        }
        assert!(HumanPacing::seeded(1).wheel(0.4).is_empty());
    }

    #[test]
    fn idling_mixes_rest_and_motion_and_settles_down_over_time() {
        let count = |quiet: Duration| {
            (0..400)
                .map(|seed| {
                    HumanPacing::seeded(seed).idle(Some((600.0, 400.0)), VIEWPORT, quiet, 0.0)
                })
                .filter(|gesture| !matches!(gesture, Gesture::Rest(_)))
                .count()
        };
        let early = count(Duration::from_secs(3));
        let late = count(Duration::from_secs(100));
        assert!(early > 150 && early < 300, "early motion {early}");
        assert!(late < early, "late {late} vs early {early}");
        assert!(
            matches!(
                HumanPacing::seeded(1).idle(
                    Some((1.0, 1.0)),
                    VIEWPORT,
                    Duration::from_millis(100),
                    0.0
                ),
                Gesture::Rest(_)
            ),
            "right after an action the hand is still"
        );
    }

    #[test]
    fn idle_motion_stays_on_the_page_and_peeks_are_undone() {
        let mut unpeeks = 0;
        for seed in 0..400 {
            let pacing = HumanPacing::seeded(seed);
            match pacing.idle(
                Some((640.0, 450.0)),
                VIEWPORT,
                Duration::from_secs(5),
                120.0,
            ) {
                Gesture::Move(steps) => assert!(steps.iter().all(|step| {
                    (0.0..VIEWPORT.0).contains(&step.point.0)
                        && (0.0..VIEWPORT.1).contains(&step.point.1)
                })),
                Gesture::Unpeek(steps) => {
                    unpeeks += 1;
                    let moved: f64 = steps.iter().map(|step| step.delta.1).sum();
                    assert!(moved < 0.0);
                }
                Gesture::Peek(_) => panic!("seed {seed}: a second peek before the first is undone"),
                Gesture::Rest(_) => {}
            }
        }
        assert!(unpeeks > 100);
        let entering = HumanPacing::seeded(5);
        let first_motion = (0..50)
            .map(|_| entering.idle(None, VIEWPORT, Duration::from_secs(2), 0.0))
            .find(|gesture| !matches!(gesture, Gesture::Rest(_)));
        match first_motion {
            Some(Gesture::Move(steps)) => assert!(steps[0].point.1 < 60.0),
            other => panic!("expected the pointer to enter the page, got {other:?}"),
        }
    }
}
