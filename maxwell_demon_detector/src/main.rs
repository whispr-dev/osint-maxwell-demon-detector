use std::path::PathBuf;

use clap::{Parser, Subcommand};

use entropy_forge::entropy::discrete::{joint_entropy, miller_madow_entropy, renyi_entropy, shannon_entropy};
use entropy_forge::entropy::lz::lz76_entropy_rate_estimate;
use entropy_forge::entropy::markov::first_order_markov_entropy_rate;
use entropy_forge::entropy::sample_entropy::{default_tolerance, sample_entropy};
use entropy_forge::graph::centrality::{entropy_centrality, system_anomaly_score};
use entropy_forge::io::{read_edge_list, read_numeric_column, read_string_column, ColumnSelector};
use entropy_forge::signal::tde::estimate_delay_gaussian;
use entropy_forge::EntropyError;

#[derive(Debug, Parser)]
#[command(name = "entropy-forge")]
#[command(about = "Entropy estimation, TDE, and graph entropy centrality in Rust")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Discrete {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        column: String,
        #[arg(long, default_value_t = 2.0)]
        base: f64,
        #[arg(long)]
        renyi_alpha: Option<f64>,
        #[arg(long, default_value_t = false)]
        miller_madow: bool,
    },
    JointEntropy {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        left: String,
        #[arg(long)]
        right: String,
        #[arg(long, default_value_t = 2.0)]
        base: f64,
    },
    MarkovRate {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        column: String,
        #[arg(long, default_value_t = 2.0)]
        base: f64,
    },
    LzRate {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        column: String,
    },
    SampleEntropy {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        column: String,
        #[arg(long, default_value_t = 2)]
        m: usize,
        #[arg(long, default_value_t = 0.2)]
        r_ratio: f64,
    },
    Tde {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        reference: String,
        #[arg(long)]
        target: String,
        #[arg(long, default_value_t = 32)]
        max_lag: usize,
    },
    EntropyCentrality {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value = "src")]
        src_col: String,
        #[arg(long, default_value = "dst")]
        dst_col: String,
        #[arg(long, default_value = "weight")]
        weight_col: String,
        #[arg(long, default_value_t = 2.0)]
        base: f64,
    },
    AnomalyScore {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        column: String,
    },
}

fn run(cli: Cli) -> Result<(), EntropyError> {
    match cli.command {
        Commands::Discrete {
            input,
            column,
            base,
            renyi_alpha,
            miller_madow,
        } => {
            let data = read_string_column(input, &ColumnSelector::parse(&column))?;
            let shannon = shannon_entropy(&data, base)?;
            println!("shannon_entropy(base={base}): {shannon:.12}");
            if miller_madow {
                let corrected = miller_madow_entropy(&data, base)?;
                println!("miller_madow_entropy(base={base}): {corrected:.12}");
            }
            if let Some(alpha) = renyi_alpha {
                let renyi = renyi_entropy(&data, alpha, base)?;
                println!("renyi_entropy(alpha={alpha}, base={base}): {renyi:.12}");
            }
        }
        Commands::JointEntropy {
            input,
            left,
            right,
            base,
        } => {
            let x = read_string_column(&input, &ColumnSelector::parse(&left))?;
            let y = read_string_column(&input, &ColumnSelector::parse(&right))?;
            let h = joint_entropy(&x, &y, base)?;
            println!("joint_entropy(base={base}): {h:.12}");
        }
        Commands::MarkovRate { input, column, base } => {
            let data = read_string_column(input, &ColumnSelector::parse(&column))?;
            let h = first_order_markov_entropy_rate(&data, base)?;
            println!("first_order_markov_entropy_rate(base={base}): {h:.12}");
        }
        Commands::LzRate { input, column } => {
            let data = read_string_column(input, &ColumnSelector::parse(&column))?;
            let h = lz76_entropy_rate_estimate(&data)?;
            println!("lz76_entropy_rate_estimate(bits/symbol): {h:.12}");
        }
        Commands::SampleEntropy {
            input,
            column,
            m,
            r_ratio,
        } => {
            let data = read_numeric_column(input, &ColumnSelector::parse(&column))?;
            let r = default_tolerance(&data, r_ratio)?;
            let se = sample_entropy(&data, m, r)?;
            println!("sample_entropy(m={m}, r={r:.12}): {se:.12}");
        }
        Commands::Tde {
            input,
            reference,
            target,
            max_lag,
        } => {
            let reference_signal = read_numeric_column(&input, &ColumnSelector::parse(&reference))?;
            let target_signal = read_numeric_column(&input, &ColumnSelector::parse(&target))?;
            let result = estimate_delay_gaussian(&reference_signal, &target_signal, max_lag)?;
            println!("best_entropy_lag: {}", result.best_entropy_lag);
            println!("best_entropy_value: {:.12}", result.best_entropy_value);
            println!("best_correlation_lag: {}", result.best_correlation_lag);
            println!("best_correlation_value: {:.12}", result.best_correlation_value);
            println!("entropy_curve:");
            for (lag, value) in result.entropy_curve {
                println!("  lag={lag:>4} entropy={value:.12}");
            }
        }
        Commands::EntropyCentrality {
            input,
            src_col,
            dst_col,
            weight_col,
            base,
        } => {
            let edges = read_edge_list(
                input,
                &ColumnSelector::parse(&src_col),
                &ColumnSelector::parse(&dst_col),
                &ColumnSelector::parse(&weight_col),
            )?;
            let report = entropy_centrality(&edges, base)?;
            println!("graph_entropy(base={base}): {:.12}", report.graph_entropy);
            println!("node,probability,out_weight,centrality");
            for row in report.rows {
                println!(
                    "{},{:.12},{:.12},{:.12}",
                    row.node, row.probability, row.out_weight, row.centrality
                );
            }
        }
        Commands::AnomalyScore { input, column } => {
            let errors = read_numeric_column(input, &ColumnSelector::parse(&column))?;
            let score = system_anomaly_score(&errors)?;
            println!("system_anomaly_score: {score:.12}");
        }
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
