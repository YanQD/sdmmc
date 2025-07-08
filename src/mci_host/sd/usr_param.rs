use super::{constants::SdTimingMode, io_voltage::SdIoVoltage};
use crate::mci_host::mci_host_card_detect::MCIHostCardDetect;
use alloc::rc::Rc;

pub(crate) struct SdUsrParam {
    pub(crate) sd_pwr: Option<SdPwrFn>,
    pub(crate) power_on_delay_ms: u32,
    pub(crate) power_off_delay_ms: u32,
    pub(crate) io_strength: Option<SdIoStrengthFn>,
    pub(crate) io_voltage: Option<SdIoVoltage>,
    pub(crate) card_detect: Option<Rc<MCIHostCardDetect>>,
    pub(crate) max_freq: u32,
    pub(crate) capability: u32,
}

type SdPwrFn = fn(bool);
type SdIoStrengthFn = fn(SdTimingMode);

impl SdUsrParam {
    pub fn new() -> Self {
        SdUsrParam {
            sd_pwr: None,
            power_on_delay_ms: 0,
            power_off_delay_ms: 0,
            io_strength: None,
            io_voltage: None,
            card_detect: None,
            max_freq: 0,
            capability: 0,
        }
    }
}
