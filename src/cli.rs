use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Discover service and identity bindings (passwords are masked).
    Inspect {
        #[command(flatten)]
        input: Input,
        #[arg(long, value_enum)]
        protocol: Option<Protocol>,
    },
    /// Generate and display an identity rotation without writing configs.
    Plan {
        #[command(flatten)]
        input: Input,
        #[arg(long, value_enum)]
        kind: IdentityKind,
    },
    /// Rotate a shared identity and all its discovered occurrences.
    Rotate {
        #[command(flatten)]
        input: Input,
        #[arg(long, value_enum)]
        kind: IdentityKind,
    },
    /// Validate the complete server set and each independent client config.
    Check {
        #[command(flatten)]
        input: Input,
    },
}

impl Command {
    pub fn input(&self) -> &Input {
        match self {
            Self::Inspect { input, .. }
            | Self::Plan { input, .. }
            | Self::Rotate { input, .. }
            | Self::Check { input } => input,
        }
    }
}

#[derive(Args, Default)]
pub struct Input {
    /// Server JSON file or config directory.
    #[arg(long)]
    pub server: PathBuf,
    /// Directory of independent client JSON files (non-recursive).
    #[arg(long, required_unless_present = "client")]
    pub clients: Option<PathBuf>,
    /// Include/select a client file; repeatable. Shared identities still rotate together.
    #[arg(long = "client")]
    pub client: Vec<PathBuf>,
    #[arg(long)]
    pub inbound_tag: Vec<String>,
    #[arg(long)]
    pub client_tag: Vec<String>,
    /// Executable override; otherwise SING_BOX, then sing-box on PATH.
    #[arg(long)]
    pub sing_box: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Protocol {
    Vless,
    Hysteria2,
}

impl Protocol {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "vless" => Some(Self::Vless),
            "hysteria2" => Some(Self::Hysteria2),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Vless => "vless",
            Self::Hysteria2 => "hysteria2",
        }
    }

    pub fn credential(self) -> &'static str {
        match self {
            Self::Vless => "uuid",
            Self::Hysteria2 => "password",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum IdentityKind {
    VlessUuid,
    Hysteria2Password,
}

impl IdentityKind {
    pub fn protocol(self) -> Protocol {
        match self {
            Self::VlessUuid => Protocol::Vless,
            Self::Hysteria2Password => Protocol::Hysteria2,
        }
    }
}
