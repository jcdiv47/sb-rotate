use anyhow::{Result, ensure};
use clap::Parser;
use sb_rotate::{
    apply, binding,
    cli::{Cli, Command},
    config::ConfigSet,
    plan,
    singbox::{Executable, require_supported},
};

fn run(cli: Cli) -> Result<()> {
    let input = cli.command.input();
    let singbox = Executable::resolve(input.sing_box.as_deref());
    require_supported(&singbox)?;
    let configs = ConfigSet::load(input)?;
    if matches!(cli.command, Command::Check { .. }) {
        ensure!(
            input.inbound_tag.is_empty() && input.client_tag.is_empty(),
            "check validates complete configs; --inbound-tag and --client-tag are not supported"
        );
        apply::check(&configs, &singbox)?;
        println!(
            "Validated server config set and {} client config(s).",
            configs.client_files.len()
        );
        return Ok(());
    }
    let inventory = binding::discover(&configs, input)?;
    match cli.command {
        Command::Inspect { protocol, .. } => {
            println!("{}", inventory.render(&configs, input, protocol))
        }
        Command::Plan { kind, .. } | Command::Rotate { kind, .. } => {
            let plan = plan::identities(&configs, &inventory, input, kind, &singbox)?;
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
        Command::Check { .. } => unreachable!(),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
