//! Robust planar homography: `cv::findHomography(..., RANSAC, ...)` from OpenCV.
//!
//! Upstream: OpenCV 4.10.0, commit 71d3237a093b60a27601c20e9ee6c3e52154e8b1,
//! <https://github.com/opencv/opencv>. Ported parts:
//! - `modules/calib3d/src/fundam.cpp`: `HomographyEstimatorCallback`
//!   (`checkSubset`, `runKernel`, `computeError`), `HomographyRefineCallback`,
//!   `scaleFor` and the RANSAC branch of `cv::findHomography`;
//! - `modules/calib3d/src/ptsetreg.cpp`: `RANSACUpdateNumIters` and
//!   `RANSACPointSetRegistrator` (`getSubset`, `findInliers`, `run`);
//! - `modules/calib3d/src/levmarq.cpp`: `LMSolverImpl::run`, which OpenCV
//!   translated from Miroslav Balda's LMSolve;
//! - `modules/calib3d/src/precomp.hpp`: `haveCollinearPoints`;
//! - `modules/core/include/opencv2/core/operations.hpp`: `RNG::next`/`uniform`.
//!
//! License: OpenCV 4.10.0 is distributed under the Apache License 2.0; these
//! files also carry OpenCV's BSD-style license header, Copyright (C) 2000-2008
//! Intel Corporation, (C) 2009 Willow Garage Inc., (C) 2013 OpenCV Foundation,
//! (C) 2015 Itseez Inc., and levmarq.cpp the BSD 2-clause notice Copyright (c)
//! 2007 Miroslav Balda.
//! Full texts are in THIRD_PARTY_NOTICES.md at the repository root.
//!
//! Modified: translated from C++ to Rust. Points and errors are f64 (OpenCV
//! keeps points and `computeError` in f32), eigen decompositions and the LM
//! linear solve use nalgebra's symmetric eigensolver in place of `cv::eigen`
//! and `DECOMP_EIG`, and the `Mat`/callback plumbing is reduced to functions.
//! The RANSAC, LM and normalization logic, constants and RNG are unchanged.
use nalgebra::{DMatrix, DVector, SMatrix, SymmetricEigen};

/// Row-major 3x3 homography, `h[8] == 1` after estimation.
pub(crate) type Homography = [f64; 9];

/// `cv::RNG`, multiply-with-carry, seeded like `RANSACPointSetRegistrator`.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = (self.0 as u32 as u64) * 4_164_903_690 + (self.0 >> 32);
        self.0 as u32
    }
    fn uniform(&mut self, a: usize, b: usize) -> usize {
        if a == b {
            a
        } else {
            (self.next() as usize) % (b - a) + a
        }
    }
}

fn scale_for(x: f64) -> f64 {
    if x.abs() > f32::EPSILON as f64 {
        1. / x
    } else {
        1.
    }
}

/// `haveCollinearPoints`: the last point lies on a line through two earlier
/// ones, or points coincide.
fn have_collinear_points(points: &[[f64; 2]]) -> bool {
    let Some(i) = points.len().checked_sub(1) else {
        return false;
    };
    for j in 0..i {
        let dx1 = points[j][0] - points[i][0];
        let dy1 = points[j][1] - points[i][1];
        for k in 0..j {
            let dx2 = points[k][0] - points[i][0];
            let dy2 = points[k][1] - points[i][1];
            if (dx2 * dy1 - dy2 * dx1).abs()
                <= f32::EPSILON as f64 * (dx1.abs() + dy1.abs() + dx2.abs() + dy2.abs())
            {
                return true;
            }
        }
    }
    false
}

/// `HomographyEstimatorCallback::checkSubset`.
fn check_subset(src: &[[f64; 2]], dst: &[[f64; 2]]) -> bool {
    if have_collinear_points(src) || have_collinear_points(dst) {
        return false;
    }
    if src.len() == 4 {
        const TT: [[usize; 3]; 4] = [[0, 1, 2], [1, 2, 3], [0, 2, 3], [0, 1, 3]];
        let det = |p: &[[f64; 2]], t: [usize; 3]| {
            SMatrix::<f64, 3, 3>::from_row_slice(&[
                p[t[0]][0], p[t[0]][1], 1., p[t[1]][0], p[t[1]][1], 1., p[t[2]][0], p[t[2]][1], 1.,
            ])
            .determinant()
        };
        let negative = TT
            .iter()
            .filter(|&&t| det(src, t) * det(dst, t) < 0.)
            .count();
        if negative != 0 && negative != 4 {
            return false;
        }
    }
    true
}

