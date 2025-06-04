mod block;
pub mod clock;
pub mod cmd;

use crate::{aux::generic_fls, constants::*, delay_us, impl_register_ops};
use core::fmt::Display;
use log::{debug, info};

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

// SD Host Controller structure
#[derive(Debug)]
pub struct SdhciHost {
    pub base_addr: usize,
    pub caps: u32,
    pub clock_base: u32,
    pub voltages: u32,
    pub quirks: u32,
    pub host_caps: u32,
    pub version: u16,

    pub timing: u32,
    pub bus_width: u8,
    pub clock: u32,
}

impl Display for SdhciHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "EMMC Controller {{ base_addr: {:#x}, caps: {:#x}, clock_base: {} }}",
            self.base_addr, self.caps, self.clock_base
        )
    }
}

impl_register_ops!(SdhciHost, base_addr);

impl SdhciHost {
    pub fn new(base_addr: usize) -> Self {
        SdhciHost {
            base_addr,
            caps: 0,
            clock_base: 0,
            voltages: 0,
            quirks: 0,
            host_caps: 0,
            version: 0,

            timing: MMC_TIMING_LEGACY,
            bus_width: 1,  // Default to 1-bit bus width
            clock: 400000, // Default to 400 kHz
        }
    }

    // Initialize the host controller
    pub fn init_host(&mut self) -> SdhciResult {
        info!("Init EMMC Controller");

        // Reset the controller
        self.reset(EMMC_RESET_ALL)?;

        let version = self.read_reg16(EMMC_HOST_CNTRL_VER);
        // version = 4.2
        self.version = version;
        info!("EMMC Version: 0x{:x}", version);

        let caps1 = self.read_reg32(EMMC_CAPABILITIES1);
        info!("EMMC Capabilities 1: 0b{:b}", caps1);

        let mut clk_mul: u32 = 0;

        if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 {
            let caps2 = self.read_reg32(EMMC_CAPABILITIES2);
            info!("EMMC Capabilities 2: 0b{:b}", caps2);
            clk_mul = (caps2 & EMMC_CLOCK_MUL_MASK) >> EMMC_CLOCK_MUL_SHIFT;
        }

        if self.clock_base == 0 {
            if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 {
                self.clock_base = (caps1 & EMMC_CLOCK_V3_BASE_MASK) >> EMMC_CLOCK_BASE_SHIFT
            } else {
                self.clock_base = (caps1 & EMMC_CLOCK_BASE_MASK) >> EMMC_CLOCK_BASE_SHIFT
            }

            self.clock_base *= 1000000; // convert to Hz
            if clk_mul != 0 {
                self.clock_base *= clk_mul;
            }
        }

        if self.clock_base == 0 {
            info!("Hardware doesn't specify base clock frequency");
            return Err(SdhciError::UnsupportedCard);
        }

        self.host_caps = MMC_MODE_HS | MMC_MODE_HS_52MHZ | MMC_MODE_4BIT;

        if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 && (caps1 & EMMC_CAN_DO_8BIT) == 0 {
            self.host_caps &= !MMC_MODE_8BIT;
        }

        // 暂时写死
        self.host_caps |= 0x48;

        // debug!("self.host_caps {:#x}", self.host_caps);

        let mut voltages = 0;

        if (caps1 & EMMC_CAN_VDD_330) != 0 {
            voltages |= MMC_VDD_32_33 | MMC_VDD_33_34;
        } else if (caps1 & EMMC_CAN_VDD_300) != 0 {
            voltages |= MMC_VDD_29_30 | MMC_VDD_30_31;
        } else if (caps1 & EMMC_CAN_VDD_180) != 0 {
            voltages |= MMC_VDD_165_195;
        } else {
            info!("Unsupported voltage range");
            return Err(SdhciError::UnsupportedCard);
        }

        self.voltages = voltages;

        info!(
            "voltage range: {:#x}, {:#x}",
            voltages,
            generic_fls(voltages) - 1
        );

        // Perform full power cycle
        self.sdhci_set_power(generic_fls(voltages) - 1).unwrap();

        // Enable interrupts
        self.write_reg32(
            EMMC_NORMAL_INT_STAT_EN,
            EMMC_INT_CMD_MASK | EMMC_INT_DATA_MASK,
        );
        self.write_reg32(EMMC_SIGNAL_ENABLE, 0x0);

        // Set initial bus width to 1-bit
        self.mmc_set_bus_width(1);

        // Set initial clock and wait for it to stabilize
        self.mmc_set_clock(400000);

        self.mmc_set_timing(MMC_TIMING_LEGACY);

        info!("EMMC initialization completed successfully");
        Ok(())
    }

    // Reset the controller
    #[inline]
    pub fn reset(&self, reset_flag: u8) -> SdhciResult {
        // Request reset
        self.write_reg8(EMMC_SOFTWARE_RESET, reset_flag);

        // Wait for reset to complete with timeout
        let mut timeout = 20; // Increased timeout
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & reset_flag) != 0 {
            if timeout == 0 {
                return Err(SdhciError::Timeout);
            }
            timeout -= 1;
            delay_us(1000);
        }

        Ok(())
    }

    pub fn mmc_set_bus_width(&mut self, width: u8) {
        /* Set bus width */
        self.bus_width = width;
        debug!("Bus width set to {}", width);
        self.mmc_set_ios();
    }

    pub fn mmc_set_timing(&mut self, timing: u32) {
        /* Set timing */
        self.timing = timing;
        self.mmc_set_ios();
    }

    pub fn mmc_set_clock(&mut self, clk: u32) {
        /* Set clock */
        self.clock = clk;
        self.mmc_set_ios();
    }
}
