use std::f64::consts::{E, PI};

use nalgebra::Matrix2;

use crate::error::EntropyError;
use crate::stats::{covariance, normalized_correlation, variance};

#[derive(Debug, Clone)]
pub struct TdeResult {
    pub best_entropy_lag: isize,
    pub best_entropy_value: f64,
    pub entropy_curve: Vec<(isize, f64)>,
    pub best_correlation_lag: isize,
    pub best_correlation_value: f64,
}

fn align<'a>(reference: &'a [f64], target: &'a [f64], lag: isize) -> Result<(&'a [f64], &'a [f64]), EntropyError> {
    if reference.is_empty() || target.is_empty() {
        return Err(EntropyError::EmptyInput);
    }

    let lag_abs = lag.unsigned_abs();
    if lag_abs >= reference.len() || lag_abs >= target.len() {
        return Err(EntropyError::InsufficientData {
            needed: lag_abs + 1,
            found: reference.len().min(target.len()),
        });
    }

    let (x, y) = if lag >= 0 {
        let keep = reference.len().min(target.len().saturating_sub(lag_abs));
        (&reference[..keep], &target[lag_abs..lag_abs + keep])
    } else {
        let keep = reference.len().saturating_sub(lag_abs).min(target.len());
        (&reference[lag_abs..lag_abs + keep], &target[..keep])
    };

    if x.len() < 4 || y.len() < 4 {
        return Err(EntropyError::InsufficientData {
            needed: 4,
            found: x.len().min(y.len()),
        });
    }

    Ok((x, y))
}

fn gaussian_joint_entropy_2d(x: &[f64], y: &[f64]) -> Result<f64, EntropyError> {
    let var_x = variance(x)?.max(1.0e-12);
    let var_y = variance(y)?.max(1.0e-12);
    let cov_xy = covariance(x, y)?;
    let mut sigma = Matrix2::new(var_x, cov_xy, cov_xy, var_y);
    sigma[(0, 0)] += 1.0e-12;
    sigma[(1, 1)] += 1.0e-12;

    let det = sigma.determinant();
    if det <= 0.0 || !det.is_finite() {
        return Err(EntropyError::SingularMatrix);
    }

    let dimension = 2.0;
    Ok(0.5 * (((2.0 * PI * E).ln() * dimension) + det.ln()))
}

pub fn estimate_delay_gaussian(
    reference: &[f64],
    target: &[f64],
    max_lag: usize,
) -> Result<TdeResult, EntropyError> {
    if reference.len() != target.len() {
        return Err(EntropyError::InvalidArgument(
            "reference and target must have equal length".to_string(),
        ));
    }
    if max_lag == 0 {
        return Err(EntropyError::InvalidArgument(
            "max_lag must be positive".to_string(),
        ));
    }

    let mut entropy_curve = Vec::new();
    let mut best_entropy_lag = 0isize;
    let mut best_entropy_value = f64::INFINITY;
    let mut best_correlation_lag = 0isize;
    let mut best_correlation_value = f64::NEG_INFINITY;

    for lag in -(max_lag as isize)..=(max_lag as isize) {
        let (x, y) = align(reference, target, lag)?;
        let entropy = gaussian_joint_entropy_2d(x, y)?;
        let corr = normalized_correlation(x, y)?.abs();

        entropy_curve.push((lag, entropy));

        if entropy < best_entropy_value {
            best_entropy_value = entropy;
            best_entropy_lag = lag;
        }
        if corr > best_correlation_value {
            best_correlation_value = corr;
            best_correlation_lag = lag;
        }
    }

    Ok(TdeResult {
        best_entropy_lag,
        best_entropy_value,
        entropy_curve,
        best_correlation_lag,
        best_correlation_value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_known_positive_lag() {
        let reference = vec![0.0, 1.0, 2.0, 3.0, 2.0, 1.0, 0.0, -1.0, -2.0, -1.0, 0.0];
        let target = vec![9.0, 9.0, 0.0, 1.0, 2.0, 3.0, 2.0, 1.0, 0.0, -1.0, -2.0];
        let result = estimate_delay_gaussian(&reference, &target, 3).unwrap();
        assert_eq!(result.best_entropy_lag, 2);
    }
}
