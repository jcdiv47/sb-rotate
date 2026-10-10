use anyhow::{Context, Result, ensure};
use clap::Parser;
use sb_rotate::{
    apply, binding, build,
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
    if let Command::Build {
        manifest,
        targets,
        publish,
        sudo,
        sing_box,
    } = &cli.command
    {
        let singbox = Executable::resolve(sing_box.as_deref());
        require_supported(&singbox)?;
        let mode = publish.then_some(if *sudo {
            build::Publish::Sudo
        } else {
            build::Publish::Direct
        });
        let report = build::build(manifest, targets, mode, &singbox)?;
        println!("{}", report.lines.join("\n"));
        if *sudo {
            build::sudo_install(&report.install)?;
            for (output, target) in &report.install {
                println!("{} -> {}", output.display(), target.display());
            }
            return Ok(());
        }
        let commands: Vec<_> = report
            .install
            .iter()
            .map(|(output, target)| build::sudo_install_command(output, target))
            .collect();
        ensure!(
            commands.is_empty(),
            "no write permission for {} published file(s); run (or use --sudo):\n{}",
            commands.len(),
            commands.join("\n")
        );
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
            let plan = plan::rotation(&configs, &inventory, input, selection.protocol, &singbox)?;
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
        Command::Check { .. } | Command::Recover { .. } | Command::Build { .. } => {
            unreachable!()
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
