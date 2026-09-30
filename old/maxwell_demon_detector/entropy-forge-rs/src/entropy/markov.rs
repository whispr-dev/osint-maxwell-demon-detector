use std::collections::HashMap;
use std::hash::Hash;

use crate::error::EntropyError;

pub fn first_order_markov_entropy_rate<T>(sequence: &[T], base: f64) -> Result<f64, EntropyError>
where
    T: Eq + Hash + Clone,
{
    if sequence.len() < 2 {
        return Err(EntropyError::InsufficientData {
            needed: 2,
            found: sequence.len(),
        });
    }
    if !base.is_finite() || base <= 0.0 || (base - 1.0).abs() < f64::EPSILON {
        return Err(EntropyError::InvalidArgument(
            "logarithm base must be finite, positive, and not equal to 1".to_string(),
        ));
    }

    let mut state_counts: HashMap<T, usize> = HashMap::new();
    let mut transitions: HashMap<T, HashMap<T, usize>> = HashMap::new();

    for window in sequence.windows(2) {
        let current = window[0].clone();
        let next = window[1].clone();
        *state_counts.entry(current.clone()).or_insert(0) += 1;
        *transitions
            .entry(current)
            .or_default()
            .entry(next)
            .or_insert(0) += 1;
    }

    let total_transitions = (sequence.len() - 1) as f64;
    let mut entropy_rate = 0.0;

    for (state, row) in transitions.iter() {
        let state_total = *state_counts.get(state).unwrap_or(&0) as f64;
        if state_total <= 0.0 {
            continue;
        }
        let pi = state_total / total_transitions;
        let row_entropy = row
            .values()
            .map(|&count| {
                let p = count as f64 / state_total;
                if p <= 0.0 {
                    0.0
                } else {
                    -p * (p.ln() / base.ln())
                }
            })
            .sum::<f64>();
        entropy_rate += pi * row_entropy;
    }

    Ok(entropy_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_sequence_has_zero_markov_entropy_rate() {
        let data = [0, 0, 0, 0, 0, 0];
        let h = first_order_markov_entropy_rate(&data, 2.0).unwrap();
        assert!(h.abs() < 1.0e-12);
    }
}
