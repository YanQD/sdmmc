use log::debug;

use crate::{constants::*, core::MmcHost};

impl MmcHost {
    /// Set the SDHCI EMMC controller's I/O settings
    pub fn sdhci_set_ios(&mut self) {
        let (card_clock, bus_width, timing) = (self.clock, self.bus_width, self.timing);

        debug!(
            "card_clock: {}, bus_width: {}, timing: {}",
            card_clock, bus_width, timing
        );

        self.host_ops_mut().mmc_set_clock(card_clock).unwrap();

        /* Set bus width */
        let mut ctrl = self.host_ops_mut().read_reg8(EMMC_HOST_CTRL1);
        if bus_width == 8 {
            ctrl &= !EMMC_CTRL_4BITBUS;
            if self.sdhci_get_version() >= EMMC_SPEC_300
                || (self.quirks & SDHCI_QUIRK_USE_WIDE8) != 0
            {
                ctrl |= EMMC_CTRL_8BITBUS;
            }
        } else {
            if self.sdhci_get_version() >= EMMC_SPEC_300
                || (self.quirks & SDHCI_QUIRK_USE_WIDE8) != 0
            {
                ctrl &= !EMMC_CTRL_8BITBUS;
            }
            if bus_width == 4 {
                ctrl |= EMMC_CTRL_4BITBUS;
            } else {
                ctrl &= !EMMC_CTRL_4BITBUS;
            }
        }

        if !(timing == MMC_TIMING_LEGACY) && (self.quirks & SDHCI_QUIRK_NO_HISPD_BIT) == 0 {
            ctrl |= EMMC_CTRL_HISPD;
        } else {
            ctrl &= !EMMC_CTRL_HISPD;
        }

        debug!("EMMC Host Control 1: {:#x}", ctrl);

        self.host_ops_mut().write_reg8(EMMC_HOST_CTRL1, ctrl);

        if timing != MMC_TIMING_LEGACY && timing != MMC_TIMING_MMC_HS && timing != MMC_TIMING_SD_HS
        {
            self.host_ops_mut().sdhci_set_power(MMC_VDD_165_195_SHIFT).unwrap();
        }

        self.sdhci_set_uhs_signaling();
    }


}