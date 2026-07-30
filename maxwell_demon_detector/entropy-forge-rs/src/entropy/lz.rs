use std::collections::HashSet;
use std::hash::Hash;

use crate::error::EntropyError;

pub fn lz76_complexity<T>(sequence: &[T]) -> Result<usize, EntropyError>
where
    T: Eq + Hash + Clone,
{
    if sequence.is_empty() {
        return Err(EntropyError::EmptyInput);
    }

    let n = sequence.len();
    let mut dictionary: HashSet<Vec<T>> = HashSet::new();
    let mut i = 0;
    let mut complexity = 0;

    while i < n {
        let mut length = 1;
        let mut inserted = false;

        while i + length <= n {
            let phrase = sequence[i..i + length].to_vec();
            if !dictionary.contains(&phrase) {
                dictionary.insert(phrase);
                complexity += 1;
                i += length;
                inserted = true;
                break;
            }
            length += 1;
        }

        if !inserted {
            complexity += 1;
            break;
        }
    }

    Ok(complexity)
}

pub fn lz76_entropy_rate_estimate<T>(sequence: &[T]) -> Result<f64, EntropyError>
where
    T: Eq + Hash + Clone,
{
    let n = sequence.len();
    if n < 2 {
        return Err(EntropyError::InsufficientData { needed: 2, found: n });
    }
    let c = lz76_complexity(sequence)? as f64;
    Ok(c * (n as f64).log2() / n as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz_complexity_is_positive() {
        let data = ["a", "b", "a", "b", "c", "a"];
        let c = lz76_complexity(&data).unwrap();
        assert!(c > 0);
    }
}
