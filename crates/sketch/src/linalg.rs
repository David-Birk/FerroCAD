//! Minimal dense linear algebra – enough for sketches with a few hundred
//! unknowns. Swap for a sparse solver once sketches get big.

/// Row-major dense matrix.
#[derive(Clone, Debug)]
pub(crate) struct Mat {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl Mat {
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self { rows, cols, data: vec![0.0; rows * cols] }
    }

    #[inline]
    pub fn get(&self, r: usize, c: usize) -> f64 {
        self.data[r * self.cols + c]
    }

    #[inline]
    pub fn set(&mut self, r: usize, c: usize, v: f64) {
        self.data[r * self.cols + c] = v;
    }

    /// `self * selfᵀ` (rows × rows).
    pub fn mul_transpose_self(&self) -> Mat {
        let mut out = Mat::zeros(self.rows, self.rows);
        for i in 0..self.rows {
            let ri = &self.data[i * self.cols..(i + 1) * self.cols];
            for j in 0..=i {
                let rj = &self.data[j * self.cols..(j + 1) * self.cols];
                let s: f64 = ri.iter().zip(rj).map(|(a, b)| a * b).sum();
                out.set(i, j, s);
                out.set(j, i, s);
            }
        }
        out
    }

    /// `selfᵀ * v` (length cols).
    pub fn transpose_mul_vec(&self, v: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.cols];
        for r in 0..self.rows {
            let vr = v[r];
            if vr == 0.0 {
                continue;
            }
            for c in 0..self.cols {
                out[c] += self.get(r, c) * vr;
            }
        }
        out
    }
}

/// Solve `a x = b` for symmetric positive definite `a` via Cholesky.
/// Returns `None` if `a` is not (numerically) positive definite.
pub(crate) fn cholesky_solve(a: &Mat, b: &[f64]) -> Option<Vec<f64>> {
    let n = a.rows;
    let mut l = Mat::zeros(n, n);
    for i in 0..n {
        for j in 0..=i {
            let mut s = a.get(i, j);
            for k in 0..j {
                s -= l.get(i, k) * l.get(j, k);
            }
            if i == j {
                if s <= 0.0 || !s.is_finite() {
                    return None;
                }
                l.set(i, i, s.sqrt());
            } else {
                l.set(i, j, s / l.get(j, j));
            }
        }
    }
    // forward: L y = b
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut s = b[i];
        for k in 0..i {
            s -= l.get(i, k) * y[k];
        }
        y[i] = s / l.get(i, i);
    }
    // backward: Lᵀ x = y
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = y[i];
        for k in i + 1..n {
            s -= l.get(k, i) * x[k];
        }
        x[i] = s / l.get(i, i);
    }
    Some(x)
}

/// Reduced row echelon form in place. Returns the pivot column of every
/// non-zero row; its length is the rank.
pub(crate) fn rref(m: &mut Mat, tol: f64) -> Vec<usize> {
    let mut pivots = Vec::new();
    let mut row = 0;
    for col in 0..m.cols {
        if row == m.rows {
            break;
        }
        // partial pivoting
        let (best, best_val) = (row..m.rows)
            .map(|r| (r, m.get(r, col).abs()))
            .fold((row, -1.0), |acc, x| if x.1 > acc.1 { x } else { acc });
        if best_val <= tol {
            continue;
        }
        if best != row {
            for c in 0..m.cols {
                m.data.swap(best * m.cols + c, row * m.cols + c);
            }
        }
        let pv = m.get(row, col);
        for c in 0..m.cols {
            let v = m.get(row, c) / pv;
            m.set(row, c, v);
        }
        for r in 0..m.rows {
            if r != row {
                let f = m.get(r, col);
                if f != 0.0 {
                    for c in 0..m.cols {
                        let v = m.get(r, c) - f * m.get(row, c);
                        m.set(r, c, v);
                    }
                }
            }
        }
        pivots.push(col);
        row += 1;
    }
    pivots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_solves_spd_system() {
        let a = Mat { rows: 2, cols: 2, data: vec![4.0, 1.0, 1.0, 3.0] };
        let x = cholesky_solve(&a, &[1.0, 2.0]).unwrap();
        assert!((4.0 * x[0] + x[1] - 1.0).abs() < 1e-12);
        assert!((x[0] + 3.0 * x[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn rref_finds_rank() {
        let mut m = Mat { rows: 3, cols: 3, data: vec![1., 2., 3., 2., 4., 6., 0., 1., 1.] };
        assert_eq!(rref(&mut m, 1e-10).len(), 2);
    }
}