/// `HomographyEstimatorCallback::runKernel`: normalized DLT.
fn run_kernel(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Homography> {
    let count = src.len() as f64;
    let (mut c_m, mut c_dst) = ([0.; 2], [0.; 2]);
    for (s, d) in src.iter().zip(dst) {
        for k in 0..2 {
            c_m[k] += s[k];
            c_dst[k] += d[k];
        }
    }
    c_m = c_m.map(|v| v / count);
    c_dst = c_dst.map(|v| v / count);
    let (mut s_m, mut s_dst) = ([0.; 2], [0.; 2]);
    for (s, d) in src.iter().zip(dst) {
        for k in 0..2 {
            s_m[k] += (s[k] - c_m[k]).abs();
            s_dst[k] += (d[k] - c_dst[k]).abs();
        }
    }
    if s_dst.iter().chain(&s_m).any(|v| v.abs() < f64::EPSILON) {
        return None;
    }
    s_dst = s_dst.map(|v| count / v);
    s_m = s_m.map(|v| count / v);
    let inv_hnorm = SMatrix::<f64, 3, 3>::from_row_slice(&[
        1. / s_dst[0],
        0.,
        c_dst[0],
        0.,
        1. / s_dst[1],
        c_dst[1],
        0.,
        0.,
        1.,
    ]);
    let hnorm2 = SMatrix::<f64, 3, 3>::from_row_slice(&[
        s_m[0],
        0.,
        -c_m[0] * s_m[0],
        0.,
        s_m[1],
        -c_m[1] * s_m[1],
        0.,
        0.,
        1.,
    ]);
    let mut ltl = SMatrix::<f64, 9, 9>::zeros();
    for (s, d) in src.iter().zip(dst) {
        let (x, y) = ((d[0] - c_dst[0]) * s_dst[0], (d[1] - c_dst[1]) * s_dst[1]);
        let (sx, sy) = ((s[0] - c_m[0]) * s_m[0], (s[1] - c_m[1]) * s_m[1]);
        let lx = [sx, sy, 1., 0., 0., 0., -x * sx, -x * sy, -x];
        let ly = [0., 0., 0., sx, sy, 1., -y * sx, -y * sy, -y];
        for j in 0..9 {
            for k in 0..9 {
                ltl[(j, k)] += lx[j] * lx[k] + ly[j] * ly[k];
            }
        }
    }
    // cv::eigen sorts descending; V[8] is the eigenvector of the smallest eigenvalue.
    let eigen = SymmetricEigen::new(ltl);
    let smallest = eigen
        .eigenvalues
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))?
        .0;
    let v = eigen.eigenvectors.column(smallest);
    let h0 = SMatrix::<f64, 3, 3>::from_row_slice(v.as_slice());
    let h = inv_hnorm * h0 * hnorm2;
    let scale = scale_for(h[(2, 2)]);
    let out: Vec<f64> = h.transpose().iter().map(|v| v * scale).collect();
    let mut result = [0.; 9];
    result.copy_from_slice(&out);
    result.iter().all(|v| v.is_finite()).then_some(result)
}

/// `HomographyEstimatorCallback::computeError`: squared reprojection error.
fn compute_error(src: &[[f64; 2]], dst: &[[f64; 2]], h: &Homography) -> Vec<f64> {
    src.iter()
        .zip(dst)
        .map(|(m, d)| {
            let ww = 1. / (h[6] * m[0] + h[7] * m[1] + h[8]);
            let dx = (h[0] * m[0] + h[1] * m[1] + h[2]) * ww - d[0];
            let dy = (h[3] * m[0] + h[4] * m[1] + h[5]) * ww - d[1];
            dx * dx + dy * dy
        })
        .collect()
}

/// `RANSACUpdateNumIters`.
fn ransac_update_num_iters(p: f64, ep: f64, model_points: i32, max_iters: usize) -> usize {
    let p = p.clamp(0., 1.);
    let ep = ep.clamp(0., 1.);
    let num = (1. - p).max(f64::MIN_POSITIVE);
    let denom = 1. - (1. - ep).powi(model_points);
    if denom < f64::MIN_POSITIVE {
        return 0;
    }
    let (num, denom) = (num.ln(), denom.ln());
    if denom >= 0. || -num >= max_iters as f64 * (-denom) {
        max_iters
    } else {
        (num / denom).round() as usize
    }
}

