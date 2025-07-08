use core::time::Duration;

use crate::{
    aux::dll_lock_wo_tmout,
    common::clock::emmc_set_clk,
    constants::*,
    host::{MmcHostError, MmcHostResult, rockchip::SdhciHost},
    mci_core::MmcHostInfo,
    mci_sleep,
};
use log::{debug, info};

#[derive(Debug, Clone, Copy)]
pub struct EMmcChipConfig {
    pub flags: u32,
    pub hs200_tx_tap: u8,
    pub hs400_tx_tap: u8,
    pub hs400_cmd_tap: u8,
    pub hs400_strbin_tap: u8,
    pub _ddr50_strbin_delay_num: u8,
}

impl EMmcChipConfig {
    pub fn rk3568_config() -> Self {
        Self {
            flags: RK_RXCLK_NO_INVERTER,
            hs200_tx_tap: 16,
            hs400_tx_tap: 8,
            hs400_cmd_tap: 8,
            hs400_strbin_tap: 3,
            _ddr50_strbin_delay_num: 16,
        }
    }
}

impl SdhciHost {
    /// Set the SDHCI EMMC controller's I/O settings
    pub fn sdhci_set_ios(&mut self, mmc_current: &MmcHostInfo) {
        let (card_clock, bus_width, timing) =
            (mmc_current.clock, mmc_current.bus_width, mmc_current.timing);

        debug!(
            "card_clock: {}, bus_width: {}, timing: {}",
            card_clock, bus_width, timing
        );

        self.dwcmshc_sdhci_emmc_set_clock(card_clock, timing)
            .unwrap();

        /* Set bus width */
        let mut ctrl = self.read_reg8(EMMC_HOST_CTRL1);
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

        self.write_reg8(EMMC_HOST_CTRL1, ctrl);

        if timing != MMC_TIMING_LEGACY && timing != MMC_TIMING_MMC_HS && timing != MMC_TIMING_SD_HS
        {
            self.sdhci_set_power(MMC_VDD_165_195_SHIFT).unwrap();
        }

        self.sdhci_set_uhs_signaling(timing);
    }

    pub fn mmc_card_busy(&self) -> bool {
        let present_state = self.read_reg32(EMMC_PRESENT_STATE);
        // Check if DATA[0] line is 0 (low level indicates busy)
        !(present_state & EMMC_DATA_0_LVL != 0)
    }

    // Rockchip EMMC设置时钟函数
    fn rockchip_emmc_set_clock(&mut self, freq: u32) -> MmcHostResult {
        // wait for command and data inhibit to be cleared
        let mut timeout = 20;
        while (self.read_reg32(EMMC_PRESENT_STATE) & (EMMC_CMD_INHIBIT | EMMC_DATA_INHIBIT)) != 0 {
            if timeout == 0 {
                debug!("Timeout waiting for cmd & data inhibit");
                return Err(MmcHostError::Timeout);
            }
            timeout -= 1;
            mci_sleep(Duration::from_micros(1000));
        }

        // first disable the clock
        self.write_reg16(EMMC_CLOCK_CONTROL, 0x0000);

        if freq == 0 {
            return Ok(());
        }

        let input_clk = emmc_set_clk(freq as u64).unwrap() as u32;
        info!("input_clk: {}", input_clk);

        let mut div = 0;
        let mut clk = 0u16;
        let sdhci_version = self.read_reg16(EMMC_HOST_CNTRL_VER);

        if (sdhci_version & 0xFF) >= EMMC_SPEC_300 {
            let caps2 = self.read_reg32(EMMC_CAPABILITIES2);
            let clk_mul = (caps2 & EMMC_CLOCK_MUL_MASK) >> EMMC_CLOCK_MUL_SHIFT;

            info!("EMMC Clock Mul: {}", clk_mul);

            // Check if the Host Controller supports Programmable Clock Mode.
            if clk_mul != 0 {
                for i in 1..=1024 {
                    if (input_clk / i) <= freq {
                        div = i;
                        break;
                    }
                }
                // Set Programmable Clock Mode in the Clock Control register.
                clk = EMMC_PROG_CLOCK_MODE;
                div -= 1;
            } else {
                // Version 3.00 divisors must be a multiple of 2.
                if input_clk <= freq {
                    div = 1;
                } else {
                    for i in (2..=2046).step_by(2) {
                        if (input_clk / i) <= freq {
                            div = i;
                            break;
                        }
                    }
                }
                div >>= 1;
            }
        } else {
            // Version 2.00 divisors must be a power of 2.
            let mut i = 1;
            while i < 256 && (input_clk / i) > freq {
                i *= 2;
            }
            div = i >> 1;
        }

        info!("EMMC Clock Divisor: 0x{:x}", div);

        clk |= ((div as u16) & 0xFF) << EMMC_DIVIDER_SHIFT;
        clk |= (((div as u16) & 0x300) >> 8) << EMMC_DIVIDER_HI_SHIFT;

        self.write_reg16(EMMC_CLOCK_CONTROL, clk);
        self.enable_card_clock(clk)?;

        Ok(())
    }

