//! Constant-velocity Kalman filter over `[cx, cy, w, h, vcx, vcy, vw, vh]` (BoT-SORT
//! `KalmanFilterXYWH`). Velocities are in pixels per reference frame of 1/30 s; `predict` takes
//! the elapsed time in reference frames, so a 30 fps stream reproduces BoT-SORT exactly.

const FRAME_MS: f64 = 1000.0 / 30.0;
const MAX_DT_FRAMES: f64 = 150.0;
const STD_WEIGHT_POSITION: f64 = 1.0 / 20.0;
const STD_WEIGHT_VELOCITY: f64 = 1.0 / 160.0;
const MIN_SIZE: f64 = 1e-6;

/// Box measurement `[cx, cy, w, h]`.
pub type Measurement = [f64; 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KalmanError {
    /// Non-finite coordinates or a non-positive width or height
    InvalidMeasurement,
    /// The innovation covariance has no Cholesky factorization
    NotPositiveDefinite,
    /// The update would leave a non-finite mean or covariance
    NonFinite,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KalmanState {
    mean: [f64; 8],
    covariance: [[f64; 8]; 8],
}

/// Elapsed time in reference frames of 1/30 s, clamped to [0, 150].
pub fn dt_frames(dt_ms: i64) -> f64 {
    (dt_ms as f64 / FRAME_MS).clamp(0.0, MAX_DT_FRAMES)
}

pub fn is_valid_measurement(measurement: &Measurement) -> bool {
    measurement.iter().all(|v| v.is_finite()) && measurement[2] > 0.0 && measurement[3] > 0.0
}

impl KalmanState {
    /// Starts a track at `measurement` with zero velocity. `None` for an invalid measurement.
    pub fn initiate(measurement: Measurement) -> Option<Self> {
        if !is_valid_measurement(&measurement) {
            return None;
        }
        let [_, _, w, h] = measurement;
        let position = 2.0 * STD_WEIGHT_POSITION;
        let velocity = 10.0 * STD_WEIGHT_VELOCITY;
        let std = [
            position * w,
            position * h,
            position * w,
            position * h,
            velocity * w,
            velocity * h,
            velocity * w,
            velocity * h,
        ];
        let mut mean = [0.0; 8];
        mean[..4].copy_from_slice(&measurement);
        let mut covariance = [[0.0; 8]; 8];
        for (i, s) in std.iter().enumerate() {
            covariance[i][i] = s * s;
        }
        Some(Self { mean, covariance })
    }

    /// Advances the state by `dt_frames` reference frames. Process noise grows linearly with the
    /// elapsed time; `dt_frames = 0` leaves the state unchanged. A step that would produce a
    /// non-finite state is dropped.
    pub fn predict(&mut self, dt_frames: f64) {
        let dt = if dt_frames.is_finite() {
            dt_frames.clamp(0.0, MAX_DT_FRAMES)
        } else {
            0.0
        };
        if dt == 0.0 {
            return;
        }

        let sizes = self.noise_sizes();
        let mut mean = self.mean;
        let (position, velocity) = mean.split_at_mut(4);
        for (p, v) in position.iter_mut().zip(velocity.iter()) {
            *p += dt * v;
        }

        let p = &self.covariance;
        let mut fp = *p;
        for i in 0..4 {
            for j in 0..8 {
                fp[i][j] = p[i][j] + dt * p[i + 4][j];
            }
        }
        let mut covariance = fp;
        for (row, fp_row) in covariance.iter_mut().zip(&fp) {
            for j in 0..4 {
                row[j] = fp_row[j] + dt * fp_row[j + 4];
            }
        }
        for (i, size) in sizes.iter().enumerate() {
            covariance[i][i] += (STD_WEIGHT_POSITION * size).powi(2) * dt;
            covariance[i + 4][i + 4] += (STD_WEIGHT_VELOCITY * size).powi(2) * dt;
        }

        if is_finite_state(&mean, &covariance) {
            self.mean = mean;
            self.covariance = covariance;
            self.clamp_size();
        }
    }

    /// Corrects the state with `measurement`. On error the state is left unchanged.
    pub fn update(&mut self, measurement: Measurement) -> Result<(), KalmanError> {
        if !is_valid_measurement(&measurement) {
            return Err(KalmanError::InvalidMeasurement);
        }

        let p = &self.covariance;
        let sizes = self.noise_sizes();
        let mut innovation_cov = [[0.0; 4]; 4];
        for (i, row) in innovation_cov.iter_mut().enumerate() {
            row.copy_from_slice(&p[i][..4]);
            row[i] += (STD_WEIGHT_POSITION * sizes[i]).powi(2);
        }
        let factor = cholesky(&innovation_cov).ok_or(KalmanError::NotPositiveDefinite)?;

        let mut gain = [[0.0; 4]; 8];
        for (gain_row, p_row) in gain.iter_mut().zip(p) {
            *gain_row = cholesky_solve(&factor, [p_row[0], p_row[1], p_row[2], p_row[3]]);
        }

        let innovation: [f64; 4] = std::array::from_fn(|i| measurement[i] - self.mean[i]);
        let mean: [f64; 8] = std::array::from_fn(|k| self.mean[k] + dot4(&gain[k], &innovation));
        let corrected: [[f64; 8]; 8] = std::array::from_fn(|a| {
            std::array::from_fn(|b| p[a][b] - dot4(&[p[a][0], p[a][1], p[a][2], p[a][3]], &gain[b]))
        });
        let covariance: [[f64; 8]; 8] = std::array::from_fn(|a| {
            std::array::from_fn(|b| 0.5 * (corrected[a][b] + corrected[b][a]))
        });

        if !is_finite_state(&mean, &covariance) {
            return Err(KalmanError::NonFinite);
        }
        self.mean = mean;
        self.covariance = covariance;
        self.clamp_size();
        Ok(())
    }

    /// Stops size changes, as BoT-SORT does before predicting a lost track.
    pub fn freeze_size(&mut self) {
        self.mean[6] = 0.0;
        self.mean[7] = 0.0;
    }

    /// `[x1, y1, x2, y2]` of the current mean.
    pub fn xyxy(&self) -> [f64; 4] {
        let [cx, cy, w, h, ..] = self.mean;
        [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
    }

    /// Box-center velocity in pixels per second.
    pub fn velocity_per_second(&self) -> (f64, f64) {
        let frames_per_second = 1000.0 / FRAME_MS;
        (
            self.mean[4] * frames_per_second,
            self.mean[5] * frames_per_second,
        )
    }

    fn noise_sizes(&self) -> [f64; 4] {
        let w = self.mean[2].max(MIN_SIZE);
        let h = self.mean[3].max(MIN_SIZE);
        [w, h, w, h]
    }

    fn clamp_size(&mut self) {
        for (size, velocity) in [(2, 6), (3, 7)] {
            if self.mean[size] < MIN_SIZE {
                self.mean[size] = MIN_SIZE;
                self.mean[velocity] = self.mean[velocity].max(0.0);
            }
        }
    }
}

fn is_finite_state(mean: &[f64; 8], covariance: &[[f64; 8]; 8]) -> bool {
    mean.iter()
        .chain(covariance.iter().flatten())
        .all(|v| v.is_finite())
}

fn dot4(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Lower Cholesky factor of a symmetric matrix, or `None` when it is not positive definite.
fn cholesky(matrix: &[[f64; 4]; 4]) -> Option<[[f64; 4]; 4]> {
    let mut lower = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..=i {
            let sum = matrix[i][j] - (0..j).map(|k| lower[i][k] * lower[j][k]).sum::<f64>();
            if i == j {
                if !sum.is_finite() || sum <= 0.0 {
                    return None;
                }
                lower[i][i] = sum.sqrt();
            } else {
                lower[i][j] = sum / lower[j][j];
            }
        }
    }
    Some(lower)
}

/// Solves `L Lᵀ x = b` for the lower Cholesky factor `L`.
fn cholesky_solve(lower: &[[f64; 4]; 4], b: [f64; 4]) -> [f64; 4] {
    let mut y = [0.0; 4];
    for i in 0..4 {
        let sum = b[i] - (0..i).map(|k| lower[i][k] * y[k]).sum::<f64>();
        y[i] = sum / lower[i][i];
    }
    let mut x = [0.0; 4];
    for i in (0..4).rev() {
        let sum = y[i] - ((i + 1)..4).map(|k| lower[k][i] * x[k]).sum::<f64>();
        x[i] = sum / lower[i][i];
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * (1.0 + a.abs().max(b.abs()))
    }

    #[test]
    fn initiate_matches_bot_sort() {
        let state = KalmanState::initiate([100.0, 50.0, 20.0, 40.0]).unwrap();
        assert_eq!(state.mean, [100.0, 50.0, 20.0, 40.0, 0.0, 0.0, 0.0, 0.0]);
        assert!(close(state.covariance[0][0], 4.0));
        assert!(close(state.covariance[1][1], 16.0));
        assert!(close(state.covariance[4][4], 1.5625));
        assert!(close(state.covariance[5][5], 6.25));
        assert_eq!(state.covariance[0][4], 0.0);
        assert!(KalmanState::initiate([0.0, 0.0, 0.0, 10.0]).is_none());
        assert!(KalmanState::initiate([f64::NAN, 0.0, 10.0, 10.0]).is_none());
    }

    #[test]
    fn one_frame_predict_matches_bot_sort() {
        let mut state = KalmanState::initiate([100.0, 50.0, 20.0, 40.0]).unwrap();
        state.mean[4] = 3.0;
        state.mean[5] = -2.0;
        state.predict(1.0);
        assert!(close(state.mean[0], 103.0));
        assert!(close(state.mean[1], 48.0));
        assert!(close(state.covariance[0][0], 4.0 + 1.5625 + 1.0));
        assert!(close(state.covariance[0][4], 1.5625));
        assert!(close(state.covariance[4][0], 1.5625));
        assert!(close(state.covariance[4][4], 1.5625 + 0.015625));
    }

    #[test]
    fn predict_scales_with_elapsed_time() {
        let mut base = KalmanState::initiate([0.0, 0.0, 20.0, 20.0]).unwrap();
        base.mean[4] = 2.0;

        let mut idle = base.clone();
        idle.predict(0.0);
        assert_eq!(idle, base);
        idle.predict(f64::NAN);
        assert_eq!(idle, base);

        let mut one = base.clone();
        one.predict(1.0);
        let mut three = base.clone();
        three.predict(3.0);
        assert!(close(three.mean[0], 6.0));
        assert!(three.covariance[0][0] > one.covariance[0][0]);
        assert!(three.covariance[4][4] > one.covariance[4][4]);

        let mut capped = base.clone();
        capped.predict(1e9);
        assert!(close(capped.mean[0], 2.0 * MAX_DT_FRAMES));
    }

    #[test]
    fn dt_frames_is_time_based_and_clamped() {
        assert!(close(dt_frames(1000), 30.0));
        assert!((dt_frames(33) - 0.99).abs() < 1e-9);
        assert_eq!(dt_frames(0), 0.0);
        assert_eq!(dt_frames(-500), 0.0);
        assert_eq!(dt_frames(i64::MAX), MAX_DT_FRAMES);
    }

    #[test]
    fn update_pulls_mean_towards_measurement() {
        let mut state = KalmanState::initiate([100.0, 100.0, 20.0, 40.0]).unwrap();
        state.predict(1.0);
        let before = state.covariance[0][0];
        state.update([110.0, 100.0, 20.0, 40.0]).unwrap();
        assert!(state.mean[0] > 100.0 && state.mean[0] < 110.0);
        assert!(state.mean[4] > 0.0);
        assert!(state.covariance[0][0] < before);
        for a in 0..8 {
            for b in 0..8 {
                assert_eq!(state.covariance[a][b], state.covariance[b][a]);
            }
        }
    }

    #[test]
    fn learns_constant_velocity() {
        let mut state = KalmanState::initiate([0.0, 0.0, 30.0, 60.0]).unwrap();
        for frame in 1..=40 {
            state.predict(1.0);
            state.update([5.0 * frame as f64, 0.0, 30.0, 60.0]).unwrap();
        }
        let (vx, vy) = state.velocity_per_second();
        assert!((vx - 150.0).abs() < 1.0, "vx = {vx}");
        assert!(vy.abs() < 1.0);
        state.predict(2.0);
        assert!((state.mean[0] - 210.0).abs() < 0.5);
    }

    #[test]
    fn invalid_updates_leave_state_untouched() {
        let mut state = KalmanState::initiate([10.0, 10.0, 5.0, 5.0]).unwrap();
        let original = state.clone();
        assert_eq!(
            state.update([f64::NAN, 10.0, 5.0, 5.0]),
            Err(KalmanError::InvalidMeasurement)
        );
        assert_eq!(
            state.update([10.0, 10.0, 0.0, 5.0]),
            Err(KalmanError::InvalidMeasurement)
        );
        assert_eq!(state, original);

        state.covariance[1][1] = -1e6;
        let broken = state.clone();
        assert_eq!(
            state.update([10.0, 10.0, 5.0, 5.0]),
            Err(KalmanError::NotPositiveDefinite)
        );
        assert_eq!(state, broken);
    }

    #[test]
    fn non_finite_predictions_are_dropped() {
        let mut state = KalmanState::initiate([10.0, 10.0, 5.0, 5.0]).unwrap();
        state.mean[4] = f64::MAX;
        let original = state.clone();
        state.predict(10.0);
        assert_eq!(state, original);
    }

    #[test]
    fn size_never_collapses_below_zero() {
        let mut state = KalmanState::initiate([10.0, 10.0, 5.0, 5.0]).unwrap();
        state.mean[6] = -10.0;
        state.predict(MAX_DT_FRAMES);
        assert!(state.mean[2] > 0.0);
        assert!(state.mean[6] >= 0.0);
        assert!(state.update([10.0, 10.0, 5.0, 5.0]).is_ok());
        assert!(state.xyxy().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn freeze_size_stops_size_velocity() {
        let mut state = KalmanState::initiate([10.0, 10.0, 5.0, 5.0]).unwrap();
        state.mean[4] = 1.0;
        state.mean[6] = 1.0;
        state.mean[7] = 1.0;
        state.freeze_size();
        state.predict(4.0);
        assert!(close(state.mean[0], 14.0));
        assert!(close(state.mean[2], 5.0));
        assert!(close(state.mean[3], 5.0));
    }

    #[test]
    fn cholesky_solve_inverts() {
        let matrix = [
            [4.0, 1.0, 0.0, 0.5],
            [1.0, 3.0, 0.2, 0.0],
            [0.0, 0.2, 2.0, 0.1],
            [0.5, 0.0, 0.1, 1.0],
        ];
        let factor = cholesky(&matrix).unwrap();
        let x = cholesky_solve(&factor, [1.0, 2.0, 3.0, 4.0]);
        for (row, expected) in matrix.iter().zip([1.0, 2.0, 3.0, 4.0]) {
            assert!(close(dot4(row, &x), expected));
        }
        let mut singular = matrix;
        singular[3] = [0.0; 4];
        assert!(cholesky(&singular).is_none());
    }
}
