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
    /// Discover inbounds, users, and bound outbounds (passwords are masked).
    Inspect {
        #[command(flatten)]
        input: Input,
        #[arg(
            long = "type",
            visible_alias = "protocol",
            value_enum,
            value_name = "TYPE"
        )]
        protocol: Option<Protocol>,
    },
    /// Generate and display a rotation without writing configs.
    Plan {
        #[command(flatten)]
        input: Input,
        #[command(flatten)]
        selection: RotationSelection,
    },
    /// Rotate the selected outbounds' credentials (and fully selected inbounds' shared keys).
    Rotate {
        #[command(flatten)]
        input: Input,
        #[command(flatten)]
        selection: RotationSelection,
    },
    /// Update a service property on every bound client (never the listen port).
    Set {
        #[command(flatten)]
        input: Input,
        #[arg(long, value_enum)]
        kind: PropertyKind,
        #[arg(long)]
        value: String,
        /// Preview without writing config files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Recover an interrupted transaction; does not require sing-box.
    Recover {
        /// Private transaction directory reported by a failed mutation.
        #[arg(
            long,
            required_unless_present = "directory",
            conflicts_with = "directory"
        )]
        journal: Option<PathBuf>,
        /// Find the pending journal through a server/client config directory.
        #[arg(long)]
        directory: Option<PathBuf>,
        /// Show recovery status without changing configs or journal metadata.
        #[arg(long)]
        dry_run: bool,
    },
    /// Merge client fragments into one validated config per target (see --manifest).
    Build {
        /// JSON build manifest; its relative paths resolve from its directory.
        #[arg(long)]
        manifest: PathBuf,
        /// Targets to build (default: all).
        targets: Vec<String>,
        /// Also copy each output to publish_dir/publish_as from the manifest.
        #[arg(long)]
        publish: bool,
        /// Publish every file as root:root 0644 via `sudo install`, after all builds pass.
        #[arg(long, requires = "publish")]
        sudo: bool,
        /// Executable override; otherwise SING_BOX, then sing-box on PATH.
        #[arg(long)]
        sing_box: Option<PathBuf>,
    },
    /// Validate the complete server set and each independent client config.
    Check {
        #[command(flatten)]
        input: Input,
    },
}

impl Command {
    pub fn input(&self) -> Option<&Input> {
        match self {
            Self::Inspect { input, .. }
            | Self::Plan { input, .. }
            | Self::Rotate { input, .. }
            | Self::Set { input, .. }
            | Self::Check { input } => Some(input),
            Self::Recover { .. } | Self::Build { .. } => None,
        }
    }
}

#[derive(Args, Default)]
pub struct Input {
    /// Server JSON file or config directory.
    #[arg(long)]
    pub server: PathBuf,
    /// Local directory of client JSON configs; repeatable (non-recursive; no deployment).
    #[arg(
        long,
        visible_alias = "client-config-dir",
        required_unless_present = "client"
    )]
    pub clients: Vec<PathBuf>,
    /// Include/select a client file; repeatable. A shared user credential still rotates everywhere.
    #[arg(long = "client")]
    pub client: Vec<PathBuf>,
    #[arg(long)]
    pub inbound_tag: Vec<String>,
    /// Select outbound tags; a shared user credential still rotates everywhere.
    #[arg(
        long = "outbound-tag",
        visible_alias = "client-tag",
        value_name = "TAG"
    )]
    pub client_tag: Vec<String>,
    /// Executable override; otherwise SING_BOX, then sing-box on PATH.
    #[arg(long)]
    pub sing_box: Option<PathBuf>,
}

#[derive(Args)]
pub struct RotationSelection {
    /// Limit to one type; by default every supported type with selected bound outbounds.
    #[arg(long = "type", value_enum, value_name = "TYPE")]
    pub protocol: Option<Protocol>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Protocol {
    Vless,
    Hysteria2,
}

impl Protocol {
    pub const ALL: [Self; 2] = [Self::Vless, Self::Hysteria2];

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PropertyKind {
    Server,
    ServerPort,
    ServerPorts,
    TlsServerName,
}
