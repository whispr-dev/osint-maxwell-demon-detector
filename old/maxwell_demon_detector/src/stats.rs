use crate::EntropyError;

pub fn mean(values: &[f64]) -> Result<f64, EntropyError> {
    if values.is_empty() {
        return Err(EntropyError::EmptyInput);
    }
    Ok(values.iter().sum::<f64>() / values.len() as f64)
}

pub fn variance(values: &[f64]) -> Result<f64, EntropyError> {
    if values.len() < 2 {
        return Err(EntropyError::InsufficientData {
            needed: 2,
            found: values.len(),
        });
    }
    let mu = mean(values)?;
    let sum_sq = values.iter().map(|x| (x - mu).powi(2)).sum::<f64>();
    Ok(sum_sq / (values.len() as f64 - 1.0))
}

pub fn stddev(values: &[f64]) -> Result<f64, EntropyError> {
    Ok(variance(values)?.sqrt())
}

pub fn median(values: &[f64]) -> Result<f64, EntropyError> {
    if values.is_empty() {
        return Err(EntropyError::EmptyInput);
    }
    let mut data = values.to_vec();
    data.sort_by(|a, b| a.total_cmp(b));
    let mid = data.len() / 2;
    if data.len() % 2 == 0 {
        Ok((data[mid - 1] + data[mid]) / 2.0)
    } else {
        Ok(data[mid])
    }
}

pub fn covariance(x: &[f64], y: &[f64]) -> Result<f64, EntropyError> {
    if x.len() != y.len() {
        return Err(EntropyError::InvalidArgument(
            "covariance inputs must have equal length".to_string(),
        ));
    }
    if x.len() < 2 {
        return Err(EntropyError::InsufficientData {
            needed: 2,
            found: x.len(),
        });
    }

    let mx = mean(x)?;
    let my = mean(y)?;
    let sum = x
        .iter()
        .zip(y.iter())
        .map(|(a, b)| (a - mx) * (b - my))
        .sum::<f64>();
    Ok(sum / (x.len() as f64 - 1.0))
}

pub fn normalized_correlation(x: &[f64], y: &[f64]) -> Result<f64, EntropyError> {
    if x.len() != y.len() {
        return Err(EntropyError::InvalidArgument(
            "correlation inputs must have equal length".to_string(),
        ));
    }
    if x.len() < 2 {
        return Err(EntropyError::InsufficientData {
            needed: 2,
            found: x.len(),
        });
    }

    let mx = mean(x)?;
    let my = mean(y)?;

    let mut num = 0.0;
    let mut dx = 0.0;
    let mut dy = 0.0;

    for (a, b) in x.iter().zip(y.iter()) {
        let ax = a - mx;
        let by = b - my;
        num += ax * by;
        dx += ax * ax;
        dy += by * by;
    }

    let denom = (dx * dy).sqrt();
    if denom <= f64::EPSILON {
        return Err(EntropyError::NonPositiveVariance);
    }

    Ok(num / denom)
}

pub fn robust_normalized_errors(errors: &[f64]) -> Result<Vec<f64>, EntropyError> {
    if errors.is_empty() {
        return Err(EntropyError::EmptyInput);
    }
    let med = median(errors)?;
    let sigma = stddev(errors)?;
    let scale = sigma.max(1.0e-12);
    Ok(errors.iter().map(|e| (e - med) / scale).collect())
}

pub fn max_abs(values: &[f64]) -> Result<f64, EntropyError> {
    values
        .iter()
        .map(|v| v.abs())
        .max_by(|a, b| a.total_cmp(b))
        .ok_or(EntropyError::EmptyInput)
}
