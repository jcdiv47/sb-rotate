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
    /// Rotate configured credentials/key material for a sing-box type.
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
            Self::Recover { .. } => None,
        }
    }
}

#[derive(Args, Default)]
pub struct Input {
    /// Server JSON file or config directory.
    #[arg(long)]
    pub server: PathBuf,
    /// Local directory of independent client JSON configs (non-recursive; no deployment).
    #[arg(
        long,
        visible_alias = "client-config-dir",
        required_unless_present = "client"
    )]
    pub clients: Option<PathBuf>,
    /// Include/select a client file; repeatable. Shared identities still rotate together.
    #[arg(long = "client")]
    pub client: Vec<PathBuf>,
    #[arg(long)]
    pub inbound_tag: Vec<String>,
    /// Select outbound tags; shared user credentials still rotate together.
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
    /// Rotate all supported material already configured with bound outbounds for this type.
    #[arg(
        long = "type",
        value_enum,
        value_name = "TYPE",
        required_unless_present = "kind",
        conflicts_with = "kind"
    )]
    pub protocol: Option<Protocol>,
    /// Rotate only this material (requires --type). UUID/password/short-ID allow outbound selection.
    #[arg(
        long,
        value_enum,
        value_name = "MATERIAL",
        requires = "protocol",
        conflicts_with = "kind"
    )]
    pub only: Option<RotationMaterial>,
    /// Legacy single-operation interface; cannot be combined with --type/--only.
    #[arg(long, value_enum, required_unless_present = "protocol")]
    pub kind: Option<RotationKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum RotationMaterial {
    Uuid,
    Password,
    RealityShortId,
    RealityKeypair,
    ObfsPassword,
}

impl RotationMaterial {
    pub fn kind(self, protocol: Protocol) -> Option<RotationKind> {
        match (protocol, self) {
            (Protocol::Vless, Self::Uuid) => Some(RotationKind::VlessUuid),
            (Protocol::Vless, Self::RealityShortId) => Some(RotationKind::VlessRealityShortId),
            (Protocol::Vless, Self::RealityKeypair) => Some(RotationKind::VlessRealityKeypair),
            (Protocol::Hysteria2, Self::Password) => Some(RotationKind::Hysteria2Password),
            (Protocol::Hysteria2, Self::ObfsPassword) => Some(RotationKind::Hysteria2ObfsPassword),
            _ => None,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum RotationKind {
    VlessUuid,
    VlessRealityShortId,
    VlessRealityKeypair,
    Hysteria2Password,
    Hysteria2ObfsPassword,
}

impl RotationKind {
    pub fn identity(self) -> Option<IdentityKind> {
        match self {
            Self::VlessUuid => Some(IdentityKind::VlessUuid),
            Self::Hysteria2Password => Some(IdentityKind::Hysteria2Password),
            _ => None,
        }
    }
}

impl From<IdentityKind> for RotationKind {
    fn from(kind: IdentityKind) -> Self {
        match kind {
            IdentityKind::VlessUuid => Self::VlessUuid,
            IdentityKind::Hysteria2Password => Self::Hysteria2Password,
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
