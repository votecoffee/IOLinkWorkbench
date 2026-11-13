use anyhow::{Result, ensure};

/// IO-Link arrays run from the highest used bit to bit zero. Sub-byte elements
/// share an octet without crossing its boundary; each octet's high gap is padding.
#[derive(Clone, Copy, Debug)]
pub(super) struct Geometry {
    pub width: u16,
    pub count: u8,
}

impl Geometry {
    pub fn new(width: u16, count: u8) -> Result<Self> {
        ensure!(
            width > 0 && count > 0,
            "array width and count must be positive"
        );
        ensure!(
            width < 8 || width.is_multiple_of(8),
            "array elements of 8 bits or more require whole-octet widths"
        );
        let layout = Self { width, count };
        ensure!(
            layout.bits() <= 1856,
            "array exceeds the 232-byte acyclic limit"
        );
        Ok(layout)
    }

    fn raw_offset(self, subindex: u8) -> u32 {
        let remaining = u32::from(self.count - subindex);
        let width = u32::from(self.width);
        if self.width < 8 {
            let per_octet = 8 / width;
            (remaining / per_octet) * 8 + (remaining % per_octet) * width
        } else {
            remaining * width
        }
    }

    pub fn offset(self, subindex: u8) -> u16 {
        self.raw_offset(subindex) as u16
    }

    pub fn bits(self) -> u32 {
        self.raw_offset(1) + u32::from(self.width)
    }
}
