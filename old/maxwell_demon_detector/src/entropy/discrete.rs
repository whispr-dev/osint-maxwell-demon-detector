use std::collections::HashMap;
use std::hash::Hash;

use crate::error::EntropyError;

fn validate_base(base: f64) -> Result<(), EntropyError> {
    if !base.is_finite() || base <= 0.0 || (base - 1.0).abs() < f64::EPSILON {
        return Err(EntropyError::InvalidArgument(
            "logarithm base must be finite, positive, and not equal to 1".to_string(),
        ));
    }
    Ok(())
}

fn log_base(x: f64, base: f64) -> f64 {
    x.ln() / base.ln()
}

fn counts<T>(sequence: &[T]) -> Result<HashMap<T, usize>, EntropyError>
where
    T: Eq + Hash + Clone,
{
    if sequence.is_empty() {
        return Err(EntropyError::EmptyInput);
    }

    let mut map = HashMap::new();
    for item in sequence.iter().cloned() {
        *map.entry(item).or_insert(0) += 1;
    }
    Ok(map)
}

pub fn shannon_entropy<T>(sequence: &[T], base: f64) -> Result<f64, EntropyError>
where
    T: Eq + Hash + Clone,
{
    validate_base(base)?;
    let counts = counts(sequence)?;
    let n = sequence.len() as f64;

    let entropy = counts
        .values()
        .map(|&count| {
            let p = count as f64 / n;
            if p <= 0.0 {
                0.0
            } else {
                -p * log_base(p, base)
            }
        })
        .sum();

    Ok(entropy)
}

pub fn miller_madow_entropy<T>(sequence: &[T], base: f64) -> Result<f64, EntropyError>
where
    T: Eq + Hash + Clone,
{
    validate_base(base)?;
    let plugin = shannon_entropy(sequence, base)?;
    let counts = counts(sequence)?;
    let k = counts.len() as f64;
    let n = sequence.len() as f64;
    Ok(plugin + ((k - 1.0) / (2.0 * n * base.ln())))
}

pub fn renyi_entropy<T>(sequence: &[T], alpha: f64, base: f64) -> Result<f64, EntropyError>
where
    T: Eq + Hash + Clone,
{
    validate_base(base)?;
    if !alpha.is_finite() || alpha <= 0.0 || (alpha - 1.0).abs() < f64::EPSILON {
        return Err(EntropyError::InvalidArgument(
            "Rényi alpha must be positive and not equal to 1".to_string(),
        ));
    }

    let counts = counts(sequence)?;
    let n = sequence.len() as f64;
    let sum = counts
        .values()
        .map(|&count| {
            let p = count as f64 / n;
            p.powf(alpha)
        })
        .sum::<f64>();

    Ok((1.0 / (1.0 - alpha)) * log_base(sum, base))
}

pub fn joint_entropy<T, U>(left: &[T], right: &[U], base: f64) -> Result<f64, EntropyError>
where
    T: Eq + Hash + Clone,
    U: Eq + Hash + Clone,
{
    if left.len() != right.len() {
        return Err(EntropyError::InvalidArgument(
            "joint entropy inputs must have equal length".to_string(),
        ));
    }

    let joint = left
        .iter()
        .cloned()
        .zip(right.iter().cloned())
        .collect::<Vec<_>>();

    shannon_entropy(&joint, base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fair_coin_has_one_bit_of_entropy() {
        let data = ["H", "T", "H", "T"];
        let h = shannon_entropy(&data, 2.0).unwrap();
        assert!((h - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn deterministic_sequence_has_zero_entropy() {
        let data = [1, 1, 1, 1, 1];
        let h = shannon_entropy(&data, 2.0).unwrap();
        assert!(h.abs() < 1.0e-12);
    }
}