    fn enable_card_clock(&mut self, mut clk: u16) -> MmcHostResult {
        clk |= EMMC_CLOCK_INT_EN;
        clk &= !EMMC_CLOCK_INT_STABLE;
        self.write_reg16(EMMC_CLOCK_CONTROL, clk);

        let mut timeout = 20;
        while (self.read_reg16(EMMC_CLOCK_CONTROL) & EMMC_CLOCK_INT_STABLE) == 0 {
            timeout -= 1;
            mci_sleep(Duration::from_micros(1000));
            if timeout == 0 {
                info!("Internal clock never stabilised.");
                return Err(MmcHostError::Timeout);
            }
        }

        self.write_reg16(EMMC_CLOCK_CONTROL, clk | EMMC_CLOCK_CARD_EN);

        debug!(
            "EMMC Clock Control: {:#x}",
            self.read_reg16(EMMC_CLOCK_CONTROL)
        );

        Ok(())
    }

    pub(crate) fn sdhci_set_power(&mut self, power: u32) -> MmcHostResult {
        let mut pwr: u8 = 0;

        if power != 0xFFFF {
            match 1 << power {
                MMC_VDD_165_195 => {
                    pwr = EMMC_POWER_180;
                }
                MMC_VDD_29_30 | MMC_VDD_30_31 => {
                    pwr = EMMC_POWER_300;
                }
                MMC_VDD_32_33 | MMC_VDD_33_34 => {
                    pwr = EMMC_POWER_330;
                }
                _ => {}
            }
        }

        if pwr == 0 {
            self.write_reg8(EMMC_POWER_CTRL, 0);
            return Ok(());
        }

        pwr |= EMMC_POWER_ON;
        self.write_reg8(EMMC_POWER_CTRL, pwr);

        info!("EMMC Power Control: {:#x}", self.read_reg8(EMMC_POWER_CTRL));

        // Small delay for power to stabilize
        mci_sleep(Duration::from_micros(10000));

        Ok(())
    }

    /// Sets the clock for the DWCMSHC SDHCI EMMC controller.
    fn dwcmshc_sdhci_emmc_set_clock(&mut self, freq: u32, timing: u32) -> MmcHostResult {
        let mut timeout = 500;
        let data = EMmcChipConfig::rk3568_config();

        self.rockchip_emmc_set_clock(freq)?;
        // Disable output clock while config DLL
        self.write_reg16(EMMC_CLOCK_CONTROL, 0);

        info!(
            "EMMC Clock Control: {:#x}",
            self.read_reg16(EMMC_CLOCK_CONTROL)
        );

        // DLL配置基于频率
        if freq >= 100_000_000 {
            // Enable DLL
            self.write_reg32(DWCMSHC_EMMC_DLL_CTRL, DWCMSHC_EMMC_DLL_CTRL_RESET);
            mci_sleep(Duration::from_micros(1000));
            self.write_reg32(DWCMSHC_EMMC_DLL_CTRL, 0);
            let mut extra = 0x1 << 16 | 0x2 << 17 | 0x3 << 19;
            self.write_reg32(DWCMSHC_EMMC_ATCTRL, extra);

            /* Init DLL Setting */
            extra = DWCMSHC_EMMC_DLL_START_DEFAULT << DWCMSHC_EMMC_DLL_START_POINT
                | DWCMSHC_EMMC_DLL_INC_VALUE << DWCMSHC_EMMC_DLL_INC
                | DWCMSHC_EMMC_DLL_START;
            self.write_reg32(DWCMSHC_EMMC_DLL_CTRL, extra);

            loop {
                if timeout <= 0 {
                    info!("Timeout waiting for DLL to be ready");
                    return Err(MmcHostError::Timeout);
                }

                if dll_lock_wo_tmout(self.read_reg32(DWCMSHC_EMMC_DLL_STATUS0)) {
                    break;
                }

                mci_sleep(Duration::from_micros(1000));
                timeout -= 1;
            }

            let dll_lock_value = ((self.read_reg32(DWCMSHC_EMMC_DLL_STATUS0) & 0xFF) * 2) & 0xFF;

            extra = DWCMSHC_EMMC_DLL_DLYENA | DLL_RXCLK_ORI_GATE;
            if (data.flags & RK_RXCLK_NO_INVERTER) != 0 {
                extra |= DLL_RXCLK_NO_INVERTER;
            }

            if (data.flags & RK_TAP_VALUE_SEL) != 0 {
                extra |= DLL_TAP_VALUE_SEL | (dll_lock_value << DLL_TAP_VALUE_OFFSET);
            }
            self.write_reg32(DWCMSHC_EMMC_DLL_RXCLK, extra);

            let mut txclk_tapnum = data.hs200_tx_tap;
            if (data.flags & RK_DLL_CMD_OUT) != 0
                && (timing == MMC_TIMING_MMC_HS400 || timing == MMC_TIMING_MMC_HS400ES)
            {
                txclk_tapnum = data.hs400_tx_tap;

                extra = DLL_CMDOUT_SRC_CLK_NEG
                    | DLL_CMDOUT_BOTH_CLK_EDGE
                    | DWCMSHC_EMMC_DLL_DLYENA
                    | (data.hs400_cmd_tap as u32)
                    | DLL_CMDOUT_TAPNUM_FROM_SW;
                if (data.flags & RK_TAP_VALUE_SEL) != 0 {
                    extra |= DLL_TAP_VALUE_SEL | (dll_lock_value << DLL_TAP_VALUE_OFFSET);
                }

                self.write_reg32(DECMSHC_EMMC_DLL_CMDOUT, extra);
            }

            extra = DWCMSHC_EMMC_DLL_DLYENA
                | DLL_TXCLK_TAPNUM_FROM_SW
                | DLL_TXCLK_NO_INVERTER
                | txclk_tapnum as u32;
            if (data.flags & RK_TAP_VALUE_SEL) != 0 {
                extra |= DLL_TAP_VALUE_SEL | (dll_lock_value << DLL_TAP_VALUE_OFFSET);
            }
            self.write_reg32(DWCMSHC_EMMC_DLL_TXCLK, extra);

            extra =
                DWCMSHC_EMMC_DLL_DLYENA | data.hs400_strbin_tap as u32 | DLL_STRBIN_TAPNUM_FROM_SW;
            if (data.flags & RK_TAP_VALUE_SEL) != 0 {
                extra |= DLL_TAP_VALUE_SEL | (dll_lock_value << DLL_TAP_VALUE_OFFSET);
            }
            self.write_reg32(DWCMSHC_EMMC_DLL_STRBIN, extra);
        } else {
            // Disable dll
            self.write_reg32(DWCMSHC_EMMC_DLL_CTRL, 0);

            // Disable cmd conflict check
            let mut extra = self.read_reg32(DWCMSHC_HOST_CTRL3);

            extra &= !0x1;
            self.write_reg32(DWCMSHC_HOST_CTRL3, extra);

            // reset the clock phase when the frequency is lower than 100MHz
            self.write_reg32(
                DWCMSHC_EMMC_DLL_CTRL,
                DWCMSHC_EMMC_DLL_BYPASS | DWCMSHC_EMMC_DLL_START,
            );
            self.write_reg32(DWCMSHC_EMMC_DLL_RXCLK, DLL_RXCLK_ORI_GATE);
            self.write_reg32(DWCMSHC_EMMC_DLL_TXCLK, 0);
            self.write_reg32(DECMSHC_EMMC_DLL_CMDOUT, 0);

            // Before switching to hs400es mode, the driver
            // will enable enhanced strobe first. PHY needs to
            // configure the parameters of enhanced strobe first.
            let ddr50_strbin_delay_num = 16;
            let extra = DWCMSHC_EMMC_DLL_DLYENA
                | DLL_STRBIN_DELAY_NUM_SEL
                | (ddr50_strbin_delay_num << DLL_STRBIN_DELAY_NUM_OFFSET);
            // info!("extra: {:#b}", extra);
            self.write_reg32(DWCMSHC_EMMC_DLL_STRBIN, extra);
        }

        // Enable card clock
        self.enable_card_clock(0)?;

        info!("Clock {:#x}", self.read_reg16(EMMC_CLOCK_CONTROL));

        Ok(())
    }

