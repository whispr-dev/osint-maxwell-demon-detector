use crate::error::EntropyError;
use crate::stats::stddev;

fn chebyshev_distance(x: &[f64], y: &[f64]) -> f64 {
    x.iter()
        .zip(y.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max)
}

pub fn default_tolerance(series: &[f64], ratio: f64) -> Result<f64, EntropyError> {
    if ratio <= 0.0 || !ratio.is_finite() {
        return Err(EntropyError::InvalidArgument(
            "r ratio must be positive and finite".to_string(),
        ));
    }
    let sigma = stddev(series)?;
    Ok((ratio * sigma).max(1.0e-12))
}

pub fn sample_entropy(series: &[f64], m: usize, r: f64) -> Result<f64, EntropyError> {
    if m == 0 {
        return Err(EntropyError::InvalidArgument(
            "embedding dimension m must be positive".to_string(),
        ));
    }
    if !r.is_finite() || r <= 0.0 {
        return Err(EntropyError::InvalidArgument(
            "tolerance r must be positive and finite".to_string(),
        ));
    }
    if series.len() <= m + 1 {
        return Err(EntropyError::InsufficientData {
            needed: m + 2,
            found: series.len(),
        });
    }

    let count_matches = |embed: usize| -> usize {
        let mut matches = 0usize;
        let limit = series.len() - embed + 1;
        for i in 0..limit {
            for j in (i + 1)..limit {
                let xi = &series[i..i + embed];
                let xj = &series[j..j + embed];
                if chebyshev_distance(xi, xj) <= r {
                    matches += 1;
                }
            }
        }
        matches
    };

    let b = count_matches(m);
    let a = count_matches(m + 1);

    if b == 0 {
        return Err(EntropyError::InvalidArgument(
            "sample entropy is undefined because there are no template matches for m"
                .to_string(),
        ));
    }
    if a == 0 {
        return Ok(f64::INFINITY);
    }

    Ok(-((a as f64) / (b as f64)).ln())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_signal_has_low_sample_entropy() {
        let series = vec![1.0; 20];
        let r = default_tolerance(&series, 0.2).unwrap();
        let se = sample_entropy(&series, 2, r).unwrap();
        assert!(se < 0.2);
    }
}
