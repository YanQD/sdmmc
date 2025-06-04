use crate::{
    aux::dll_lock_wo_tmout,
    clock::emmc_set_clk,
    commands::MmcCommand,
    constants::*,
    delay_us,
    host::rockship::{SdhciError, SdhciHost},
};
use log::{debug, info};

use super::SdhciResult;

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
    fn rk3568_config() -> Self {
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
    pub fn mmc_set_ios(&mut self) {
        let (card_clock, bus_width, timing) = (self.clock, self.bus_width, self.timing);

        debug!(
            "card_clock: {}, bus_width: {}, timing: {}",
            card_clock, bus_width, timing
        );

        self.dwcmshc_sdhci_emmc_set_clock(card_clock).unwrap();

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

        self.sdhci_set_uhs_signaling();
    }

    pub fn mmc_card_busy(&self) -> bool {
        let present_state = self.read_reg32(EMMC_PRESENT_STATE);
        // Check if DATA[0] line is 0 (low level indicates busy)
        !(present_state & EMMC_DATA_0_LVL != 0)
    }

    // Rockchip EMMC设置时钟函数
    fn rockchip_emmc_set_clock(&mut self, freq: u32) -> SdhciResult {
        // wait for command and data inhibit to be cleared
        let mut timeout = 20;
        while (self.read_reg32(EMMC_PRESENT_STATE) & (EMMC_CMD_INHIBIT | EMMC_DATA_INHIBIT)) != 0 {
            if timeout == 0 {
                debug!("Timeout waiting for cmd & data inhibit");
                return Err(SdhciError::Timeout);
            }
            timeout -= 1;
            delay_us(1000);
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

    fn enable_card_clock(&mut self, mut clk: u16) -> SdhciResult {
        clk |= EMMC_CLOCK_INT_EN;
        clk &= !EMMC_CLOCK_INT_STABLE;
        self.write_reg16(EMMC_CLOCK_CONTROL, clk);

        let mut timeout = 20;
        while (self.read_reg16(EMMC_CLOCK_CONTROL) & EMMC_CLOCK_INT_STABLE) == 0 {
            timeout -= 1;
            delay_us(1000);
            if timeout == 0 {
                info!("Internal clock never stabilised.");
                return Err(SdhciError::Timeout);
            }
        }

        self.write_reg16(EMMC_CLOCK_CONTROL, clk | EMMC_CLOCK_CARD_EN);

        debug!(
            "EMMC Clock Control: {:#x}",
            self.read_reg16(EMMC_CLOCK_CONTROL)
        );

        Ok(())
    }

    pub fn sdhci_set_power(&mut self, power: u32) -> SdhciResult {
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
        delay_us(10000);

        Ok(())
    }

    /// Sets the clock for the DWCMSHC SDHCI EMMC controller.
    fn dwcmshc_sdhci_emmc_set_clock(&mut self, freq: u32) -> SdhciResult {
        let mut timeout = 500;
        let timing = self.timing;
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
            delay_us(1000);
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
                    return Err(SdhciError::Timeout);
                }

                if dll_lock_wo_tmout(self.read_reg32(DWCMSHC_EMMC_DLL_STATUS0)) {
                    break;
                }

                delay_us(1000);
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

    fn sdhci_set_uhs_signaling(&self) {
        let timing = self.timing;

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

    pub fn mmc_card_hs400es(&self) -> bool {
        let timing = self.timing;
        timing == MMC_TIMING_MMC_HS400ES
    }

    /// check if the card is in HS200 mode
    pub fn mmc_card_hs200(&self) -> bool {
        let timing = self.timing;
        timing == MMC_TIMING_MMC_HS200
    }

    /// check if the card supports high-speed modes
    pub fn mmc_card_hs(&self) -> bool {
        let timing = self.timing;
        (timing == MMC_TIMING_MMC_HS) || (timing == MMC_TIMING_SD_HS)
    }

    pub fn mmc_set_bus_speed(&mut self, avail_type: u32) {
        let mut clock = 0;

        if self.mmc_card_hs() {
            clock = if (avail_type & EXT_CSD_CARD_TYPE_52 as u32) != 0 {
                MMC_HIGH_52_MAX_DTR
            } else {
                MMC_HIGH_26_MAX_DTR
            };
        } else if self.mmc_card_hs200() {
            clock = MMC_HS200_MAX_DTR;
        }

        self.mmc_set_clock(clock);
    }

    pub fn mmc_select_card_type(&self, ext_csd: &[u8]) -> u16 {
        let card_type = ext_csd[EXT_CSD_CARD_TYPE as usize] as u16;
        let host_caps = self.host_caps;
        let mut avail_type = 0;

        if (host_caps & MMC_MODE_HS != 0) && (card_type & EXT_CSD_CARD_TYPE_26 != 0) {
            avail_type |= EXT_CSD_CARD_TYPE_26;
        }

        if (host_caps & MMC_MODE_HS != 0) && (card_type & EXT_CSD_CARD_TYPE_52 != 0) {
            avail_type |= EXT_CSD_CARD_TYPE_52;
        }

        if (host_caps & MMC_MODE_DDR_52MHZ != 0)
            && (card_type & EXT_CSD_CARD_TYPE_DDR_1_8V as u16 != 0)
        {
            avail_type |= EXT_CSD_CARD_TYPE_DDR_1_8V as u16;
        }

        if (host_caps & MMC_MODE_HS200 != 0) && (card_type & EXT_CSD_CARD_TYPE_HS200_1_8V != 0) {
            avail_type |= EXT_CSD_CARD_TYPE_HS200_1_8V;
        }

        if (host_caps & MMC_MODE_HS400 != 0)
            && (host_caps & MMC_MODE_8BIT != 0)
            && (card_type & EXT_CSD_CARD_TYPE_HS400_1_8V != 0)
        {
            avail_type |= EXT_CSD_CARD_TYPE_HS200_1_8V | EXT_CSD_CARD_TYPE_HS400_1_8V;
        }

        if (host_caps & MMC_MODE_HS400ES != 0)
            && (host_caps & MMC_MODE_8BIT != 0)
            && (ext_csd[EXT_CSD_STROBE_SUPPORT as usize] != 0)
            && (avail_type & EXT_CSD_CARD_TYPE_HS400_1_8V != 0)
        {
            avail_type |= EXT_CSD_CARD_TYPE_HS200_1_8V
                | EXT_CSD_CARD_TYPE_HS400_1_8V
                | EXT_CSD_CARD_TYPE_HS400ES;
        }

        avail_type
    }

    /// Send a single tuning block read command over the SDHCI interface
    fn mmc_send_tuning(&mut self, opcode: u8) -> SdhciResult {
        // Helper to pack DMA boundary and block size fields
        let make_blksz = |dma: u16, blksz: u16| ((dma & 0x7) << 12) | (blksz & 0x0FFF);

        // Determine current bus width (1/4/8 bits)
        let bus_width = self.bus_width;

        // Choose block size: 128 bytes for HS200 on 8-bit bus, else 64 bytes
        let block_size = if opcode == MMC_SEND_TUNING_BLOCK_HS200 && bus_width == MMC_BUS_WIDTH_8BIT
        {
            128
        } else {
            64
        };

        // Program block size and enable DMA boundary
        self.write_reg16(EMMC_BLOCK_SIZE, make_blksz(7, block_size));
        // Set transfer mode to single-block read
        self.write_reg16(EMMC_XFER_MODE, EMMC_TRNS_READ);

        // Build and send the tuning command
        let cmd = MmcCommand::new(opcode, 0, MMC_RSP_R1);
        self.send_command(&cmd, None).unwrap();

        Ok(())
    }

    /// Perform HS200 tuning sequence (also used for HS400 initial tuning)
    pub fn mmc_hs200_tuning(&mut self) -> SdhciResult {
        let opcode = MMC_SEND_TUNING_BLOCK_HS200;
        let timing = self.timing;

        match timing {
            // HS400 tuning must be issued in HS200 mode; reject direct HS400 timing
            MMC_TIMING_MMC_HS400 => {
                return Err(SdhciError::InvalidValue);
            }
            // HS200 timing: OK to proceed with tuning here
            MMC_TIMING_MMC_HS200 => {
                // HS400 re-tuning is not expected; leave periodic tuning disabled
            }
            // Any other timing mode is invalid for HS200 tuning
            _ => {
                return Err(SdhciError::InvalidValue);
            }
        }

        // Set the EXEC_TUNING bit in Host Control2 to start tuning
        let mut ctrl = self.read_reg16(EMMC_HOST_CTRL2);
        ctrl |= MMC_CTRL_EXEC_TUNING;
        self.write_reg16(EMMC_HOST_CTRL2, ctrl);

        // Invoke the common tuning loop implementation
        self.__emmc_execute_tuning(opcode)
    }

    /// Core tuning loop: send tuning blocks until the controller indicates success or timeout
    fn __emmc_execute_tuning(&mut self, opcode: u8) -> SdhciResult {
        const MAX_TUNING_LOOP: usize = 40;

        for _ in 0..MAX_TUNING_LOOP {
            // Send one tuning block command
            self.mmc_send_tuning(opcode)?;

            // Read back Host Control2 to check tuning status
            let ctrl = self.read_reg16(EMMC_HOST_CTRL2);

            // If the EXEC_TUNING bit has been cleared by hardware...
            if (ctrl & MMC_CTRL_EXEC_TUNING) == 0 {
                // ...and the TUNED_CLK bit is set, tuning succeeded
                if (ctrl & MMC_CTRL_TUNED_CLK) != 0 {
                    return Ok(());
                }
                // EXEC_TUNING cleared but no TUNED_CLK => break and report failure
                break;
            }
        }

        // Exceeded max loops without success: timeout
        Err(SdhciError::Timeout)
    }
}
