use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::ExitCode,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sqlite_schema::{
    SchemaSource, apply_migration, diff_schemas, inspect_schema, parse_plan_json, plan_migration,
    render_plan_json, render_plan_summary,
};

#[derive(Debug, Parser)]
#[command(name = "sqlite-schema")]
#[command(about = "Plan and apply reviewable SQLite schema changes")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate and save a migration plan without changing the target database.
    Plan {
        /// Existing SQLite database whose schema is the current state.
        database: PathBuf,
        /// SQL file defining the desired schema.
        #[arg(long, value_name = "SCHEMA_SQL")]
        file: PathBuf,
        /// Destination for the versioned JSON plan.
        #[arg(long, value_name = "PLAN_JSON")]
        output: PathBuf,
    },
    /// Apply a saved migration plan after verifying the target schema.
    Apply {
        /// Existing SQLite database to update.
        database: PathBuf,
        /// Saved versioned migration plan.
        #[arg(long, value_name = "PLAN_JSON")]
        plan: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Plan {
            database,
            file,
            output,
        } => run_plan(database, file, output),
        Command::Apply { database, plan } => run_apply(database, plan),
    }
}

fn run_apply(database: PathBuf, plan_path: PathBuf) -> Result<()> {
    let contents = fs::read(&plan_path)
        .with_context(|| format!("failed to read migration plan {}", plan_path.display()))?;
    let plan = parse_plan_json(&contents)
        .with_context(|| format!("failed to load migration plan {}", plan_path.display()))?;
    apply_migration(&database, &plan)
        .with_context(|| format!("failed to apply migration to {}", database.display()))?;

    println!(
        "Applied plan {} to {}",
        plan_path.display(),
        database.display()
    );
    Ok(())
}

fn run_plan(database: PathBuf, schema_file: PathBuf, output: PathBuf) -> Result<()> {
    let schema_sql = fs::read_to_string(&schema_file).with_context(|| {
        format!(
            "failed to read desired schema SQL from {}",
            schema_file.display()
        )
    })?;
    let current = inspect_schema(SchemaSource::Database(&database))
        .with_context(|| format!("failed to inspect current database {}", database.display()))?;
    let desired = inspect_schema(SchemaSource::Sql(&schema_sql)).with_context(|| {
        format!(
            "failed to inspect desired schema from {}",
            schema_file.display()
        )
    })?;
    let difference = diff_schemas(&current, &desired).context("failed to compare schemas")?;
    let plan =
        plan_migration(&current, &desired, difference).context("failed to build migration plan")?;
    let json = render_plan_json(&plan).context("failed to serialize migration plan")?;
    save_new_plan(&output, &json)?;

    print!("{}", render_plan_summary(&plan, &output));
    Ok(())
}

fn save_new_plan(output: &PathBuf, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| {
            format!(
                "refusing to overwrite existing plan destination {}",
                output.display()
            )
        })?;

    if let Err(source) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(output);
        return Err(source)
            .with_context(|| format!("failed to save migration plan to {}", output.display()));
    }

    Ok(())
}
