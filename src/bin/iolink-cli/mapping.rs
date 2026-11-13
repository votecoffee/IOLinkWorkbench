use clap::{Args, ValueEnum};
use iolink_workbench::{
    decode::{ByteOrder, Field, Kind},
    profile::Profile,
};

#[derive(Clone, Copy, ValueEnum)]
enum KindChoice {
    Unsigned,
    Signed,
    Bool,
    Float32,
}

#[derive(Clone, Copy, ValueEnum)]
enum OrderChoice {
    BigEndian,
    LittleEndian,
}

#[derive(Args)]
#[group(required = true, multiple = true)]
pub struct Arguments {
    #[arg(long)]
    bit_offset: Option<u16>,
    #[arg(long)]
    bits: Option<u8>,
    #[arg(long, value_enum)]
    kind: Option<KindChoice>,
    #[arg(long, value_enum)]
    byte_order: Option<OrderChoice>,
    #[arg(long, allow_hyphen_values = true)]
    scale: Option<f64>,
    #[arg(long, allow_hyphen_values = true)]
    offset: Option<f64>,
    #[arg(long)]
    unit: Option<String>,
    #[arg(long, conflicts_with = "clear_decimals")]
    decimals: Option<u8>,
    #[arg(long)]
    clear_decimals: bool,
}

impl Arguments {
    pub fn apply(self, profile: &Profile, name: &str) -> anyhow::Result<Profile> {
        let existing = profile
            .effective_fields()
            .into_iter()
            .find(|(field, _)| field.name == name)
            .map(|(field, _)| field.clone());
        if existing.is_none() {
            anyhow::ensure!(
                self.bit_offset.is_some() && self.bits.is_some() && self.kind.is_some(),
                "a new field requires --bit-offset, --bits and --kind"
            );
        }
        let mut field = existing.unwrap_or_else(|| Field {
            name: name.to_owned(),
            ..Default::default()
        });
        macro_rules! update {
            ($value:ident) => {
                if let Some(value) = self.$value {
                    field.$value = value;
                }
            };
        }
        update!(bit_offset);
        update!(bits);
        update!(scale);
        update!(offset);
        update!(unit);
        if let Some(kind) = self.kind {
            field.kind = match kind {
                KindChoice::Unsigned => Kind::Unsigned,
                KindChoice::Signed => Kind::Signed,
                KindChoice::Bool => Kind::Bool,
                KindChoice::Float32 => Kind::Float32,
            };
        }
        if let Some(order) = self.byte_order {
            field.byte_order = match order {
                OrderChoice::BigEndian => ByteOrder::BigEndian,
                OrderChoice::LittleEndian => ByteOrder::LittleEndian,
            };
        }
        if let Some(decimals) = self.decimals {
            field.decimals = Some(decimals);
        }
        if self.clear_decimals {
            field.decimals = None;
        }
        profile.with_field(field)
    }
}
