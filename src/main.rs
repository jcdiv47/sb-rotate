use anyhow::{Context, Result, ensure};
use clap::Parser;
use sb_rotate::{
    apply, binding,
    cli::{Cli, Command},
    config::ConfigSet,
    plan, protocol, recovery,
    singbox::{Executable, require_supported},
};

fn run(cli: Cli) -> Result<()> {
    if let Command::Recover {
        journal,
        directory,
        dry_run,
    } = &cli.command
    {
        let path = match journal {
            Some(path) => path.clone(),
            None => recovery::pending_journal(
                directory
                    .as_deref()
                    .context("recovery directory required")?,
            )?
            .context("no pending transaction found in this directory")?,
        };
        println!("{}", recovery::recover(&path, *dry_run)?);
        return Ok(());
    }
    let input = cli.command.input().context("config input required")?;
    let singbox = Executable::resolve(input.sing_box.as_deref());
    require_supported(&singbox)?;
    let configs = ConfigSet::load(input)?;
    if matches!(
        &cli.command,
        Command::Rotate { .. } | Command::Set { dry_run: false, .. }
    ) {
        recovery::check_pending(&configs)?;
    }
    if matches!(cli.command, Command::Check { .. }) {
        ensure!(
            input.inbound_tag.is_empty() && input.client_tag.is_empty(),
            "check validates complete configs; --inbound-tag and --outbound-tag (--client-tag) are not supported"
        );
        apply::check(&configs, &singbox)?;
        println!(
            "Validated server config set and {} client config(s).",
            configs.client_files.len()
        );
        return Ok(());
    }
    let inventory = binding::discover(&configs, input)?;
    match &cli.command {
        Command::Inspect { protocol, .. } => {
            println!("{}", inventory.render(&configs, input, *protocol))
        }
        Command::Plan { selection, .. } | Command::Rotate { selection, .. } => {
            let plan = match selection.protocol {
                Some(protocol) => plan::rotation_by_type(
                    &configs,
                    &inventory,
                    input,
                    protocol,
                    selection.only,
                    &singbox,
                )?,
                None => plan::rotation(
                    &configs,
                    &inventory,
                    input,
                    selection.kind.context("--type or --kind required")?,
                    &singbox,
                )?,
            };
            // Validate edit topology for previews too; plan never creates config files.
            plan.materialize(&configs)?;
            println!("{}", plan.render());
            if matches!(cli.command, Command::Rotate { .. }) {
                let count = apply::apply(&configs, &plan, &singbox)?;
                println!("Updated {count} config file(s).");
            } else {
                println!("Preview only; no config files written. Rotate generates fresh values.");
            }
        }
        Command::Set {
            kind,
            value,
            dry_run,
            ..
        } => {
            let plan = protocol::properties::set(&configs, &inventory, input, *kind, value)?;
            plan.materialize(&configs)?;
            println!("{}", plan.render());
            if *dry_run {
                println!("Preview only; no config files written.");
            } else {
                let count = apply::apply(&configs, &plan, &singbox)?;
                println!("Updated {count} config file(s).");
            }
        }
        Command::Check { .. } | Command::Recover { .. } => unreachable!(),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
