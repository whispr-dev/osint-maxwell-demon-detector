use std::collections::{BTreeSet, HashMap};

use crate::error::EntropyError;
use crate::stats::{max_abs, robust_normalized_errors};

#[derive(Debug, Clone)]
pub struct WeightedEdge {
    pub src: String,
    pub dst: String,
    pub weight: f64,
}

#[derive(Debug, Clone)]
pub struct EntropyCentralityRow {
    pub node: String,
    pub probability: f64,
    pub out_weight: f64,
    pub centrality: f64,
}

#[derive(Debug, Clone)]
pub struct GraphEntropyReport {
    pub graph_entropy: f64,
    pub rows: Vec<EntropyCentralityRow>,
}

fn log_base(x: f64, base: f64) -> f64 {
    x.ln() / base.ln()
}

fn node_out_weights(edges: &[WeightedEdge]) -> HashMap<String, f64> {
    let mut weights = HashMap::new();
    for edge in edges {
        *weights.entry(edge.src.clone()).or_insert(0.0) += edge.weight.max(0.0);
        weights.entry(edge.dst.clone()).or_insert(0.0);
    }
    weights
}

pub fn graph_entropy(edges: &[WeightedEdge], base: f64) -> Result<f64, EntropyError> {
    if edges.is_empty() {
        return Err(EntropyError::EmptyInput);
    }
    if !base.is_finite() || base <= 0.0 || (base - 1.0).abs() < f64::EPSILON {
        return Err(EntropyError::InvalidArgument(
            "logarithm base must be finite, positive, and not equal to 1".to_string(),
        ));
    }

    let weights = node_out_weights(edges);
    let total = weights.values().sum::<f64>();
    if total <= 0.0 {
        return Err(EntropyError::InvalidArgument(
            "graph has no positive edge weight".to_string(),
        ));
    }

    let entropy = weights
        .values()
        .filter(|&&w| w > 0.0)
        .map(|&w| {
            let p = w / total;
            -p * log_base(p, base)
        })
        .sum();

    Ok(entropy)
}

pub fn entropy_centrality(edges: &[WeightedEdge], base: f64) -> Result<GraphEntropyReport, EntropyError> {
    if edges.is_empty() {
        return Err(EntropyError::EmptyInput);
    }

    let base_entropy = graph_entropy(edges, base)?;
    let weights = node_out_weights(edges);
    let total_weight = weights.values().sum::<f64>().max(1.0e-12);

    let mut all_nodes = BTreeSet::new();
    for edge in edges {
        all_nodes.insert(edge.src.clone());
        all_nodes.insert(edge.dst.clone());
    }

    let mut rows = Vec::new();
    for node in all_nodes {
        let pruned_edges = edges
            .iter()
            .filter(|e| e.src != node && e.dst != node)
            .cloned()
            .collect::<Vec<_>>();

        let pruned_entropy = if pruned_edges.is_empty() {
            0.0
        } else {
            graph_entropy(&pruned_edges, base)?
        };

        rows.push(EntropyCentralityRow {
            probability: weights.get(&node).copied().unwrap_or(0.0) / total_weight,
            out_weight: weights.get(&node).copied().unwrap_or(0.0),
            centrality: base_entropy - pruned_entropy,
            node,
        });
    }

    rows.sort_by(|a, b| b.centrality.total_cmp(&a.centrality));

    Ok(GraphEntropyReport {
        graph_entropy: base_entropy,
        rows,
    })
}

pub fn system_anomaly_score(errors: &[f64]) -> Result<f64, EntropyError> {
    let normalized = robust_normalized_errors(errors)?;
    max_abs(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_node_has_highest_entropy_centrality() {
        let edges = vec![
            WeightedEdge { src: "hub".into(), dst: "a".into(), weight: 3.0 },
            WeightedEdge { src: "hub".into(), dst: "b".into(), weight: 2.0 },
            WeightedEdge { src: "hub".into(), dst: "c".into(), weight: 1.0 },
            WeightedEdge { src: "a".into(), dst: "hub".into(), weight: 0.5 },
        ];
        let report = entropy_centrality(&edges, 2.0).unwrap();
        assert_eq!(report.rows[0].node, "hub");
    }
}
