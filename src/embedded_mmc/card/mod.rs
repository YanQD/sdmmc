// 卡设备也有不同
mod info;

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;
use info::MmcCardInfo;
use info::SdCardInfo;
use core::fmt::Debug;
use core::sync::atomic::AtomicBool;

use super::commands::DataBuffer;
use super::commands::MmcCommand;

#[derive(Debug, Clone, Copy)]
pub enum CardType {
    Unknown,
    Mmc,
    SdV1,
    SdV2,
}

#[derive(Debug, Clone)]
pub enum Card {
    Mmc(MmcCardInfo),
    Sd(SdCardInfo),
}

pub struct MmcCard {
    card: Option<Card>,
}

impl MmcCard {
    pub fn new() -> Self {
        MmcCard {
            card: None,
        }
    }

    pub fn set_card(&mut self, card: Card) {
        self.card = Some(card);
    }
    
    pub fn get_card(&self) -> Option<&Card> {
        self.card.as_ref()
    }

    pub fn is_present(&self) -> bool {
        self.card.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    Idle        = 0,  // idle state
    Ready       = 1,  // ready state  
    Ident       = 2,  // identification state
    Stby        = 3,  // stand-by state
    Tran        = 4,  // transfer state
    Data        = 5,  // sending-data state
    Rcv         = 6,  // receive-data state
    Prg         = 7,  // programming state
    Dis         = 8,  // disconnect state
    Btst        = 9,  // bus-test state
    Slp         = 10, // sleep state
}