use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::ExitCode,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sqlite_schema::{
    SchemaSource, diff_schemas, inspect_schema, plan_migration, render_plan_json,
    render_plan_summary,
};

#[derive(Debug, Parser)]
#[command(name = "sqlite-schema")]
#[command(about = "Plan reviewable SQLite schema changes")]
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
    }
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
