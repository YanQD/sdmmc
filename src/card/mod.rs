// 卡设备也有不同
mod device;

extern crate alloc;
use core::fmt::Debug;
use device::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardType {
    Unknown,
    Mmc,
    SdV1,
    SdV2,
}

#[derive(Debug, Clone)]
pub enum CardExt {
    Mmc(MmcCardExt),
    Sd(SdCardExt),
}

impl CardExt {
    pub fn new(card_type: CardType) -> Self {
        match card_type {
            CardType::Mmc => CardExt::Mmc(MmcCardExt::new()),
            CardType::SdV1 | CardType::SdV2 => CardExt::Sd(SdCardExt::new()),
            _ => CardExt::Mmc(MmcCardExt::new()), // Default to MMC for unknown types
        }
    }

    pub fn is_mmc(&self) -> bool {
        matches!(self, CardExt::Mmc(_))
    }

    pub fn is_sd(&self) -> bool {
        matches!(self, CardExt::Sd(_))
    }

    pub fn as_mmc(&self) -> Option<&MmcCardExt> {
        if let CardExt::Mmc(ext) = self {
            Some(ext)
        } else {
            None
        }
    }

    pub fn as_sd(&self) -> Option<&SdCardExt> {
        if let CardExt::Sd(ext) = self {
            Some(ext)
        } else {
            None
        }
    }

    pub fn as_mut_mmc(&mut self) -> Option<&mut MmcCardExt> {
        if let CardExt::Mmc(ext) = self {
            Some(ext)
        } else {
            None
        }
    }

    pub fn as_mut_sd(&mut self) -> Option<&mut SdCardExt> {
        if let CardExt::Sd(ext) = self {
            Some(ext)
        } else {
            None
        }
    }
}

pub struct MmcCard {
    card_type: CardType,
    is_initialized: bool,
    base_info: BaseCardInfo,
    extension: Option<CardExt>,
}

impl MmcCard {
    pub fn new() -> Self {
        MmcCard {
            card_type: CardType::Unknown,
            is_initialized: false,
            base_info: BaseCardInfo::new(),
            extension: None,
        }
    }

    pub fn set_cardext(&mut self, cardext: CardExt) {
        self.extension = Some(cardext);
    }

    pub fn cardext(&self) -> Option<&CardExt> {
        self.extension.as_ref()
    }

    pub fn cardext_mut(&mut self) -> Option<&mut CardExt> {
        self.extension.as_mut()
    }

    pub fn card_type(&self) -> CardType {
        self.card_type
    }

    pub fn set_card_type(&mut self, card_type: CardType) {
        self.card_type = card_type;
    }

    pub fn is_initialized(&self) -> bool {
        self.is_initialized
    }

    pub fn set_initialized(&mut self, initialized: bool) {
        self.is_initialized = initialized;
    }

    pub fn base_info(&self) -> &BaseCardInfo {
        &self.base_info
    }

    pub fn base_info_mut(&mut self) -> &mut BaseCardInfo {
        &mut self.base_info
    }

    pub fn set_base_info(&mut self, base_info: BaseCardInfo) {
        self.base_info = base_info;
    }
}