/// `RANSACPointSetRegistrator::run` with `modelPoints = 4`. Returns the best
/// minimal-sample model and its inlier mask.
fn ransac(
    src: &[[f64; 2]],
    dst: &[[f64; 2]],
    threshold: f64,
    confidence: f64,
    max_iters: usize,
) -> Option<(Homography, Vec<bool>)> {
    const MODEL_POINTS: usize = 4;
    let count = src.len();
    if count < MODEL_POINTS {
        return None;
    }
    if count == MODEL_POINTS {
        return run_kernel(src, dst).map(|h| (h, vec![true; count]));
    }
    let mut rng = Rng(u64::MAX);
    let mut niters = max_iters.max(1);
    let mut best: Option<(Homography, Vec<bool>)> = None;
    let mut max_good = 0;
    let t = threshold * threshold;
    let mut iter = 0;
    while iter < niters {
        // getSubset with maxAttempts = 10000
        let mut subset = None;
        for _ in 0..10_000 {
            let mut idx = [0usize; MODEL_POINTS];
            for i in 0..MODEL_POINTS {
                let mut candidate = rng.uniform(0, count);
                while idx[..i].contains(&candidate) {
                    candidate = rng.uniform(0, count);
                }
                idx[i] = candidate;
            }
            let ms1 = idx.map(|i| src[i]);
            let ms2 = idx.map(|i| dst[i]);
            if check_subset(&ms1, &ms2) {
                subset = Some((ms1, ms2));
                break;
            }
        }
        let Some((ms1, ms2)) = subset else {
            if iter == 0 {
                return None;
            }
            break;
        };
        iter += 1;
        let Some(model) = run_kernel(&ms1, &ms2) else {
            continue;
        };
        let mask: Vec<bool> = compute_error(src, dst, &model)
            .into_iter()
            .map(|e| e <= t)
            .collect();
        let good = mask.iter().filter(|&&m| m).count();
        if good > max_good.max(MODEL_POINTS - 1) {
            max_good = good;
            best = Some((model, mask));
            niters = ransac_update_num_iters(
                confidence,
                (count - good) as f64 / count as f64,
                MODEL_POINTS as i32,
                niters,
            );
        }
    }
    best
}

/// `HomographyRefineCallback::compute`: residuals and Jacobian over 9 params.
fn refine_residuals(
    src: &[[f64; 2]],
    dst: &[[f64; 2]],
    h: &DVector<f64>,
    jacobian: bool,
) -> (DVector<f64>, Option<DMatrix<f64>>) {
    let n = src.len();
    let mut err = DVector::zeros(n * 2);
    let mut jac = jacobian.then(|| DMatrix::zeros(n * 2, 9));
    for (i, (m, d)) in src.iter().zip(dst).enumerate() {
        let (mx, my) = (m[0], m[1]);
        let ww = h[6] * mx + h[7] * my + h[8];
        let ww = if ww.abs() > f64::EPSILON { 1. / ww } else { 0. };
        let xi = (h[0] * mx + h[1] * my + h[2]) * ww;
        let yi = (h[3] * mx + h[4] * my + h[5]) * ww;
        err[i * 2] = xi - d[0];
        err[i * 2 + 1] = yi - d[1];
        if let Some(j) = jac.as_mut() {
            let r = i * 2;
            j[(r, 0)] = mx * ww;
            j[(r, 1)] = my * ww;
            j[(r, 2)] = ww;
            j[(r, 6)] = -mx * ww * xi;
            j[(r, 7)] = -my * ww * xi;
            j[(r, 8)] = -ww * xi;
            j[(r + 1, 3)] = mx * ww;
            j[(r + 1, 4)] = my * ww;
            j[(r + 1, 5)] = ww;
            j[(r + 1, 6)] = -mx * ww * yi;
            j[(r + 1, 7)] = -my * ww * yi;
            j[(r + 1, 8)] = -ww * yi;
        }
    }
    (err, jac)
}

