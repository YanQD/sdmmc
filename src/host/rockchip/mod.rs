mod caps;
pub mod clock;
pub mod cmd;
pub mod regs;

use core::fmt::Display;
use log::info;

use crate::{
    aux::generic_fls,
    common::commands::{DataBuffer, MmcCommand},
    constants::*,
    core::MmcHostInfo,
    host::{MmcHostError, MmcHostOps, MmcHostResult, rockchip::caps::SdhciCapabilities},
};

// SD Host Controller structure
#[derive(Debug, Clone)]
pub struct SdhciHost {
    pub base_addr: usize,
    pub clock_base: u32,
    pub voltages: u32,
    pub quirks: u32,
    pub host_caps: u32,
    pub version: u16,
}

impl Display for SdhciHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SdhciHost(base_addr: {:#x}, clock_base: {}, voltages: {:#x}, quirks: {:#x}, host_caps: {:#x}, version: 0x{:x})",
            self.base_addr,
            self.clock_base,
            self.voltages,
            self.quirks,
            self.host_caps,
            self.version,
        )
    }
}

impl SdhciHost {
    pub fn new(base_addr: usize) -> Self {
        SdhciHost {
            base_addr,
            clock_base: 0,
            voltages: 0,
            quirks: 0,
            host_caps: 0,
            version: 0,
        }
    }

    // Initialize the host controller
    fn init_host(&mut self) -> MmcHostResult {
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
            return Err(MmcHostError::UnsupportedCard);
        }

        self.host_caps = MMC_MODE_HS | MMC_MODE_HS_52MHZ | MMC_MODE_4BIT;

        if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 && (caps1 & EMMC_CAN_DO_8BIT) == 0 {
            self.host_caps &= !MMC_MODE_8BIT;
        }

        // 暂时写死
        self.host_caps |= 0x48;

        let mut voltages = 0;

        if (caps1 & EMMC_CAN_VDD_330) != 0 {
            voltages |= MMC_VDD_32_33 | MMC_VDD_33_34;
        } else if (caps1 & EMMC_CAN_VDD_300) != 0 {
            voltages |= MMC_VDD_29_30 | MMC_VDD_30_31;
        } else if (caps1 & EMMC_CAN_VDD_180) != 0 {
            voltages |= MMC_VDD_165_195;
        } else {
            info!("Unsupported voltage range");
            return Err(MmcHostError::UnsupportedCard);
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

        info!("EMMC initialization completed successfully");
        Ok(())
    }
}

impl MmcHostOps for SdhciHost {
    type Capabilities = SdhciCapabilities;

    fn init_host(&mut self) -> MmcHostResult {
        self.init_host()
    }

    fn read_reg8(&self, offset: u32) -> u8 {
        self.read_reg8(offset)
    }

    fn write_reg8(&self, offset: u32, value: u8) {
        self.write_reg8(offset, value)
    }

    fn read_reg16(&self, offset: u32) -> u16 {
        self.read_reg16(offset)
    }

    fn write_reg16(&self, offset: u32, value: u16) {
        self.write_reg16(offset, value)
    }

    fn read_reg32(&self, offset: u32) -> u32 {
        self.read_reg32(offset)
    }

    fn write_reg32(&self, offset: u32, value: u32) {
        self.write_reg32(offset, value)
    }

    fn mmc_send_command(&self, cmd: &MmcCommand, data_buffer: Option<DataBuffer>) -> MmcHostResult {
        self.send_command(cmd, data_buffer)
    }

    fn mmc_card_busy(&self) -> bool {
        self.mmc_card_busy()
    }

    fn mmc_set_ios(&mut self, mmc_current: &MmcHostInfo) {
        self.sdhci_set_ios(mmc_current)
    }

    fn get_capabilities(&self) -> Self::Capabilities {
        SdhciCapabilities {
            voltages: self.voltages,
            host_caps: self.host_caps,
            clock_base: self.clock_base,
            version: self.version,
            quirks: self.quirks,
        }
    }
}
