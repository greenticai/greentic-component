use std::ffi::OsString;

use anyhow::{Error, Result, bail};
use clap::{Arg, ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand};

#[cfg(feature = "store")]
use crate::cmd::store::StoreCommand;
use crate::cmd::{
    self, build::BuildArgs, doctor::DoctorArgs, flow::FlowCommand, hash::HashArgs,
    inspect::InspectArgs, new::NewArgs, templates::TemplatesArgs, test::TestArgs,
    wizard::WizardCliArgs,
};
use crate::scaffold::engine::ScaffoldEngine;

#[derive(Parser, Debug)]
#[command(
    name = "greentic-component",
    about = "Toolkit for Greentic component developers",
    version,
    arg_required_else_help = true
)]
pub struct Cli {
    #[arg(long = "locale", value_name = "LOCALE", global = true)]
    locale: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Scaffold a new Greentic component project
    New(NewArgs),
    /// Component wizard helpers
    Wizard(Box<WizardCliArgs>),
    /// List available component templates
    Templates(TemplatesArgs),
    /// Run component doctor checks
    Doctor(DoctorArgs),
    /// Inspect manifests and describe payloads
    Inspect(InspectArgs),
    /// Recompute manifest hashes
    Hash(HashArgs),
    /// Build component wasm + update config flows
    Build(BuildArgs),
    /// Invoke a component locally with an in-memory state/secrets harness
    #[command(
        long_about = "Invoke a component locally with in-memory state/secrets. \
See docs/component-developer-guide.md for a walkthrough."
    )]
    Test(Box<TestArgs>),
    /// Flow utilities (config flow regeneration)
    #[command(subcommand)]
    Flow(FlowCommand),
    /// Interact with the component store
    #[cfg(feature = "store")]
    #[command(subcommand)]
    Store(StoreCommand),
}

pub fn main() -> Result<()> {
    let argv: Vec<OsString> = std::env::args_os().collect();
    cmd::i18n::init(cmd::i18n::cli_locale_from_argv(&argv));

    let mut command = localize_help(Cli::command(), true);
    let matches = match command.try_get_matches_from_mut(argv) {
        Ok(matches) => matches,
        Err(err) => err.exit(),
    };
    let cli = Cli::from_arg_matches(&matches).map_err(|err| Error::msg(err.to_string()))?;
    cmd::i18n::init(cli.locale.clone());
    let engine = ScaffoldEngine::new();
    match cli.command {
        Commands::New(args) => cmd::new::run(args, &engine),
        Commands::Wizard(command) => cmd::wizard::run_cli(*command),
        Commands::Templates(args) => cmd::templates::run(args, &engine),
        Commands::Doctor(args) => cmd::doctor::run(args).map_err(Error::new),
        Commands::Inspect(args) => {
            let result = cmd::inspect::run(&args)?;
            cmd::inspect::emit_warnings(&result.warnings);
            if args.strict && !result.warnings.is_empty() {
                bail!(
                    "component-inspect: {} warning(s) treated as errors (--strict)",
                    result.warnings.len()
                );
            }
            Ok(())
        }
        Commands::Hash(args) => cmd::hash::run(args),
        Commands::Build(args) => cmd::build::run(args),
        Commands::Test(args) => cmd::test::run(*args),
        Commands::Flow(flow_cmd) => cmd::flow::run(flow_cmd),
        #[cfg(feature = "store")]
        Commands::Store(store_cmd) => cmd::store::run(store_cmd),
    }
}