/// Solve `a x = b` for symmetric `a` like `cv::solve(..., DECOMP_EIG)`:
/// pseudo-inverse over the eigen basis, dropping vanishing eigenvalues.
fn solve_eig(a: &DMatrix<f64>, b: &DVector<f64>) -> DVector<f64> {
    let eigen = SymmetricEigen::new(a.clone());
    let largest = eigen.eigenvalues.iter().fold(0f64, |m, v| m.max(v.abs()));
    let mut x = DVector::zeros(b.len());
    for (k, &lambda) in eigen.eigenvalues.iter().enumerate() {
        if lambda.abs() > largest * f64::EPSILON * a.nrows() as f64 {
            let v = eigen.eigenvectors.column(k);
            x += v * (v.dot(b) / lambda);
        }
    }
    x
}

/// `LMSolverImpl::run` with `HomographyRefineCallback`, eps = FLT_EPSILON.
fn refine(src: &[[f64; 2]], dst: &[[f64; 2]], h: &mut Homography, max_iters: usize) {
    let eps = f32::EPSILON as f64;
    let mut x = DVector::from_row_slice(h);
    let (mut r, jac) = refine_residuals(src, dst, &x, true);
    let Some(mut j) = jac else { return };
    let mut s = r.norm_squared();
    let mut a = j.transpose() * &j;
    let mut v = j.transpose() * &r;
    let diag = a.diagonal();
    let (r_lo, r_hi) = (0.25, 0.75);
    let (mut lambda, mut lc) = (1f64, 0.75f64);
    let mut iter = 0;
    loop {
        let mut ap = a.clone();
        for i in 0..9 {
            ap[(i, i)] += lambda * diag[i];
        }
        let d = solve_eig(&ap, &v);
        let xd = &x - &d;
        let (rd, _) = refine_residuals(src, dst, &xd, false);
        let sd = rd.norm_squared();
        let temp_d = &a * &d * -1. + &v * 2.;
        let ds = d.dot(&temp_d);
        let ratio = (s - sd) / if ds.abs() > f64::EPSILON { ds } else { 1. };
        if ratio > r_hi {
            lambda *= 0.5;
            if lambda < lc {
                lambda = 0.;
            }
        } else if ratio < r_lo {
            let t = d.dot(&v);
            let mut nu = (sd - s) / if t.abs() > f64::EPSILON { t } else { 1. } + 2.;
            nu = nu.clamp(2., 10.);
            if lambda == 0. {
                let inverse = a.clone().pseudo_inverse(f64::EPSILON).unwrap_or(a.clone());
                let maxval = (0..9).fold(f64::EPSILON, |m, i| m.max(inverse[(i, i)].abs()));
                lambda = 1. / maxval;
                lc = lambda;
                nu *= 0.5;
            }
            lambda *= nu;
        }
        if sd < s {
            s = sd;
            x = xd;
            let (rn, jn) = refine_residuals(src, dst, &x, true);
            let Some(jn) = jn else { break };
            r = rn;
            j = jn;
            a = j.transpose() * &j;
            v = j.transpose() * &r;
        }
        iter += 1;
        let proceed = iter < max_iters && d.amax() >= eps && r.amax() >= eps;
        if !proceed {
            break;
        }
    }
    if x.iter().all(|v| v.is_finite()) {
        h.copy_from_slice(x.as_slice());
    }
}

/// `cv::findHomography(src, dst, RANSAC, threshold, mask, max_iters, confidence)`:
/// RANSAC over minimal samples, a DLT on all inliers, then 10 LM iterations
/// on the inliers. Returns the homography (`h[8] == 1`) and inlier mask.
pub(crate) fn find_homography(
    src: &[[f64; 2]],
    dst: &[[f64; 2]],
    threshold: f64,
    max_iters: usize,
    confidence: f64,
) -> Option<(Homography, Vec<bool>)> {
    if src.len() != dst.len() || src.len() < 4 {
        return None;
    }
    let (mut h, mask) = ransac(src, dst, threshold, confidence, max_iters)?;
    if src.len() > 4 {
        let (inl_src, inl_dst): (Vec<_>, Vec<_>) = src
            .iter()
            .zip(dst)
            .zip(&mask)
            .filter(|(_, &m)| m)
            .map(|((s, d), _)| (*s, *d))
            .unzip();
        if !inl_src.is_empty() {
            if let Some(all) = run_kernel(&inl_src, &inl_dst) {
                h = all;
            }
            refine(&inl_src, &inl_dst, &mut h, 10);
            let scale = scale_for(h[8]);
            h = h.map(|v| v * scale);
        }
    }
    Some((h, mask))
}
