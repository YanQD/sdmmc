mod ext;
pub mod rockship;

use core::fmt::Display;

#[derive(Debug, Clone, PartialEq)]
pub enum SdhciError {
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

impl Display for SdhciError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SdhciError::RegisterAccessFailed => write!(f, "Register access failed"),
            SdhciError::ClockSetupFailed => write!(f, "Clock setup failed"),
            SdhciError::CardDetectFailed => write!(f, "Card detection failed"),
            SdhciError::StrobeConfigurationFailed => write!(f, "Strobe configuration failed"),
            SdhciError::InvalidRegister => write!(f, "Invalid register"),
            SdhciError::InvalidValue => write!(f, "Invalid value"),
            SdhciError::HardwareError => write!(f, "Hardware error"),
            SdhciError::GpioError => write!(f, "GPIO error"),
            SdhciError::UnsupportedOperation => write!(f, "Unsupported operation"),
            SdhciError::DeviceNotFound => write!(f, "Device not found"),
            SdhciError::ProbeFailure => write!(f, "Probe failure"),
            SdhciError::PhyInitFailed => write!(f, "PHY initialization failed"),
            SdhciError::UnsupportedCard => write!(f, "Unsupported card type"),
            SdhciError::CommandError => write!(f, "Command execution error"),
            SdhciError::DataError => write!(f, "Data transfer error"),
            SdhciError::Timeout => write!(f, "Operation timed out"),
        }
    }
}

pub type SdhciResult<T = ()> = Result<T, SdhciError>;