fn localize_help(mut command: clap::Command, is_root: bool) -> clap::Command {
    if let Some(about) = command.get_about().map(|s| s.to_string()) {
        command = command.about(cmd::i18n::tr_lit(&about));
    }
    if let Some(long_about) = command.get_long_about().map(|s| s.to_string()) {
        command = command.long_about(cmd::i18n::tr_lit(&long_about));
    }
    if let Some(before) = command.get_before_help().map(|s| s.to_string()) {
        command = command.before_help(cmd::i18n::tr_lit(&before));
    }
    if let Some(after) = command.get_after_help().map(|s| s.to_string()) {
        command = command.after_help(cmd::i18n::tr_lit(&after));
    }

    command = command
        .disable_help_subcommand(true)
        .disable_help_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help(cmd::i18n::tr_lit("Print help")),
        );
    if is_root {
        command = command.disable_version_flag(true).arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .action(ArgAction::Version)
                .help(cmd::i18n::tr_lit("Print version")),
        );
    }

    let arg_ids = command
        .get_arguments()
        .map(|arg| arg.get_id().clone())
        .collect::<Vec<_>>();
    for arg_id in arg_ids {
        command = command.mut_arg(arg_id, |arg| {
            let mut arg = arg;
            if let Some(help) = arg.get_help().map(ToString::to_string) {
                arg = arg.help(cmd::i18n::tr_lit(&help));
            }
            if let Some(long_help) = arg.get_long_help().map(ToString::to_string) {
                arg = arg.long_help(cmd::i18n::tr_lit(&long_help));
            }
            arg
        });
    }

    let sub_names = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_string())
        .collect::<Vec<_>>();
    for name in sub_names {
        command = command.mut_subcommand(name, |sub| localize_help(sub, false));
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_subcommand() {
        let cli = Cli::try_parse_from([
            "greentic-component",
            "--locale",
            "nl",
            "new",
            "--name",
            "demo",
            "--json",
        ])
        .expect("expected CLI to parse");
        assert_eq!(cli.locale.as_deref(), Some("nl"));
        match cli.command {
            Commands::New(args) => {
                assert_eq!(args.name, "demo");
                assert!(args.json);
                assert!(!args.no_check);
                assert!(!args.no_git);
            }
            _ => panic!("expected new args"),
        }
    }

    #[test]
    fn parses_wizard_command() {
        let cli = Cli::try_parse_from([
            "greentic-component",
            "wizard",
            "--mode",
            "doctor",
            "--execution",
            "dry-run",
            "--locale",
            "ar",
        ])
        .expect("expected CLI to parse");
        assert_eq!(cli.locale.as_deref(), Some("ar"));
        match cli.command {
            Commands::Wizard(args) => {
                assert!(matches!(
                    args.args.mode,
                    crate::cmd::wizard::RunMode::Doctor
                ));
                assert!(matches!(
                    args.args.execution,
                    crate::cmd::wizard::ExecutionMode::DryRun
                ));
            }
            _ => panic!("expected wizard args"),
        }
    }

    #[test]
    fn parses_wizard_legacy_new_command() {
        let cli = Cli::try_parse_from([
            "greentic-component",
            "wizard",
            "new",
            "wizard-smoke",
            "--out",
            "/tmp",
        ])
        .expect("expected CLI to parse");
        match cli.command {
            Commands::Wizard(args) => match args.command {
                Some(crate::cmd::wizard::WizardSubcommand::New(new_args)) => {
                    assert_eq!(new_args.name.as_deref(), Some("wizard-smoke"));
                    assert_eq!(new_args.out.as_deref(), Some(std::path::Path::new("/tmp")));
                }
                _ => panic!("expected wizard new subcommand"),
            },
            _ => panic!("expected wizard args"),
        }
    }

    #[test]
    fn parses_wizard_validate_command_alias() {
        let cli = Cli::try_parse_from([
            "greentic-component",
            "wizard",
            "validate",
            "--mode",
            "create",
        ])
        .expect("expected CLI to parse");
        match cli.command {
            Commands::Wizard(args) => assert!(matches!(
                args.command,
                Some(crate::cmd::wizard::WizardSubcommand::Validate(_))
            )),
            _ => panic!("expected wizard args"),
        }
    }

    #[test]
    fn parses_wizard_validate_flag() {
        let cli = Cli::try_parse_from([
            "greentic-component",
            "wizard",
            "--validate",
            "--mode",
            "doctor",
        ])
        .expect("expected CLI to parse");
        match cli.command {
            Commands::Wizard(args) => {
                assert!(args.args.validate);
                assert!(!args.args.apply);
                assert!(matches!(
                    args.args.mode,
                    crate::cmd::wizard::RunMode::Doctor
                ));
            }
            _ => panic!("expected wizard args"),
        }
    }

    #[test]
    fn parses_wizard_answers_aliases() {
        let cli = Cli::try_parse_from([
            "greentic-component",
            "wizard",
            "--answers",
            "in.json",
            "--emit-answers",
            "out.json",
            "--schema-version",
            "1.2.3",
            "--migrate",
        ])
        .expect("expected CLI to parse");
        match cli.command {
            Commands::Wizard(args) => {
                assert_eq!(
                    args.args.answers.as_deref(),
                    Some(std::path::Path::new("in.json"))
                );
                assert_eq!(
                    args.args.emit_answers.as_deref(),
                    Some(std::path::Path::new("out.json"))
                );
                assert_eq!(args.args.schema_version.as_deref(), Some("1.2.3"));
                assert!(args.args.migrate);
            }
            _ => panic!("expected wizard args"),
        }
    }

    #[cfg(feature = "store")]
    #[test]
    fn parses_store_fetch_command() {
        let cli = Cli::try_parse_from([
            "greentic-component",
            "--locale",
            "nl",
            "store",
            "fetch",
            "--out",
            "/tmp/out",
            "file:///tmp/component.wasm",
        ])
        .expect("expected CLI to parse");
        assert_eq!(cli.locale.as_deref(), Some("nl"));
        match cli.command {
            Commands::Store(crate::cmd::store::StoreCommand::Fetch(args)) => {
                assert_eq!(args.out, std::path::PathBuf::from("/tmp/out"));
                assert_eq!(args.source, "file:///tmp/component.wasm");
            }
            _ => panic!("expected store fetch args"),
        }
    }
}
