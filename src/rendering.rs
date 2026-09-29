use std::{fmt::Write, path::Path};

use crate::MigrationPlan;

/// Renders the complete versioned machine-readable plan with a terminal newline.
pub fn render_plan_json(plan: &MigrationPlan) -> Result<Vec<u8>, serde_json::Error> {
    let mut json = serde_json::to_vec_pretty(plan)?;
    json.push(b'\n');
    Ok(json)
}

/// Renders a concise review summary for a saved migration plan.
#[must_use]
pub fn render_plan_summary(plan: &MigrationPlan, saved_path: &Path) -> String {
    let mut output = String::new();
    writeln!(output, "Migration plan v{}", plan.format_version())
        .expect("writing to a String cannot fail");

    for operation in plan.operations() {
        writeln!(
            output,
            "- {} table {}",
            operation.operation().as_str(),
            operation.table()
        )
        .expect("writing to a String cannot fail");
        writeln!(
            output,
            "  data effect: {}",
            operation.data_effect().as_str()
        )
        .expect("writing to a String cannot fail");
        writeln!(
            output,
            "  structural cost: {}",
            operation.structural_cost().as_str()
        )
        .expect("writing to a String cannot fail");
        writeln!(
            output,
            "  data dependent: {}",
            operation.is_data_dependent()
        )
        .expect("writing to a String cannot fail");
    }

    writeln!(output, "Saved plan: {}", saved_path.display())
        .expect("writing to a String cannot fail");
    output
}
