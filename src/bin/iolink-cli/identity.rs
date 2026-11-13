use clap::Args;
use iolink_workbench::profile::{IdentityDraft, Profile};

#[derive(Args)]
#[group(required = true, multiple = true)]
pub struct Arguments {
    /// Unsigned decimal or 0x hexadecimal, at most 16 bits.
    #[arg(long, conflicts_with = "clear_vendor_id")]
    vendor_id: Option<String>,
    #[arg(long)]
    clear_vendor_id: bool,
    /// Unsigned decimal or 0x hexadecimal, at most 24 bits.
    #[arg(long, conflicts_with = "clear_device_id")]
    device_id: Option<String>,
    #[arg(long)]
    clear_device_id: bool,
    /// Exact literal text, including spaces; empty text is an empty-string pin.
    #[arg(long, conflicts_with = "clear_product_id")]
    product_id: Option<String>,
    #[arg(long)]
    clear_product_id: bool,
    #[arg(long, conflicts_with = "clear_product_name")]
    product_name: Option<String>,
    #[arg(long)]
    clear_product_name: bool,
    #[arg(long, conflicts_with = "clear_serial")]
    serial: Option<String>,
    #[arg(long)]
    clear_serial: bool,
    /// Start without pins, then apply any provided values.
    #[arg(long)]
    clear_all: bool,
}

impl Arguments {
    pub fn apply(self, profile: &Profile) -> anyhow::Result<Profile> {
        let mut draft = if self.clear_all {
            IdentityDraft::default()
        } else {
            IdentityDraft::from(&profile.expected_identity)
        };
        macro_rules! update {
            ($field:ident, $clear:ident) => {
                if let Some(value) = self.$field {
                    draft.$field = Some(value);
                }
                if self.$clear {
                    draft.$field = None;
                }
            };
        }
        update!(vendor_id, clear_vendor_id);
        update!(device_id, clear_device_id);
        update!(product_id, clear_product_id);
        update!(product_name, clear_product_name);
        update!(serial, clear_serial);
        profile.with_identity(draft.parse()?)
    }
}
