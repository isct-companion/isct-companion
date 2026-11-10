use super::opencv_homography::find_homography;

fn project(h: &[f64; 9], [x, y]: [f64; 2]) -> [f64; 2] {
    let w = h[6] * x + h[7] * y + h[8];
    [
        (h[0] * x + h[1] * y + h[2]) / w,
        (h[3] * x + h[4] * y + h[5]) / w,
    ]
}

#[test]
fn ransac_homography_recovers_a_projective_map_through_outliers_and_rejects_degenerate_sets() {
    let truth = [1.8, 0.3, 120., -0.2, 1.4, 60., 0.0006, -0.0004, 1.];
    let model: Vec<[f64; 2]> = (0..30)
        .map(|i| [(i % 6) as f64 * 40., (i / 6) as f64 * 35.])
        .collect();
    let mut image: Vec<[f64; 2]> = model.iter().map(|&p| project(&truth, p)).collect();
    // A third of the correspondences are gross outliers.
    for (k, p) in image.iter_mut().enumerate().filter(|(k, _)| k % 3 == 0) {
        p[0] += 40. + k as f64;
        p[1] -= 25.;
    }
    let (h, mask) = find_homography(&model, &image, 1., 2000, 0.999).unwrap();
    for (k, (&m, &inlier)) in model.iter().zip(&mask).enumerate() {
        assert_eq!(inlier, k % 3 != 0, "point {k}");
        let [x, y] = project(&h, m);
        let [tx, ty] = project(&truth, m);
        assert!((x - tx).hypot(y - ty) < 1e-6, "point {k}");
    }
    assert!((h[8] - 1.).abs() < 1e-12);
    // Too few, or all on one line: no model.
    assert!(find_homography(&model[..3], &image[..3], 1., 100, 0.99).is_none());
    let line: Vec<[f64; 2]> = (0..8).map(|i| [i as f64, 2. * i as f64]).collect();
    assert!(find_homography(&line, &line, 1., 100, 0.99).is_none());
}
