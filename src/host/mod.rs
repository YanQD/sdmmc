pub mod pythium;
pub mod rockchip;

extern crate alloc;
use alloc::string::String;

use super::common::commands::DataBuffer;
use super::common::commands::MmcCommand;
use crate::common::HostCapabilities;
use crate::mci_core::MmcHostInfo;
use alloc::vec::Vec;
use core::fmt::Debug;
use core::fmt::Display;

#[derive(Debug)]
pub struct UDevice {
    pub name: String,
    pub compatible: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MmcHostError {
    RegisterAccessFailed,
    ClockSetupFailed,
    CardDetectFailed,
    StrobeConfigurationFailed,
    InvalidRegister,
    InvalidValue,
    HardwareError,
    IoError,
    UnsupportedOperation,
    DeviceNotFound,
    ProbeFailure,
    PhyInitFailed,
    UnsupportedCard,
    CommandError,
    DataError,
    Timeout,
    MemoryError,
}

impl Display for MmcHostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MmcHostError::RegisterAccessFailed => write!(f, "Register access failed"),
            MmcHostError::ClockSetupFailed => write!(f, "Clock setup failed"),
            MmcHostError::CardDetectFailed => write!(f, "Card detection failed"),
            MmcHostError::StrobeConfigurationFailed => write!(f, "Strobe configuration failed"),
            MmcHostError::InvalidRegister => write!(f, "Invalid register"),
            MmcHostError::InvalidValue => write!(f, "Invalid value"),
            MmcHostError::HardwareError => write!(f, "Hardware error"),
            MmcHostError::IoError => write!(f, "IO error"),
            MmcHostError::UnsupportedOperation => write!(f, "Unsupported operation"),
            MmcHostError::DeviceNotFound => write!(f, "Device not found"),
            MmcHostError::ProbeFailure => write!(f, "Probe failure"),
            MmcHostError::PhyInitFailed => write!(f, "PHY initialization failed"),
            MmcHostError::UnsupportedCard => write!(f, "Unsupported card type"),
            MmcHostError::CommandError => write!(f, "Command execution error"),
            MmcHostError::DataError => write!(f, "Data transfer error"),
            MmcHostError::Timeout => write!(f, "Operation timed out"),
            MmcHostError::MemoryError => write!(f, "Memory allocation error"),
        }
    }
}

pub type MmcHostResult<T = ()> = Result<T, MmcHostError>;

pub trait MmcHostOps: Debug + Send + Sync {
    type Capabilities: HostCapabilities;

    fn init_host(&mut self) -> MmcHostResult;

    fn read_reg32(&self, offset: u32) -> u32;
    fn write_reg32(&self, offset: u32, value: u32);
    fn read_reg16(&self, offset: u32) -> u16;
    fn write_reg16(&self, offset: u32, value: u16);
    fn read_reg8(&self, offset: u32) -> u8;
    fn write_reg8(&self, offset: u32, value: u8);

    fn mmc_send_command(&self, cmd: &MmcCommand, data_buffer: Option<DataBuffer>) -> MmcHostResult;
    fn mmc_card_busy(&self) -> bool;
    fn mmc_set_ios(&mut self, mmc_current: &MmcHostInfo);

    fn get_capabilities(&self) -> Self::Capabilities;
}
