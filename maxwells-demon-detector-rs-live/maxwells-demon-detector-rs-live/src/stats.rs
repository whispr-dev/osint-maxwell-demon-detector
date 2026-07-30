pub fn shannon_entropy_u64(counts: &[u64]) -> f64 {
    let total: u64 = counts.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let total_f = total as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / total_f;
            -p * p.log2()
        })
        .sum()
}

pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

pub fn stddev(values: &[f64], mu: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let var = values
        .iter()
        .map(|v| {
            let d = *v - mu;
            d * d
        })
        .sum::<f64>()
        / values.len() as f64;
    var.sqrt()
}

pub fn normalized_entropy(counts: &[u64]) -> f64 {
    if counts.len() < 2 {
        return 0.0;
    }
    let h = shannon_entropy_u64(counts);
    let max_h = (counts.len() as f64).log2();
    if max_h <= 0.0 {
        0.0
    } else {
        (h / max_h).clamp(0.0, 1.0)
    }
}