    fn sdhci_set_uhs_signaling(&self, timing: u32) {
        let mut ctrl_2 = self.read_reg16(EMMC_HOST_CTRL2);
        ctrl_2 &= !MMC_CTRL_UHS_MASK;

        if (timing != MMC_TIMING_LEGACY)
            && (timing != MMC_TIMING_MMC_HS)
            && (timing != MMC_TIMING_SD_HS)
        {
            ctrl_2 |= MMC_CTRL_VDD_180;
        }

        if (timing == MMC_TIMING_MMC_HS200) || (timing == MMC_TIMING_UHS_SDR104) {
            ctrl_2 |= MMC_CTRL_UHS_SDR104 | MMC_CTRL_DRV_TYPE_A;
        } else if timing == MMC_TIMING_UHS_SDR12 {
            ctrl_2 |= MMC_CTRL_UHS_SDR12;
        } else if timing == MMC_TIMING_UHS_SDR25 {
            ctrl_2 |= MMC_CTRL_UHS_SDR25;
        } else if (timing == MMC_TIMING_UHS_SDR50) || (timing == MMC_TIMING_MMC_HS) {
            ctrl_2 |= MMC_CTRL_UHS_SDR50;
        } else if (timing == MMC_TIMING_UHS_DDR50) || (timing == MMC_TIMING_MMC_DDR52) {
            ctrl_2 |= MMC_CTRL_UHS_DDR50;
        } else if timing == MMC_TIMING_MMC_HS400 || timing == MMC_TIMING_MMC_HS400ES {
            ctrl_2 |= MMC_CTRL_HS400 | MMC_CTRL_DRV_TYPE_A;
        }

        debug!("EMMC Host Control 2: {:#x}", ctrl_2);

        self.write_reg16(EMMC_HOST_CTRL2, ctrl_2);
    }

    fn sdhci_get_version(&self) -> u16 {
        self.read_reg16(EMMC_HOST_CNTRL_VER) & 0xFF
    }
}
