#![cfg(feature = "cli")]

use std::io::Write;

use anyhow::Result;
use clap::Args;

use crate::scaffold::engine::{ScaffoldEngine, TemplateDescriptor};

#[derive(Args, Debug, Clone)]
pub struct TemplatesArgs {
    /// Emit JSON instead of a table
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: TemplatesArgs, engine: &ScaffoldEngine) -> Result<()> {
    let templates = engine.templates()?;
    if args.json {
        print_json(&templates)?;
    } else {
        print_table(&templates);
    }
    Ok(())
}

fn print_json(templates: &[TemplateDescriptor]) -> Result<()> {
    let mut handle = std::io::stdout();
    serde_json::to_writer_pretty(&mut handle, templates)?;
    handle.write_all(b"\n").ok();
    Ok(())
}

fn print_table(templates: &[TemplateDescriptor]) {
    println!(
        "{:<24} {:<12} {:<32} SOURCE",
        "TEMPLATE", "LOCATION", "DESCRIPTION"
    );
    for tpl in templates {
        let description = tpl.description.as_deref().unwrap_or("-");
        println!(
            "{:<24} {:<12} {:<32} {}",
            tpl.id,
            tpl.location,
            truncate(description, 32),
            tpl.display_path()
        );
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut result = String::new();
    let limit = max.saturating_sub(3);
    for (idx, ch) in value.chars().enumerate() {
        if idx >= limit {
            result.push_str("...");
            return result;
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scaffold::engine::TemplateLocation;

    #[test]
    fn table_header_is_stable() {
        let templates = vec![];
        print_table(&templates);
    }

    #[test]
    fn truncate_leaves_short_strings_and_ellipsizes_long_ones() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghijklmnopqrstuvwxyz", 8), "abcde...");
    }

    #[test]
    fn print_json_serializes_template_fields() {
        let templates = vec![TemplateDescriptor {
            id: "rust-wasi".into(),
            location: TemplateLocation::BuiltIn,
            path: Some(std::path::PathBuf::from("/tmp/template")),
            description: Some("A template".into()),
            tags: vec!["wasm".into()],
        }];
        print_json(&templates).unwrap();
    }
}
