use anyhow::Result;
use clap::Parser;
use mdlr::cli::{Cli, Command, OutputFormat, SessionAction, TargetAction};
use mdlr::config;
use mdlr::extract::{extractor_for_path, supported_extensions, Extractor};
use mdlr::graph::{Edge, EdgeKind, Graph};
use mdlr::metrics::{BucketedMetrics, MetricsDisplay};
use mdlr::session::{Session, SessionStore, Target};
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = SessionStore::new()?;

    match cli.command {
        Command::Session { action } => handle_session(action, &store),
        Command::Target { action } => handle_target(action, &store),
        Command::Analyze { session, format } => handle_analyze(&session, format, &store),
        Command::Export { session, format } => handle_export(&session, format, &store),
    }
}

fn handle_session(action: SessionAction, store: &SessionStore) -> Result<()> {
    match action {
        SessionAction::New { name } => {
            store.create(&name)?;
            println!("Created session '{}'", name);
        }
        SessionAction::List => {
            let sessions = store.list()?;
            if sessions.is_empty() {
                println!("No sessions found");
            } else {
                println!("Sessions:");
                for session in sessions {
                    println!("  {}", session);
                }
            }
        }
        SessionAction::Delete { name } => {
            store.delete(&name)?;
            println!("Deleted session '{}'", name);
        }
        SessionAction::Show { name } => {
            let session = store.load(&name)?;
            print_session_info(&session);
        }
    }
    Ok(())
}

fn handle_target(action: TargetAction, store: &SessionStore) -> Result<()> {
    match action {
        TargetAction::Add { path, session } => {
            let mut sess = store.load(&session)?;
            let target = mdlr::cli::parse_target(&path);
            sess.add_target(target);
            store.save(&sess)?;
            println!("Added target '{}' to session '{}'", path, session);
        }
        TargetAction::List { session } => {
            let sess = store.load(&session)?;
            if sess.targets.is_empty() {
                println!("No targets in session '{}'", session);
            } else {
                println!("Targets in session '{}':", session);
                for target in &sess.targets {
                    println!("  {}", format_target(target));
                }
            }
        }
        TargetAction::Clear { session } => {
            let mut sess = store.load(&session)?;
            sess.clear_targets();
            store.save(&sess)?;
            println!("Cleared targets from session '{}'", session);
        }
    }
    Ok(())
}

fn handle_analyze(session_name: &str, format: OutputFormat, store: &SessionStore) -> Result<()> {
    let mut session = store.load(session_name)?;
    let config = config::load()?;

    let graph = build_graph(&session.targets)?;
    session.update_graph(graph.clone());
    store.save(&session)?;

    let metrics = mdlr::metrics::compute(&graph);

    match format {
        OutputFormat::Text => {
            println!("Analysis for session '{}'", session_name);
            println!();
            println!("Graph: {} units, {} edges", graph.units.len(), graph.edges.len());
            println!();
            let display = MetricsDisplay::new(&metrics, &config);
            print!("{}", display);
        }
        OutputFormat::Json => {
            let bucketed = BucketedMetrics::from_metrics(&metrics, &config);
            let output = serde_json::json!({
                "session": session_name,
                "units": graph.units.len(),
                "edges": graph.edges.len(),
                "metrics": {
                    "dag_density": {
                        "value": bucketed.dag_density.value,
                        "bucket": bucketed.dag_density.bucket,
                    },
                    "fan_in": {
                        "max": {
                            "value": bucketed.fan_in.max.value as usize,
                            "bucket": bucketed.fan_in.max.bucket,
                        },
                        "mean": {
                            "value": bucketed.fan_in.mean.value,
                            "bucket": bucketed.fan_in.mean.bucket,
                        },
                    },
                    "fan_out": {
                        "max": {
                            "value": bucketed.fan_out.max.value as usize,
                            "bucket": bucketed.fan_out.max.bucket,
                        },
                        "mean": {
                            "value": bucketed.fan_out.mean.value,
                            "bucket": bucketed.fan_out.mean.bucket,
                        },
                    }
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn handle_export(session_name: &str, format: OutputFormat, store: &SessionStore) -> Result<()> {
    let session = store.load(session_name)?;

    match format {
        OutputFormat::Json => {
            let json = mdlr::graph::serialize::to_json(&session.graph)?;
            println!("{}", json);
        }
        OutputFormat::Text => {
            println!("Graph for session '{}'", session_name);
            println!();
            println!("Units ({}):", session.graph.units.len());
            for unit in &session.graph.units {
                println!("  {} ({:?}) - {:?}", unit.id, unit.kind, unit.file);
            }
            println!();
            println!("Edges ({}):", session.graph.edges.len());
            for edge in &session.graph.edges {
                println!("  {} -> {} ({:?})", edge.from, edge.to, edge.kind);
            }
        }
    }

    Ok(())
}

fn build_graph(targets: &[Target]) -> Result<Graph> {
    let mut graph = Graph::new();
    let mut all_units = Vec::new();

    for target in targets {
        match target {
            Target::Directory(dir) => {
                collect_files_recursive(dir, &mut |path| {
                    if let Some(extractor) = extractor_for_path(path) {
                        if let Ok(units) = extract_file(path, extractor.as_ref()) {
                            all_units.extend(units);
                        }
                    }
                })?;
            }
            Target::File(path) => {
                if let Some(extractor) = extractor_for_path(path) {
                    let units = extract_file(path, extractor.as_ref())?;
                    all_units.extend(units);
                }
            }
            Target::Object { file, name } => {
                if let Some(extractor) = extractor_for_path(file) {
                    let units = extract_file(file, extractor.as_ref())?;
                    let filtered: Vec<_> = units
                        .into_iter()
                        .filter(|u| u.id.contains(name))
                        .collect();
                    all_units.extend(filtered);
                }
            }
        }
    }

    let unit_ids: std::collections::HashSet<_> = all_units.iter().map(|u| u.id.clone()).collect();

    for unit in &all_units {
        for call in &unit.calls {
            if unit_ids.contains(call) {
                graph.add_edge(Edge {
                    from: unit.id.clone(),
                    to: call.clone(),
                    kind: EdgeKind::Calls,
                });
            }
        }
    }

    for unit in all_units {
        graph.add_unit(unit);
    }

    Ok(graph)
}

fn extract_file(path: &Path, extractor: &dyn Extractor) -> Result<Vec<mdlr::graph::Unit>> {
    let source = fs::read_to_string(path)?;
    extractor.extract(&source, path)
}

fn collect_files_recursive<F>(dir: &Path, callback: &mut F) -> Result<()>
where
    F: FnMut(&Path),
{
    if !dir.is_dir() {
        return Ok(());
    }

    let extensions = supported_extensions();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            collect_files_recursive(&path, callback)?;
        } else if let Some(ext) = path.extension() {
            if extensions.contains(&ext.to_str().unwrap_or("")) {
                callback(&path);
            }
        }
    }

    Ok(())
}

fn print_session_info(session: &Session) {
    println!("Session: {}", session.id);
    println!("Created: {}", session.created_at);
    println!("Updated: {}", session.updated_at);
    println!("Targets: {}", session.targets.len());
    for target in &session.targets {
        println!("  {}", format_target(target));
    }
    println!("Graph: {} units, {} edges", session.graph.units.len(), session.graph.edges.len());
}

fn format_target(target: &Target) -> String {
    match target {
        Target::Directory(p) => format!("{} (directory)", p.display()),
        Target::File(p) => format!("{} (file)", p.display()),
        Target::Object { file, name } => format!("{}::{} (object)", file.display(), name),
    }
}
