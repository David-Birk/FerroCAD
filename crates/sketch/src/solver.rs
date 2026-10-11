//! Damped minimum-norm Gauss–Newton solver and degree-of-freedom analysis.

use crate::constraint::implicit_residuals;
use crate::linalg::{cholesky_solve, rref, Mat};
use crate::sketch::{Entity, EntityId, PointId, Sketch};

/// Residuals below this are considered solved (model units, i.e. mm).
pub const TOLERANCE: f64 = 1e-9;
const MAX_ITERATIONS: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct SolveReport {
    pub iterations: usize,
    pub residual: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SolveError {
    /// The constraints contradict each other (or the solver got stuck).
    /// The sketch is left exactly as it was before the call.
    DidNotConverge { residual: f64 },
}

impl std::fmt::Display for SolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveError::DidNotConverge { residual } => {
                write!(f, "constraints conflict (residual {residual:.3e})")
            }
        }
    }
}

impl std::error::Error for SolveError {}

/// Result of [`Sketch::analyze`].
#[derive(Clone, Debug)]
pub struct Analysis {
    /// Remaining degrees of freedom. 0 means fully constrained.
    pub dof: usize,
    /// Equations that add no information (redundant constraints).
    pub redundant: usize,
    /// Per parameter: is its value pinned down by the constraints?
    pub determined: Vec<bool>,
}

impl Analysis {
    pub fn is_fully_constrained(&self) -> bool {
        self.dof == 0
    }
}

impl Sketch {
    /// All residuals for parameter vector `p`.
    pub fn residuals_at(&self, p: &[f64]) -> Vec<f64> {
        let mut out = Vec::new();
        implicit_residuals(self, p, &mut out);
        for c in self.constraints.iter().flatten() {
            c.residuals(self, p, &mut out);
        }
        out
    }

    pub fn residual(&self) -> f64 {
        max_abs(&self.residuals_at(&self.params))
    }

    fn free_params(&self) -> Vec<usize> {
        (0..self.params.len()).filter(|&i| !self.fixed[i]).collect()
    }

    /// Central-difference Jacobian with respect to the free parameters.
    fn jacobian(&self, p: &[f64], free: &[usize], m: usize) -> Mat {
        let mut jac = Mat::zeros(m, free.len());
        let mut q = p.to_vec();
        for (col, &i) in free.iter().enumerate() {
            let h = 1e-7 * p[i].abs().max(1.0);
            q[i] = p[i] + h;
            let rp = self.residuals_at(&q);
            q[i] = p[i] - h;
            let rm = self.residuals_at(&q);
            q[i] = p[i];
            for row in 0..m {
                jac.set(row, col, (rp[row] - rm[row]) / (2.0 * h));
            }
        }
        jac
    }

    /// Move the geometry until every constraint holds. On failure the sketch
    /// is restored to its previous state.
    pub fn solve(&mut self) -> Result<SolveReport, SolveError> {
        let backup = self.params.clone();
        let result = self.solve_inner();
        if result.is_err() {
            self.params = backup;
        }
        result
    }

    fn solve_inner(&mut self) -> Result<SolveReport, SolveError> {
        let free = self.free_params();
        let mut r = self.residuals_at(&self.params);
        let mut err = norm2(&r);
        let mut lambda = 1e-9;

        for iteration in 0..MAX_ITERATIONS {
            if max_abs(&r) < TOLERANCE {
                return Ok(SolveReport { iterations: iteration, residual: max_abs(&r) });
            }
            if free.is_empty() {
                break;
            }
            let jac = self.jacobian(&self.params, &free, r.len());
            let jjt = jac.mul_transpose_self();
            let neg_r: Vec<f64> = r.iter().map(|v| -v).collect();

            // Minimum-norm step: δ = Jᵀ (J Jᵀ + λI)⁻¹ (−r). Increase damping
            // until the step actually reduces the error.
            let mut improved = false;
            while lambda < 1e10 {
                let mut a = jjt.clone();
                for i in 0..a.rows {
                    let v = a.get(i, i) + lambda;
                    a.set(i, i, v);
                }
                if let Some(y) = cholesky_solve(&a, &neg_r) {
                    let delta = jac.transpose_mul_vec(&y);
                    let mut trial = self.params.clone();
                    for (k, &i) in free.iter().enumerate() {
                        trial[i] += delta[k];
                    }
                    let r_trial = self.residuals_at(&trial);
                    let err_trial = norm2(&r_trial);
                    if err_trial < err && err_trial.is_finite() {
                        self.params = trial;
                        r = r_trial;
                        err = err_trial;
                        lambda = (lambda * 0.1).max(1e-12);
                        improved = true;
                        break;
                    }
                }
                lambda *= 10.0;
            }
            if !improved {
                break;
            }
        }

        let residual = max_abs(&r);
        if residual < TOLERANCE {
            Ok(SolveReport { iterations: MAX_ITERATIONS, residual })
        } else {
            Err(SolveError::DidNotConverge { residual })
        }
    }

    /// Drag a point towards `target` while keeping all constraints – used
    /// for interactive editing. The point is pinned during the solve.
    pub fn drag_point(&mut self, id: PointId, target: [f64; 2]) -> Result<SolveReport, SolveError> {
        let was_fixed = self.is_point_fixed(id);
        let old = self.point_pos(id);
        self.set_point_pos(id, target);
        self.set_point_fixed(id, true);
        let result = self.solve();
        self.set_point_fixed(id, was_fixed);
        if result.is_err() {
            self.set_point_pos(id, old);
        }
        result
    }

    /// Degrees of freedom and which parameters are fully determined.
    pub fn analyze(&self) -> Analysis {
        let free = self.free_params();
        let m = self.residuals_at(&self.params).len();
        let mut determined: Vec<bool> = self.fixed.clone();

        if free.is_empty() {
            return Analysis { dof: 0, redundant: m, determined };
        }
        if m == 0 {
            return Analysis { dof: free.len(), redundant: 0, determined };
        }

        let mut jac = self.jacobian(&self.params, &free, m);
        let scale = jac.data.iter().fold(0.0f64, |a, v| a.max(v.abs())).max(1.0);
        let pivots = rref(&mut jac, 1e-8 * scale);
        let rank = pivots.len();

        // Null-space basis: one vector per non-pivot column. A parameter is
        // determined iff every null vector has a zero component there.
        let non_pivot: Vec<usize> = (0..free.len()).filter(|c| !pivots.contains(c)).collect();
        let mut movable = vec![false; free.len()];
        for &f in &non_pivot {
            movable[f] = true;
            for (row, &pc) in pivots.iter().enumerate() {
                if jac.get(row, f).abs() > 1e-8 {
                    movable[pc] = true;
                }
            }
        }
        for (k, &i) in free.iter().enumerate() {
            determined[i] = !movable[k];
        }

        Analysis { dof: free.len() - rank, redundant: m - rank, determined }
    }

    pub fn is_point_determined(&self, analysis: &Analysis, id: PointId) -> bool {
        let p = self.point(id);
        analysis.determined[p.x.0] && analysis.determined[p.y.0]
    }

    /// Fully constrained entity: all its points (and radius) are determined.
    pub fn is_entity_determined(&self, analysis: &Analysis, id: EntityId) -> bool {
        let e = self.entity(id);
        let pts = e.points().into_iter().all(|p| self.is_point_determined(analysis, p));
        match *e {
            Entity::Circle { radius, .. } => pts && analysis.determined[radius.0],
            _ => pts,
        }
    }
}

fn norm2(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0, |a, x| a.max(x.abs()))
}
