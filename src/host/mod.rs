pub mod pythium;
pub mod rockship;

extern crate alloc;
use alloc::string::String;

use alloc::vec::Vec;
use core::fmt::Debug;
use core::fmt::Display;

use super::commands::DataBuffer;
use super::commands::MmcCommand;

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
    GpioError,
    UnsupportedOperation,
    DeviceNotFound,
    ProbeFailure,
    PhyInitFailed,
    UnsupportedCard,
    CommandError,
    DataError,
    Timeout,
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
            MmcHostError::GpioError => write!(f, "GPIO error"),
            MmcHostError::UnsupportedOperation => write!(f, "Unsupported operation"),
            MmcHostError::DeviceNotFound => write!(f, "Device not found"),
            MmcHostError::ProbeFailure => write!(f, "Probe failure"),
            MmcHostError::PhyInitFailed => write!(f, "PHY initialization failed"),
            MmcHostError::UnsupportedCard => write!(f, "Unsupported card type"),
            MmcHostError::CommandError => write!(f, "Command execution error"),
            MmcHostError::DataError => write!(f, "Data transfer error"),
            MmcHostError::Timeout => write!(f, "Operation timed out"),
        }
    }
}

pub type MmcHostResult<T = ()> = Result<T, MmcHostError>;

pub trait MmcHostOps: Debug + Send + Sync {
    fn init_host(&mut self) -> MmcHostResult;

    fn read_reg32(&self, offset: u32) -> u32;
    fn write_reg32(&self, offset: u32, value: u32);
    fn read_reg16(&self, offset: u32) -> u16;
    fn write_reg16(&self, offset: u32, value: u16);
    fn read_reg8(&self, offset: u32) -> u8;
    fn write_reg8(&self, offset: u32, value: u8);

    fn mmc_send_command(&self, cmd: &MmcCommand, data_buffer: Option<DataBuffer>) -> MmcHostResult;
    fn mmc_card_busy(&self) -> bool;
    fn mmc_set_ios(&mut self);

    fn mmc_card_hs400es(&self) -> bool;
    fn mmc_card_hs200(&self) -> bool;
    fn mmc_set_bus_speed(&mut self, avail_type: u32);
    fn mmc_select_card_type(&self, ext_csd: &[u8]) -> u16;
    fn mmc_hs200_tuning(&mut self) -> MmcHostResult;
    fn mmc_set_bus_width(&mut self, width: u8);
    fn mmc_set_timing(&mut self, timing: u32);
    fn mmc_set_clock(&mut self, clk: u32);

    fn bus_width(&self) -> u8;
    fn host_caps(&self) -> u32;
    fn voltages(&self) -> u32;
}
