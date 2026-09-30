use std::path::Path;

use crate::error::EntropyError;
use crate::graph::centrality::WeightedEdge;

#[derive(Debug, Clone)]
pub enum ColumnSelector {
    Name(String),
    Index(usize),
}

impl ColumnSelector {
    pub fn parse(raw: &str) -> Self {
        match raw.parse::<usize>() {
            Ok(index) => Self::Index(index),
            Err(_) => Self::Name(raw.to_string()),
        }
    }
}

fn resolve_column(headers: &csv::StringRecord, selector: &ColumnSelector) -> Result<usize, EntropyError> {
    match selector {
        ColumnSelector::Index(index) => {
            if *index < headers.len() {
                Ok(*index)
            } else {
                Err(EntropyError::ColumnNotFound(index.to_string()))
            }
        }
        ColumnSelector::Name(name) => headers
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| EntropyError::ColumnNotFound(name.clone())),
    }
}

pub fn read_string_column<P: AsRef<Path>>(
    path: P,
    selector: &ColumnSelector,
) -> Result<Vec<String>, EntropyError> {
    let mut rdr = csv::Reader::from_path(path)?;
    let headers = rdr.headers()?.clone();
    let index = resolve_column(&headers, selector)?;

    let mut values = Vec::new();
    for record in rdr.records() {
        let record = record?;
        let value = record
            .get(index)
            .ok_or_else(|| EntropyError::ColumnNotFound(index.to_string()))?;
        values.push(value.to_string());
    }

    if values.is_empty() {
        return Err(EntropyError::EmptyInput);
    }

    Ok(values)
}

pub fn read_numeric_column<P: AsRef<Path>>(
    path: P,
    selector: &ColumnSelector,
) -> Result<Vec<f64>, EntropyError> {
    let mut rdr = csv::Reader::from_path(path)?;
    let headers = rdr.headers()?.clone();
    let index = resolve_column(&headers, selector)?;
    let column_name = headers
        .get(index)
        .map(|s| s.to_string())
        .unwrap_or_else(|| index.to_string());

    let mut values = Vec::new();
    for record in rdr.records() {
        let record = record?;
        let raw = record
            .get(index)
            .ok_or_else(|| EntropyError::ColumnNotFound(index.to_string()))?;
        let value = raw.parse::<f64>().map_err(|_| EntropyError::ParseFloat {
            column: column_name.clone(),
            value: raw.to_string(),
        })?;
        values.push(value);
    }

    if values.is_empty() {
        return Err(EntropyError::EmptyInput);
    }

    Ok(values)
}

pub fn read_edge_list<P: AsRef<Path>>(
    path: P,
    src_col: &ColumnSelector,
    dst_col: &ColumnSelector,
    weight_col: &ColumnSelector,
) -> Result<Vec<WeightedEdge>, EntropyError> {
    let mut rdr = csv::Reader::from_path(path)?;
    let headers = rdr.headers()?.clone();
    let src_index = resolve_column(&headers, src_col)?;
    let dst_index = resolve_column(&headers, dst_col)?;
    let weight_index = resolve_column(&headers, weight_col)?;
    let weight_name = headers
        .get(weight_index)
        .map(|s| s.to_string())
        .unwrap_or_else(|| weight_index.to_string());

    let mut edges = Vec::new();
    for record in rdr.records() {
        let record = record?;
        let src = record
            .get(src_index)
            .ok_or_else(|| EntropyError::ColumnNotFound(src_index.to_string()))?
            .to_string();
        let dst = record
            .get(dst_index)
            .ok_or_else(|| EntropyError::ColumnNotFound(dst_index.to_string()))?
            .to_string();
        let raw_weight = record
            .get(weight_index)
            .ok_or_else(|| EntropyError::ColumnNotFound(weight_index.to_string()))?;
        let weight = raw_weight.parse::<f64>().map_err(|_| EntropyError::ParseFloat {
            column: weight_name.clone(),
            value: raw_weight.to_string(),
        })?;
        edges.push(WeightedEdge { src, dst, weight });
    }

    if edges.is_empty() {
        return Err(EntropyError::EmptyInput);
    }

    Ok(edges)
}
